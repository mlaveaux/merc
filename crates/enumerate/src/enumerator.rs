use std::collections::VecDeque;
use std::ops::ControlFlow;
use std::rc::Rc;

use ahash::HashMap;
use ahash::HashMapExt;
use merc_data::BasicSort;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataFunctionSymbol;
use merc_data::DataVariable;
use merc_data::SortExpression;
use merc_sabre::RewriteEngine;

use crate::binding::BindingArena;
use crate::binding::BindingChain;
use crate::enumeration_plan::EnumerationPlanId;
use crate::enumeration_plan::EnumerationPlans;
use crate::enumeration_plan::NotEnumerableReason;
use crate::enumeration_plan::SortEnumerability;
use crate::fresh::FreshVariableGenerator;
use crate::one_point::apply_one_point_rule;
use crate::ordering::order_variables_by_constraints;

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
    /// The search hit a limit, or a variable's sort was not enumerable, before
    /// finding a witness or exhausting the space. The caller must not treat
    /// this as [`WitnessOutcome::NoneExists`]: for `∀`, that distinction is
    /// exactly `true` versus "unknown".
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
    /// the variables still to be instantiated
    remaining: Vec<DataVariable>,
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
    /// Scratch buffers, reused across calls rather than reallocated per search.
    solution_buf: Vec<DataExpression>,
    bindings_arena: BindingArena,
    queue: VecDeque<WorkItem>,
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
            solution_buf: Vec::new(),
            bindings_arena: BindingArena::default(),
            queue: VecDeque::new(),
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
        mut consume: F,
    ) -> Outcome<B>
    where
        F: FnMut(&mut R, &Solution<'_>) -> ControlFlow<B>,
    {
        let true_literal = bool_literal(true);
        let false_literal = bool_literal(false);

        self.bindings_arena.clear();
        let body = rewriter.rewrite(body);
        let (remaining, body, initial_bindings) =
            apply_one_point_rule(rewriter, &mut self.bindings_arena, vars.to_vec(), body);
        let remaining = order_variables_by_constraints(remaining, &body);

        self.drive(
            rewriter,
            generator,
            vars,
            remaining,
            body,
            initial_bindings,
            &false_literal,
            false,
            |rewriter, leaf_body, solution| {
                if *leaf_body == true_literal {
                    consume(rewriter, solution)
                } else {
                    ControlFlow::Continue(())
                }
            },
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
        let (identity, absorbing) = match kind {
            QuantifierKind::Exists => (bool_literal(false), bool_literal(true)),
            QuantifierKind::Forall => (bool_literal(true), bool_literal(false)),
        };

        self.bindings_arena.clear();
        let body = rewriter.rewrite(body);
        let (remaining, body, initial_bindings) =
            apply_one_point_rule(rewriter, &mut self.bindings_arena, vars.to_vec(), body);
        let remaining = order_variables_by_constraints(remaining, &body);

        let mut found: Option<Vec<DataExpression>> = None;
        let outcome = self.drive(
            rewriter,
            generator,
            vars,
            remaining,
            body,
            initial_bindings,
            &identity,
            true,
            |_rewriter, leaf_body, solution| {
                if *leaf_body == absorbing {
                    found = Some(solution.values().to_vec());
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        );

        match (outcome, found) {
            (_, Some(values)) => WitnessOutcome::Found(values),
            (Outcome::Exhausted, None) => WitnessOutcome::NoneExists,
            (Outcome::LimitReached, None) | (Outcome::NotEnumerable(..), None) => WitnessOutcome::GaveUp,
            (Outcome::Stopped(()), None) => {
                unreachable!("Stopped only occurs when on_leaf returns Break, which only happens once `found` is set")
            }
        }
    }

    /// Expands `remaining_vars` breadth-first, pruning any branch whose body
    /// has already rewritten to `reject` regardless of its still-free
    /// variables, and reports every leaf (fully ground) branch to `on_leaf`.
    ///
    /// `remaining_vars` is expected to already be in the order the caller
    /// wants variables expanded in — both callers pass it through
    /// [`order_variables_by_constraints`] first (§6.3). Fresh variables
    /// introduced while expanding a constructor are still appended at the
    /// tail of each work item's own remaining list (§4.1/§4.5), so this
    /// ordering only ever affects which of the *original* variables is
    /// expanded first, not the fresh ones a variable's own expansion spawns.
    ///
    /// `all_vars` is the *original* variable list (before the one-point rule
    /// may have removed some of them) — every one of them is resolved into the
    /// [`Solution`] handed to `on_leaf`, since one-point-eliminated variables
    /// are bound in `initial_bindings` just as surely as the ones the search
    /// itself binds.
    #[allow(clippy::too_many_arguments)]
    fn drive<R: RewriteEngine, B>(
        &mut self,
        rewriter: &mut R,
        generator: &mut FreshVariableGenerator,
        all_vars: &[DataVariable],
        remaining_vars: Vec<DataVariable>,
        body: DataExpression,
        initial_bindings: BindingChain,
        reject: &DataExpression,
        bounded: bool,
        mut on_leaf: impl FnMut(&mut R, &DataExpression, &Solution<'_>) -> ControlFlow<B>,
    ) -> Outcome<B> {
        let limits = self.limits;
        let mut processed: usize = 0;
        let mut truncated = false;

        self.queue.clear();
        self.queue.push_back(WorkItem {
            remaining: remaining_vars,
            body,
            bindings: initial_bindings,
            depth: 0,
        });

        while let Some(item) = self.queue.pop_front() {
            if item.body == *reject {
                continue;
            }

            if item.remaining.is_empty() {
                self.bindings_arena
                    .resolve_all(rewriter, item.bindings, all_vars, &mut self.solution_buf);
                let solution = Solution {
                    values: &self.solution_buf,
                };

                match on_leaf(rewriter, &item.body, &solution) {
                    ControlFlow::Break(b) => return Outcome::Stopped(b),
                    ControlFlow::Continue(()) => continue,
                }
            }

            let variable = item.remaining[0].clone();
            let rest = &item.remaining[1..];

            let Some(sort_id) = self.plans.get(&variable.sort()) else {
                return Outcome::NotEnumerable(variable, NotEnumerableReason::UnknownSort);
            };

            match self.plans.plan(sort_id).enumerability() {
                SortEnumerability::NotEnumerable(reason) => {
                    return Outcome::NotEnumerable(variable, reason);
                }
                SortEnumerability::Finite => {
                    // A free function over the single field, so the elements
                    // stay borrowed while the other fields are mutated below.
                    let elements = cached_finite_elements(&mut self.finite_elements, rewriter, &self.plans, sort_id);
                    for element in elements {
                        let new_bindings = self
                            .bindings_arena
                            .extend(item.bindings, variable.clone(), element.clone());
                        let new_body = rewrite_bound(rewriter, &self.bindings_arena, &item.body, new_bindings);
                        self.queue.push_back(WorkItem {
                            remaining: rest.to_vec(),
                            body: new_body,
                            bindings: new_bindings,
                            depth: item.depth,
                        });
                    }
                }
                SortEnumerability::InfiniteEnumerable => {
                    if bounded && item.depth >= limits.max_depth {
                        truncated = true;
                        continue;
                    }

                    // Iterate the constructors through an owned handle, so the
                    // other fields of `self` stay mutable in the loop body.
                    let plans = self.plans.clone();
                    for constructor in plans.plan(sort_id).constructors() {
                        if bounded {
                            if processed >= limits.max_items {
                                return Outcome::LimitReached;
                            }
                            processed += 1;
                        }

                        let mut fresh_vars = Vec::with_capacity(constructor.arity());
                        let mut arguments = Vec::with_capacity(constructor.arity());
                        for &argument_sort in constructor.arguments() {
                            let fresh = generator.generate("v", plans.plan(argument_sort).sort().copy());
                            arguments.push(DataExpression::from(fresh.clone()));
                            fresh_vars.push(fresh);
                        }

                        let value: DataExpression = if arguments.is_empty() {
                            constructor.symbol().clone().into()
                        } else {
                            DataApplication::with_args(constructor.symbol(), &arguments).into()
                        };
                        // `rewrite_with` splices substitution images in without rewriting them,
                        // so they must already be normal forms — a constructor application is
                        // not automatically one (`@c0`/`@succ_nat(_)` under a machine-word
                        // `Nat` encoding still rewrite into digit form).
                        let value = rewriter.rewrite(&value);

                        let new_bindings = self.bindings_arena.extend(item.bindings, variable.clone(), value);
                        let new_body = rewrite_bound(rewriter, &self.bindings_arena, &item.body, new_bindings);

                        let mut new_remaining = rest.to_vec();
                        new_remaining.extend(fresh_vars);

                        self.queue.push_back(WorkItem {
                            remaining: new_remaining,
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
    bindings_arena: &BindingArena,
    body: &DataExpression,
    bindings: BindingChain,
) -> DataExpression {
    rewriter.rewrite_with(body, &bindings_arena.substitution(bindings))
}

/// Returns the canonical `Bool` literal `true`/`false` (mCRL2's
/// `sort_bool::true_`/`false_`).
pub(crate) fn bool_literal(value: bool) -> DataExpression {
    let name = if value { "true" } else { "false" };
    let bool_sort = SortExpression::from(BasicSort::new("Bool"));
    DataFunctionSymbol::with_sort(name, bool_sort.copy()).into()
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
