use std::collections::VecDeque;
use std::ops::ControlFlow;

use ahash::HashMap;
use ahash::HashMapExt;
use merc_data::BasicSort;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataFunctionSymbol;
use merc_data::DataVariable;
use merc_data::SortExpression;
use merc_sabre::RewriteEngine;

use crate::binding::BindingChain;
use crate::fresh::FreshVariableGenerator;
use crate::one_point::apply_one_point_rule;
use crate::sort_plan::NotEnumerableReason;
use crate::sort_plan::SortEnumerability;
use crate::sort_plan::SortPlanId;
use crate::sort_plan::SortPlans;

/// Configures how far [`Enumerator::find_witness`] searches before giving up.
///
/// **Only [`Enumerator::find_witness`] (quantifier witness search) honours
/// these.** [`Enumerator::enumerate`] (`sum`-variable exploration) ignores
/// them entirely and always runs to exhaustion: truncating a `sum` silently
/// drops transitions and produces a wrong LTS, so there is no safe bound to
/// apply there — see `docs/enumeration-crate-plan.md` §4.6/§7.2. A `sum` over
/// a genuinely unconstrained infinite sort is therefore a real non-termination
/// risk today; bounding *that* case safely needs the caller-facing
/// `--truncate-sum`/hard-error machinery §4.6 describes, which is `merc_lps_data`
/// (Phase 3) wiring, not something the enumerator core can decide on its own.
///
/// Must stay fixed for the lifetime of a cache built over this enumerator's
/// results: a summand or quantifier cache keyed on read positions assumes two
/// calls with the same inputs enumerate the same solutions, which a search
/// bound that changes between calls (e.g. "search deeper on a second visit")
/// would silently violate — see §6.4.
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
#[derive(Debug)]
pub enum Outcome {
    /// Every solution was reported; the search space is exhausted.
    Exhausted,
    /// The consumer callback returned [`ControlFlow::Break`].
    Stopped,
    /// [`EnumerationLimits::max_items`] or `max_depth` was hit somewhere in
    /// the search; results are incomplete.
    ///
    /// [`Enumerator::enumerate`] never returns this: it ignores the limits
    /// entirely (see [`EnumerationLimits`]'s doc comment).
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
///
/// Decoupled from `merc_data`'s binder representation on purpose: the
/// enumerator works over plain `(variables, body)` pairs, so it needs no
/// binder accessors and is usable ahead of the Sabre-side quantifier wiring
/// (`docs/enumeration-crate-plan.md` §5, blocked on capture-avoiding
/// substitution — see §5.3).
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
/// Borrows the enumerator's reused scratch buffer rather than owning a fresh
/// `Vec` per solution, so consuming a solution costs no aterm protection-set
/// traffic beyond the values themselves — see
/// `docs/enumeration-crate-plan.md` §4.4. Clone the values out
/// ([`Solution::values`] returns a slice) if a solution must outlive the
/// callback invocation that received it.
pub struct Solution<'s> {
    values: &'s [DataExpression],
}

impl Solution<'_> {
    /// Returns the bound variables' values, in `vars` order.
    pub fn values(&self) -> &[DataExpression] {
        self.values
    }
}

/// One unfinished branch of the search: the variables still to be
/// instantiated, and the goal body already rewritten under the bindings
/// chosen so far (with those still-`remaining` variables held as free normal
/// forms). See `docs/enumeration-crate-plan.md` §4.1.
struct WorkItem {
    remaining: Vec<DataVariable>,
    body: DataExpression,
    bindings: BindingChain,
    depth: u32,
}

/// Enumerates ground constructor instances of a data sort, closing a
/// quantifier body (via [`Enumerator::find_witness`]) or a `sum`-variable
/// guard (via [`Enumerator::enumerate`]).
///
/// See `docs/enumeration-crate-plan.md` for the full design. `R` is generic
/// over the rewrite engine so the naive/innermost engines can stand in for
/// `SabreRewriter` in tests.
pub struct Enumerator<'a, R: RewriteEngine> {
    rewriter: &'a mut R,
    plans: &'a SortPlans,
    limits: EnumerationLimits,
    /// See [`FreshVariableGenerator`]: seeded once by the caller from
    /// whatever namespace this enumerator must not collide with, and reused
    /// for this `Enumerator`'s whole lifetime.
    generator: FreshVariableGenerator,
    /// Finite sorts' materialised element lists, cached **per enumerator
    /// instance** — populated lazily on first use and reused for the rest of
    /// this `Enumerator`'s lifetime, across every `enumerate`/`find_witness`
    /// call it serves. See `docs/enumeration-crate-plan.md` §6.2; unlike
    /// [`crate::SortPlans`] itself (immutable and shared by `&` across
    /// threads/enumerators), this cache is owned by one `Enumerator` because
    /// the elements it stores are already-rewritten `DataExpression`s tied to
    /// this enumerator's `R` — sharing it across enumerators built over
    /// different rewriters (or different `RewriteSpecification`s) would be
    /// unsound.
    finite_elements: HashMap<SortPlanId, Vec<DataExpression>>,
    /// Reused across leaves to report a [`Solution`] without allocating a
    /// fresh `Vec` per solution (§4.4).
    solution_buf: Vec<DataExpression>,
}

impl<'a, R: RewriteEngine> Enumerator<'a, R> {
    /// Builds an enumerator with the default [`EnumerationLimits`].
    ///
    /// `generator` must be seeded with every name this enumerator's fresh
    /// variables must not collide with — see [`FreshVariableGenerator`]'s doc
    /// comment for why that is the caller's responsibility, not this
    /// constructor's.
    pub fn new(rewriter: &'a mut R, plans: &'a SortPlans, generator: FreshVariableGenerator) -> Self {
        Enumerator {
            rewriter,
            plans,
            limits: EnumerationLimits::default(),
            generator,
            finite_elements: HashMap::new(),
            solution_buf: Vec::new(),
        }
    }

    /// Sets the [`EnumerationLimits`] used by [`Enumerator::find_witness`].
    ///
    /// Must be called before the first search, and not changed afterwards:
    /// see [`EnumerationLimits`]'s doc comment on why a bound that changes
    /// between calls breaks caching built over this enumerator's results.
    pub fn with_limits(mut self, limits: EnumerationLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Enumerates every ground substitution `σ` of `vars` for which
    /// `rewrite(bodyσ)` is the `Bool` literal `true`, reporting each one to
    /// `consume`.
    ///
    /// Runs to exhaustion or until `consume` returns [`ControlFlow::Break`];
    /// **never bounded by [`EnumerationLimits`]** — see that type's doc
    /// comment for why. Intended for `sum`-variable successor generation,
    /// where every solution is wanted and a truncated result would be an
    /// unsound state space.
    pub fn enumerate<F>(&mut self, vars: &[DataVariable], body: &DataExpression, mut consume: F) -> Outcome
    where
        F: FnMut(&Solution<'_>) -> ControlFlow<()>,
    {
        let true_literal = bool_literal(true);
        let false_literal = bool_literal(false);

        let body = self.rewriter.rewrite(body);
        let (remaining, body, initial_bindings) = apply_one_point_rule(self.rewriter, vars.to_vec(), body);

        self.drive(
            vars,
            remaining,
            body,
            initial_bindings,
            &false_literal,
            false,
            |leaf_body, solution| {
                if *leaf_body == true_literal {
                    consume(solution)
                } else {
                    ControlFlow::Continue(())
                }
            },
        )
    }

    /// Searches for a ground substitution `σ` of `vars` that decides `kind`:
    /// for [`QuantifierKind::Exists`], one making `rewrite(bodyσ)` `true`; for
    /// [`QuantifierKind::Forall`], a counterexample making it `false`.
    ///
    /// Bounded by [`EnumerationLimits`] (set via
    /// [`Enumerator::with_limits`]); returns [`WitnessOutcome::GaveUp`] rather
    /// than guessing when the bound is hit or a variable's sort cannot be
    /// enumerated.
    pub fn find_witness(
        &mut self,
        vars: &[DataVariable],
        body: &DataExpression,
        kind: QuantifierKind,
    ) -> WitnessOutcome {
        let (identity, absorbing) = match kind {
            QuantifierKind::Exists => (bool_literal(false), bool_literal(true)),
            QuantifierKind::Forall => (bool_literal(true), bool_literal(false)),
        };

        let body = self.rewriter.rewrite(body);
        let (remaining, body, initial_bindings) = apply_one_point_rule(self.rewriter, vars.to_vec(), body);

        let mut found: Option<Vec<DataExpression>> = None;
        let outcome = self.drive(
            vars,
            remaining,
            body,
            initial_bindings,
            &identity,
            true,
            |leaf_body, solution| {
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
            (Outcome::Stopped, None) => {
                unreachable!("Stopped only occurs when on_leaf returns Break, which only happens once `found` is set")
            }
        }
    }

    /// The shared search core behind both [`Enumerator::enumerate`] and
    /// [`Enumerator::find_witness`]: expands `remaining_vars` breadth-first
    /// (§4.5, for fairness across co-enumerated infinite sorts), pruning any
    /// branch whose body has already rewritten to `reject` regardless of its
    /// still-free variables (sound: see the module-level reasoning in
    /// `docs/enumeration-crate-plan.md` §5.1 step 6), and reports every leaf
    /// (fully ground) branch to `on_leaf`.
    ///
    /// `all_vars` is the *original* variable list (before the one-point rule
    /// may have removed some of them) — every one of them is resolved into
    /// the [`Solution`] handed to `on_leaf`, since one-point-eliminated
    /// variables are bound in `initial_bindings` just as surely as the ones
    /// the search itself binds.
    #[allow(clippy::too_many_arguments)]
    fn drive(
        &mut self,
        all_vars: &[DataVariable],
        remaining_vars: Vec<DataVariable>,
        body: DataExpression,
        initial_bindings: BindingChain,
        reject: &DataExpression,
        bounded: bool,
        mut on_leaf: impl FnMut(&DataExpression, &Solution<'_>) -> ControlFlow<()>,
    ) -> Outcome {
        let limits = self.limits;
        let mut processed: usize = 0;
        let mut truncated = false;

        let mut queue: VecDeque<WorkItem> = VecDeque::new();
        queue.push_back(WorkItem {
            remaining: remaining_vars,
            body,
            bindings: initial_bindings,
            depth: 0,
        });

        while let Some(item) = queue.pop_front() {
            if item.body == *reject {
                continue;
            }

            if item.remaining.is_empty() {
                self.solution_buf.clear();
                self.solution_buf.extend(item.bindings.resolve_all(all_vars));
                let solution = Solution {
                    values: &self.solution_buf,
                };
                match on_leaf(&item.body, &solution) {
                    ControlFlow::Break(()) => return Outcome::Stopped,
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
                    let elements = self.cached_finite_elements(sort_id);
                    for element in elements {
                        let new_body = self.rewrite_singleton(&item.body, &variable, &element);
                        queue.push_back(WorkItem {
                            remaining: rest.to_vec(),
                            body: new_body,
                            bindings: item.bindings.extend(variable.clone(), element),
                            depth: item.depth,
                        });
                    }
                }
                SortEnumerability::InfiniteEnumerable => {
                    if bounded && item.depth >= limits.max_depth {
                        truncated = true;
                        continue;
                    }

                    // `self.plans` is a `&'a SortPlans` field (a reference,
                    // not owned data), so copying it out borrows nothing of
                    // `self` — `constructors` can be iterated while `self` is
                    // mutated below to build each candidate's fresh variables
                    // and rewrite the body.
                    let plans = self.plans;
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
                            let fresh = self.fresh_variable(plans.plan(argument_sort).sort());
                            arguments.push(DataExpression::from(fresh.clone()));
                            fresh_vars.push(fresh);
                        }

                        let value: DataExpression = if arguments.is_empty() {
                            constructor.symbol().clone().into()
                        } else {
                            DataApplication::with_args(constructor.symbol(), &arguments).into()
                        };

                        let new_body = self.rewrite_singleton(&item.body, &variable, &value);

                        let mut new_remaining = rest.to_vec();
                        new_remaining.extend(fresh_vars);

                        queue.push_back(WorkItem {
                            remaining: new_remaining,
                            body: new_body,
                            bindings: item.bindings.extend(variable.clone(), value),
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

    /// Returns `sort_id`'s materialised element list, computing and caching it
    /// on first use (§6.2). Only ever called for a [`SortEnumerability::Finite`]
    /// sort, whose constructor argument sorts are themselves finite by
    /// construction (`SortPlans::build`'s finiteness fixpoint), so the
    /// recursion in [`Enumerator::materialize_finite_elements`] always
    /// terminates.
    fn cached_finite_elements(&mut self, sort_id: SortPlanId) -> Vec<DataExpression> {
        if let Some(elements) = self.finite_elements.get(&sort_id) {
            return elements.clone();
        }
        let elements = self.materialize_finite_elements(sort_id);
        self.finite_elements.insert(sort_id, elements.clone());
        elements
    }

    fn materialize_finite_elements(&mut self, sort_id: SortPlanId) -> Vec<DataExpression> {
        let plans = self.plans;
        let mut result = Vec::new();

        for constructor in plans.plan(sort_id).constructors() {
            if constructor.arity() == 0 {
                result.push(constructor.symbol().clone().into());
                continue;
            }

            let mut per_argument: Vec<Vec<DataExpression>> = Vec::with_capacity(constructor.arity());
            for &argument_sort in constructor.arguments() {
                per_argument.push(self.cached_finite_elements(argument_sort));
            }

            for combination in cartesian_product(&per_argument) {
                let value: DataExpression = DataApplication::with_args(constructor.symbol(), &combination).into();
                result.push(self.rewriter.rewrite(&value));
            }
        }

        result
    }

    /// Substitutes `variable ↦ value` into `body` and rewrites the result.
    /// `value` is always a fresh constructor application (or a cached finite
    /// element, itself already a rewriter normal form), so it satisfies
    /// [`merc_sabre::utilities::RewriteSubstitution::get`]'s "already normal"
    /// contract.
    fn rewrite_singleton(
        &mut self,
        body: &DataExpression,
        variable: &DataVariable,
        value: &DataExpression,
    ) -> DataExpression {
        let mut singleton = HashMap::new();
        singleton.insert(variable.clone(), value.clone());
        self.rewriter.rewrite_with(body, &singleton)
    }

    fn fresh_variable(&mut self, sort: &SortExpression) -> DataVariable {
        self.generator.generate("v", sort.copy())
    }
}

/// Returns the canonical `Bool` literal `true`/`false` (mCRL2's
/// `sort_bool::true_`/`false_`).
pub(crate) fn bool_literal(value: bool) -> DataExpression {
    let name = if value { "true" } else { "false" };
    let bool_sort = SortExpression::from(BasicSort::new("Bool"));
    DataFunctionSymbol::with_sort(name, bool_sort.copy()).into()
}

/// Returns the cartesian product of `lists`, as one combination per element
/// of the result, each holding one element from every list in order. Empty
/// for an empty `lists`... actually returns a single empty combination, the
/// identity for the product, which is what an arity-0 caller needs.
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
