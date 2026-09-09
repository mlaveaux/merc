use std::borrow::Cow;
use std::collections::VecDeque;
use std::ops::ControlFlow;
use std::rc::Rc;

use ahash::HashMap;
use ahash::HashMapExt;
use merc_aterm::Protected;
use merc_aterm::Term;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_data::DataVariableRef;
use merc_data::bool_literal;
use merc_sabre::RewriteEngine;

use crate::binding::BindingArena;
use crate::binding::BindingChain;
use crate::binding::BindingGuard;
use crate::enumeration_plan::EnumerationPlanId;
use crate::enumeration_plan::EnumerationPlans;
use crate::enumeration_plan::NotEnumerableReason;
use crate::enumeration_plan::SortEnumerability;
use crate::fresh::FreshVariableGenerator;
use crate::one_point::OnePointPolarity;
use crate::one_point::apply_one_point_rule;
use crate::ordering::VariableRanks;
use crate::ordering::apply_variable_ranks;
use crate::ordering::compute_variable_ranks;
use crate::remaining::RemainingArena;
use crate::remaining::RemainingList;

/// Configures how far [`Enumerator::find_witness`] searches before giving up.
///
/// Must stay fixed for the lifetime of a cache built over this enumerator's
/// results: a summand or quantifier cache keyed on read positions assumes two
/// calls with the same inputs enumerate the same solutions, a search bound that
/// changes between calls violates this assumption.
#[derive(Clone, Copy, Debug)]
pub struct EnumerationLimits {
    /// Maximum number of work items processed before giving up. mCRL2's
    /// `qlimit`, default 1000.
    pub max_items: usize,
    /// Maximum constructor-nesting depth per variable before giving up on
    /// that branch. No mCRL2 equivalent; caps how unpredictable a truncation
    /// is, which matters for cache correctness (§6.4).
    pub max_depth: u32,
}

impl Default for EnumerationLimits {
    fn default() -> Self {
        EnumerationLimits {
            max_items: 1000,
            max_depth: 64,
        }
    }
}

/// Why an [`Enumerator::enumerate`] call stopped.
///
/// `B` is the consumer callback's [`ControlFlow::Break`] payload (`()` if it
/// never breaks with one), so a caller can thread e.g. a `Result` straight
/// out of [`Outcome::Stopped`] instead of latching it in a captured variable.
#[derive(Debug)]
pub enum Outcome<B = ()> {
    /// Every solution was reported; the search space is exhausted.
    Exhausted,
    /// The consumer callback returned [`ControlFlow::Break`], carrying its payload.
    Stopped(B),
    /// [`EnumerationLimits::max_items`] or `max_depth` was hit somewhere in
    /// the search; results are incomplete.
    LimitReached,
    /// A fully ground branch's body rewrote to neither `true` nor `false`
    /// (carried here), so it decided nothing.
    Undecided(DataExpression),
    /// `variable`'s sort cannot be enumerated at all (`reason`), so the whole
    /// search is abandoned.
    NotEnumerable(DataVariable, NotEnumerableReason),
}

/// The result of [`Enumerator::find_witness`].
#[derive(Debug)]
pub enum WitnessOutcome {
    /// A witness was found; `values` are the bound variables' values, in the
    /// same order as the `vars` argument.
    Found(Vec<DataExpression>),
    /// The search was exhausted and no witness exists.
    NoneExists,
    /// The search hit a limit, a variable's sort was not enumerable, or some
    /// ground body rewrote to neither `true` nor `false`, before finding a
    /// witness or exhausting the space. The caller must not treat this as
    /// [`WitnessOutcome::NoneExists`]: for `∀`, that distinction is exactly
    /// `true` versus "unknown".
    GaveUp,
}

/// Which quantifier [`Enumerator::find_witness`] is deciding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuantifierKind {
    /// Find a witness making the body `true`.
    Exists,
    /// Find a counterexample making the body `false`; exhaustion means the
    /// quantifier holds.
    Forall,
}

/// A ground solution reported by [`Enumerator::enumerate`]: the bound
/// variables' values, in the same order as the `vars` argument.
///
/// Borrows the enumerator's scratch buffer, so it is only valid for the
/// duration of the consumer callback it is handed to.
pub struct Solution<'s> {
    values: &'s [DataExpression],
}

impl Solution<'_> {
    /// Returns the bound variables' values, in `vars` order.
    pub fn values(&self) -> &[DataExpression] {
        self.values
    }
}

/// One unfinished branch of the search.
struct WorkItem {
    /// The variables still to be instantiated, in enumeration order, as a
    /// [`RemainingList`] of indices into [`Enumerator::var_pool`].
    remaining: RemainingList,
    /// the goal body already rewritten under the current bindings
    body: DataExpression,
    /// a `Copy` handle into the enumerator's `BindingArena`
    bindings: BindingChain,
    /// the search depth at which this work item was created
    depth: u32,
}

/// Enumerates ground constructor instances of a data sort, closing a
/// quantifier body (via [`Enumerator::find_witness`]) or a `sum`-variable
/// guard (via [`Enumerator::enumerate`]).
pub struct Enumerator {
    plans: Rc<EnumerationPlans>,
    limits: EnumerationLimits,
    /// Finite sorts' materialised element list. Tied to the rewriter that
    /// produced them, so it must not outlive a change of `RewriteEngine`.
    finite_elements: HashMap<EnumerationPlanId, Vec<DataExpression>>,
    /// The `Bool` literals
    true_literal: DataExpression,
    false_literal: DataExpression,
    /// Backing store `WorkItem.remaining`'s indices point into for the
    /// current `drive` call.
    var_pool: Protected<Vec<DataVariableRef<'static>>>,
    /// Caches [`compute_variable_ranks`]'s result.
    ordering_cache: HashMap<(usize, usize), VariableRanks>,
    /// Scratch buffers, reused across calls rather than reallocated per search.
    solution_buf: Vec<DataExpression>,
    bindings_arena: BindingArena,
    remaining_arena: RemainingArena,
    queue: VecDeque<WorkItem>,
    /// Scratch buffers for one constructor expansion's fresh arguments and
    /// their `var_pool` indices, reused across every constructor of every
    /// [`SortEnumerability::InfiniteEnumerable`] variable in the search
    /// rather than reallocated per constructor.
    constructor_args_buf: Vec<DataExpression>,
    fresh_indices_buf: Vec<u32>,
}

impl Enumerator {
    /// Builds an enumerator with the default [`EnumerationLimits`].
    ///
    /// The `Rc` keeps `Enumerator` free of a lifetime parameter, so it can be
    /// stored as a plain field. The rewriter and [`FreshVariableGenerator`]
    /// are borrowed per call instead, since they may differ between searches.
    pub fn new(plans: Rc<EnumerationPlans>) -> Self {
        Enumerator {
            plans,
            limits: EnumerationLimits::default(),
            finite_elements: HashMap::new(),
            true_literal: bool_literal(true),
            false_literal: bool_literal(false),
            var_pool: Protected::new(Vec::new()),
            ordering_cache: HashMap::new(),
            solution_buf: Vec::new(),
            bindings_arena: BindingArena::default(),
            remaining_arena: RemainingArena::default(),
            queue: VecDeque::new(),
            constructor_args_buf: Vec::new(),
            fresh_indices_buf: Vec::new(),
        }
    }

    /// Sets the [`EnumerationLimits`] used by [`Enumerator::find_witness`].
    ///
    /// Must be called before the first search, and not changed afterwards.
    pub fn with_limits(mut self, limits: EnumerationLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Enumerates every ground substitution `σ` of `vars` for which
    /// `rewrite(body, σ)` is the `Bool` literal `true`, reporting each one to
    /// `consume`.
    ///
    /// Runs to exhaustion or until `consume` returns [`ControlFlow::Break`];
    /// **never bounded by [`EnumerationLimits`]** — see that type's doc
    /// comment for why. Intended for `sum`-variable successor generation,
    /// where every solution is wanted and a truncated result would be an
    /// unsound state space.
    pub fn enumerate<R: RewriteEngine, F, B>(
        &mut self,
        rewriter: &mut R,
        generator: &mut FreshVariableGenerator,
        vars: &[DataVariable],
        body: &DataExpression,
        consume: F,
    ) -> Outcome<B>
    where
        F: FnMut(&mut R, &Solution<'_>) -> ControlFlow<B>,
    {
        let body = rewriter.rewrite(body);
        self.enumerate_normalized(rewriter, generator, vars, body, consume)
    }

    /// Same as [`Self::enumerate`], but for a `body` the caller has already
    /// rewritten to normal form.
    pub fn enumerate_normalized<R: RewriteEngine, F, B>(
        &mut self,
        rewriter: &mut R,
        generator: &mut FreshVariableGenerator,
        vars: &[DataVariable],
        body: DataExpression,
        consume: F,
    ) -> Outcome<B>
    where
        F: FnMut(&mut R, &Solution<'_>) -> ControlFlow<B>,
    {
        self.bindings_arena.clear();
        let remaining = self.order_variables(vars, vars, &body);

        self.drive(
            rewriter,
            generator,
            vars,
            &remaining,
            body,
            BindingChain::default(),
            false, // reject on `false`, accept on `true`
            false, // unbounded
            consume,
        )
    }

    /// Searches for a ground substitution `σ` of `vars` that decides `kind`:
    /// for [`QuantifierKind::Exists`], one making `rewrite(body, σ)` `true`;
    /// for [`QuantifierKind::Forall`], a counterexample making it `false`.
    ///
    /// Bounded by [`EnumerationLimits`], returns [`WitnessOutcome::GaveUp`]
    /// rather than guessing when the bound is hit or a variable's sort cannot
    /// be enumerated.
    pub fn find_witness<R: RewriteEngine>(
        &mut self,
        rewriter: &mut R,
        generator: &mut FreshVariableGenerator,
        vars: &[DataVariable],
        body: &DataExpression,
        kind: QuantifierKind,
    ) -> WitnessOutcome {
        // `reject_is_true` says which literal settles the quantifier for a
        // branch without deciding it; the other produces the witness.
        let (reject_is_true, polarity) = match kind {
            QuantifierKind::Exists => (false, OnePointPolarity::Existential),
            QuantifierKind::Forall => (true, OnePointPolarity::Universal),
        };

        self.bindings_arena.clear();
        let body = rewriter.rewrite(body);
        // Scoped so this guard is dropped before `drive` below takes its own:
        // `Protected::write` only ever hands out one guard at a time.
        let (remaining, body, initial_bindings) = {
            let mut guard = self.bindings_arena.chain.write();
            apply_one_point_rule(rewriter, &mut guard, vars.to_vec(), body, polarity)
        };
        let remaining = self.order_variables(vars, &remaining, &body);

        let mut found: Option<Vec<DataExpression>> = None;
        let outcome = self.drive(
            rewriter,
            generator,
            vars,
            &remaining,
            body,
            initial_bindings,
            reject_is_true,
            true,
            |_rewriter, solution| {
                found = Some(solution.values().to_vec());
                ControlFlow::Break(())
            },
        );

        match (outcome, found) {
            (_, Some(values)) => WitnessOutcome::Found(values),
            (Outcome::Exhausted, None) => WitnessOutcome::NoneExists,
            (Outcome::LimitReached, None) | (Outcome::NotEnumerable(..), None) | (Outcome::Undecided(_), None) => {
                WitnessOutcome::GaveUp
            }
            (Outcome::Stopped(()), None) => {
                unreachable!("Stopped only occurs when on_leaf returns Break, which only happens once `found` is set")
            }
        }
    }

    /// Orders `remaining` for enumeration based on precomputed ranks.
    ///
    /// The overwhelming common case — a single remaining variable, or a body
    /// this pair has never been seen with before mentioning more than one of
    /// them — needs no reordering at all, so this borrows `remaining` rather
    /// than requiring an owned `Vec` from every caller: a search with only
    /// one variable left to enumerate (true for most LPS summands) pays no
    /// allocation here whatsoever.
    fn order_variables<'v>(
        &mut self,
        vars: &[DataVariable],
        remaining: &'v [DataVariable],
        body: &DataExpression,
    ) -> Cow<'v, [DataVariable]> {
        if remaining.len() <= 1 {
            return Cow::Borrowed(remaining);
        }

        let key = (vars.as_ptr() as usize, body.index());
        let ranks = self
            .ordering_cache
            .entry(key)
            .or_insert_with(|| compute_variable_ranks(remaining, body));
        Cow::Owned(apply_variable_ranks(remaining.to_vec(), ranks))
    }

    /// Expands `remaining_vars` breadth-first, pruning any branch whose body
    /// has already rewritten to `reject` regardless of its still-free
    /// variables, and reports every leaf (fully ground) branch whose body is
    /// `accept` to `on_leaf`.
    ///
    /// A leaf that is neither `reject` nor `accept` decided nothing, so the
    /// search reports [`Outcome::Undecided`] rather than
    /// [`Outcome::Exhausted`]: the two are indistinguishable from inside the
    /// search, and only the caller knows whether an under-approximated result
    /// set is acceptable.
    ///
    /// `remaining_vars` is expected to already be in the order the caller wants
    /// variables expanded in — both callers pass it through
    /// [`Self::order_variables`] first (§6.3). Fresh variables introduced while
    /// expanding a constructor are still appended at the tail of each work
    /// item's own remaining list (§4.1/§4.5), so this ordering only ever
    /// affects which of the *original* variables is expanded first, not the
    /// fresh ones a variable's own expansion spawns.
    ///
    /// `all_vars` is the *original* variable list (before the one-point rule
    /// may have removed some of them) — every one of them is resolved into the
    /// [`Solution`] handed to `on_leaf`, since one-point-eliminated variables
    /// are bound in `initial_bindings` just as surely as the ones the search
    /// itself binds.
    ///
    /// `reject_is_true` selects which of the enumerator's cached `Bool`
    /// literals is the rejecting one.
    #[allow(clippy::too_many_arguments)]
    fn drive<R: RewriteEngine, B>(
        &mut self,
        rewriter: &mut R,
        generator: &mut FreshVariableGenerator,
        all_vars: &[DataVariable],
        remaining_vars: &[DataVariable],
        body: DataExpression,
        initial_bindings: BindingChain,
        reject_is_true: bool,
        bounded: bool,
        mut on_leaf: impl FnMut(&mut R, &Solution<'_>) -> ControlFlow<B>,
    ) -> Outcome<B> {
        let limits = self.limits;
        let mut processed: usize = 0;
        let mut truncated = false;
        let mut undecided: Option<DataExpression> = None;
        let (reject, accept) = if reject_is_true {
            (&self.true_literal, &self.false_literal)
        } else {
            (&self.false_literal, &self.true_literal)
        };

        self.queue.clear();
        self.remaining_arena.clear();
        // Both guards are held for the whole call rather than re-acquired per
        // access.
        let mut var_pool = self.var_pool.write();
        var_pool.clear();
        let mut bindings = self.bindings_arena.chain.write();
        let mut initial_indices = Vec::with_capacity(remaining_vars.len());
        for variable in remaining_vars {
            // SAFETY: the resulting ref is pushed into `var_pool` immediately below.
            let var_ref = unsafe { var_pool.protect(variable) };
            initial_indices.push(u32::try_from(var_pool.len()).expect("more variables than fit in a u32"));
            var_pool.push(var_ref.into());
        }

        self.queue.push_back(WorkItem {
            remaining: RemainingList::new(&mut self.remaining_arena, initial_indices),
            body,
            bindings: initial_bindings,
            depth: 0,
        });

        while let Some(item) = self.queue.pop_front() {
            if item.body == *reject {
                continue;
            }

            // Counted here rather than per constructor expansion, so the bound
            // covers every branch the search takes, not only the ones over an
            // infinite sort.
            if bounded {
                if processed >= limits.max_items {
                    return Outcome::LimitReached;
                }
                processed += 1;
            }

            if item.remaining.is_empty() {
                if item.body != *accept {
                    undecided.get_or_insert_with(|| item.body.clone());
                    continue;
                }

                BindingArena::resolve_all_into(
                    &mut self.bindings_arena.memo,
                    &mut self.bindings_arena.scratch,
                    &bindings,
                    rewriter,
                    item.bindings,
                    all_vars,
                    &mut self.solution_buf,
                );
                let solution = Solution {
                    values: &self.solution_buf,
                };

                match on_leaf(rewriter, &solution) {
                    ControlFlow::Break(b) => return Outcome::Stopped(b),
                    ControlFlow::Continue(()) => continue,
                }
            }

            // Only a constructor expansion nests deeper.
            if bounded && item.depth >= limits.max_depth {
                truncated = true;
                continue;
            }

            // Not protected: `variable_ref` borrows straight from `var_pool`.
            let (variable_index, rest) = item
                .remaining
                .pop_front(&mut self.remaining_arena)
                .expect("checked non-empty above");
            let variable_ref = &var_pool[variable_index as usize];

            let Some(sort_id) = self.plans.get(&variable_ref.sort()) else {
                return Outcome::NotEnumerable(variable_ref.protect(), NotEnumerableReason::UnknownSort);
            };

            match self.plans.plan(sort_id).enumerability() {
                SortEnumerability::NotEnumerable(reason) => {
                    return Outcome::NotEnumerable(variable_ref.protect(), reason);
                }
                SortEnumerability::Finite => {
                    // A free function over the single field, so the elements
                    // stay borrowed while the other fields are mutated below.
                    // No mutation of `var_pool` happens in this arm, so
                    // `variable_ref` stays valid for the whole loop as-is.
                    let elements = cached_finite_elements(&mut self.finite_elements, rewriter, &self.plans, sort_id);
                    for element in elements {
                        let element_ref = element.copy();
                        let new_bindings =
                            BindingArena::extend(&mut bindings, item.bindings, variable_ref, &element_ref);
                        let new_body = rewrite_bound(rewriter, &bindings, &item.body, new_bindings);
                        self.queue.push_back(WorkItem {
                            remaining: rest,
                            body: new_body,
                            bindings: new_bindings,
                            depth: item.depth,
                        });
                    }
                }
                SortEnumerability::InfiniteEnumerable => {
                    // Iterate the constructors through an owned handle, so the
                    // other fields of `self` stay mutable in the loop body.
                    let plans = self.plans.clone();
                    for constructor in plans.plan(sort_id).constructors() {
                        self.fresh_indices_buf.clear();
                        self.constructor_args_buf.clear();
                        for &argument_sort in constructor.arguments() {
                            let fresh = generator.generate("v", plans.plan(argument_sort).sort().copy());
                            self.constructor_args_buf.push(DataExpression::from(fresh.clone()));
                            // SAFETY: the resulting ref is pushed into `var_pool` immediately below.
                            let fresh_ref = unsafe { var_pool.protect(&fresh) };
                            self.fresh_indices_buf
                                .push(u32::try_from(var_pool.len()).expect("more variables than fit in a u32"));
                            var_pool.push(fresh_ref.into());
                        }

                        let value: DataExpression = if self.constructor_args_buf.is_empty() {
                            constructor.symbol().clone().into()
                        } else {
                            DataApplication::with_args(constructor.symbol(), &self.constructor_args_buf).into()
                        };
                        // `rewrite_with` splices substitution images in without rewriting them,
                        // so they must already be normal forms — a constructor application is
                        // not automatically one (`@c0`/`@succ_nat(_)` under a machine-word
                        // `Nat` encoding still rewrite into digit form).
                        //
                        // Only `value_ref` (copied straight into `bindings` below) is needed, so
                        // `rewrite_ref` skips protecting the normal form independently: the
                        // engines that have no cheaper option (see `RewriteEngine::rewrite_ref`)
                        // still allocate exactly as before, but `InnermostRewriter` — the one
                        // every production caller actually uses — does not.
                        let rewritten = rewriter.rewrite_ref(&value);
                        let value_ref = rewritten.as_ref();

                        // Re-borrowed from `var_pool` here, rather than reusing
                        // `variable_ref` from before the match.
                        let variable_ref = &var_pool[variable_index as usize];
                        let new_bindings =
                            BindingArena::extend(&mut bindings, item.bindings, &variable_ref.copy(), &value_ref);
                        let new_body = rewrite_bound(rewriter, &bindings, &item.body, new_bindings);

                        self.queue.push_back(WorkItem {
                            remaining: rest.with_appended(&mut self.remaining_arena, self.fresh_indices_buf.drain(..)),
                            body: new_body,
                            bindings: new_bindings,
                            depth: item.depth + 1,
                        });
                    }
                }
            }
        }

        if truncated {
            Outcome::LimitReached
        } else if let Some(body) = undecided {
            Outcome::Undecided(body)
        } else {
            Outcome::Exhausted
        }
    }
}

/// Returns `sort_id`'s materialised element list, computing and caching it on
/// first use.
fn cached_finite_elements<'e, R: RewriteEngine>(
    finite_elements: &'e mut HashMap<EnumerationPlanId, Vec<DataExpression>>,
    rewriter: &mut R,
    plans: &EnumerationPlans,
    sort_id: EnumerationPlanId,
) -> &'e [DataExpression] {
    if !finite_elements.contains_key(&sort_id) {
        let elements = materialize_finite_elements(finite_elements, rewriter, plans, sort_id);
        finite_elements.insert(sort_id, elements);
    }
    finite_elements.get(&sort_id).expect("just inserted above")
}

fn materialize_finite_elements<R: RewriteEngine>(
    finite_elements: &mut HashMap<EnumerationPlanId, Vec<DataExpression>>,
    rewriter: &mut R,
    plans: &EnumerationPlans,
    sort_id: EnumerationPlanId,
) -> Vec<DataExpression> {
    let mut result = Vec::new();

    for constructor in plans.plan(sort_id).constructors() {
        if constructor.arity() == 0 {
            result.push(constructor.symbol().clone().into());
            continue;
        }

        let mut per_argument: Vec<Vec<DataExpression>> = Vec::with_capacity(constructor.arity());
        for &argument_sort in constructor.arguments() {
            per_argument.push(cached_finite_elements(finite_elements, rewriter, plans, argument_sort).to_vec());
        }

        for combination in cartesian_product(&per_argument) {
            let value: DataExpression = DataApplication::with_args(constructor.symbol(), &combination).into();
            result.push(rewriter.rewrite(&value));
        }
    }

    result
}

/// Rewrites `body` under `bindings`.
///
/// `bindings` is the branch's whole chain, but `body` only still mentions the
/// variable just bound — every earlier one was substituted away in a prior
/// step — so this is equivalent to a singleton substitution.
fn rewrite_bound<R: RewriteEngine>(
    rewriter: &mut R,
    guard: &BindingGuard<'_>,
    body: &DataExpression,
    bindings: BindingChain,
) -> DataExpression {
    rewriter.rewrite_with(body, &BindingArena::substitution(guard, bindings))
}

/// Returns the cartesian product of `lists`: one combination per element of
/// the result, each holding one element from every list in order. An empty
/// `lists` yields a single empty combination — the identity of the product,
/// which is what an arity-0 constructor needs.
pub(crate) fn cartesian_product(lists: &[Vec<DataExpression>]) -> Vec<Vec<DataExpression>> {
    let mut result: Vec<Vec<DataExpression>> = vec![Vec::new()];
    for list in lists {
        let mut next = Vec::with_capacity(result.len() * list.len());
        for prefix in &result {
            for element in list {
                let mut combination = prefix.clone();
                combination.push(element.clone());
                next.push(combination);
            }
        }
        result = next;
    }
    result
}
