use ahash::AHashSet;
use ahash::HashMap;
use ahash::HashMapExt;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_sabre::RewriteEngine;

use crate::enumerator::bool_literal;
use crate::enumerator::cartesian_product;
use crate::sort_plan::SortEnumerability;
use crate::sort_plan::SortPlanId;
use crate::sort_plan::SortPlans;

/// A deliberately simple, unoptimized enumerator, for differential and
/// property-based testing against [`Enumerator`](crate::Enumerator) — the
/// same role [`merc_sabre::NaiveRewriter`] plays for the rewrite engines.
///
/// Where [`Enumerator`](crate::Enumerator) instantiates one variable at a
/// time through a breadth-first work queue (§4.1), normalising the goal
/// incrementally and pruning branches early, [`NaiveEnumerator`] takes the
/// opposite, obviously-correct approach: materialise every ground term of
/// each variable's sort up to a size bound, substitute *all* variables at
/// once, and rewrite once per combination. No incremental normalisation, no
/// early rejection, no one-point rule, no fairness scheme to get right — just
/// brute force. Two independent algorithms agreeing on the same result set is
/// much stronger evidence of correctness than either alone.
///
/// Only ever finds solutions reachable within `max_size` constructor
/// applications per variable, so it is a *reference for bounded goals*, not a
/// general substitute for [`Enumerator::enumerate`](crate::Enumerator::enumerate)
/// (which never bounds its search at all — see
/// [`EnumerationLimits`](crate::EnumerationLimits)'s doc comment on why).
/// Comparisons against it are only meaningful for goals known to have every
/// solution within `max_size`.
pub struct NaiveEnumerator<'a, R: RewriteEngine> {
    rewriter: &'a mut R,
    plans: &'a SortPlans,
    max_size: u32,
    /// Memoises `terms_up_to_size` per `(sort, size)`, since the same sort is
    /// revisited once per co-enumerated variable of that sort and once per
    /// recursive constructor argument.
    cache: HashMap<(SortPlanId, u32), Vec<DataExpression>>,
}

impl<'a, R: RewriteEngine> NaiveEnumerator<'a, R> {
    /// Builds a naive enumerator that considers ground terms with at most
    /// `max_size` constructor applications per variable.
    pub fn new(rewriter: &'a mut R, plans: &'a SortPlans, max_size: u32) -> Self {
        NaiveEnumerator {
            rewriter,
            plans,
            max_size,
            cache: HashMap::new(),
        }
    }

    /// Returns every ground substitution of `vars` (as a value vector in
    /// `vars` order) for which `rewrite(bodyσ)` is the `Bool` literal `true`,
    /// as a *set* — neither this nor [`Enumerator`](crate::Enumerator)
    /// promises an order.
    pub fn enumerate_all(&mut self, vars: &[DataVariable], body: &DataExpression) -> AHashSet<Vec<DataExpression>> {
        let per_variable: Vec<Vec<DataExpression>> = vars
            .iter()
            .map(|v| match self.plans.get(&v.sort()) {
                Some(sort_id) => self.terms_up_to_size(sort_id, self.max_size),
                None => Vec::new(),
            })
            .collect();

        let true_literal = bool_literal(true);
        let mut results = AHashSet::new();
        for combination in cartesian_product(&per_variable) {
            let mut sigma = HashMap::new();
            for (variable, value) in vars.iter().zip(&combination) {
                sigma.insert(variable.clone(), value.clone());
            }

            if self.rewriter.rewrite_with(body, &sigma) == true_literal {
                results.insert(combination);
            }
        }
        results
    }

    /// Every ground term of `sort_id` reachable within `size` constructor
    /// applications (a finite sort's full element list, once `size` is large
    /// enough to reach every element — see `SortPlan::min_size`).
    fn terms_up_to_size(&mut self, sort_id: SortPlanId, size: u32) -> Vec<DataExpression> {
        if let Some(cached) = self.cache.get(&(sort_id, size)) {
            return cached.clone();
        }

        let mut result = Vec::new();
        if size > 0
            && matches!(
                self.plans.plan(sort_id).enumerability(),
                SortEnumerability::Finite | SortEnumerability::InfiniteEnumerable
            )
        {
            let plans = self.plans;
            for constructor in plans.plan(sort_id).constructors() {
                if constructor.arity() == 0 {
                    result.push(constructor.symbol().clone().into());
                    continue;
                }

                // Each argument independently gets a budget of `size - 1`,
                // rather than precisely partitioning `size - 1` across the
                // arguments: a deliberate over-approximation (some resulting
                // terms exceed `size` total constructor applications) that
                // keeps this simple, at the cost of some wasted work — this
                // is the "naive" enumerator, not the optimised one.
                let per_argument: Vec<Vec<DataExpression>> = constructor
                    .arguments()
                    .iter()
                    .map(|&arg_sort| self.terms_up_to_size(arg_sort, size - 1))
                    .collect();

                for combination in cartesian_product(&per_argument) {
                    let value: DataExpression = DataApplication::with_args(constructor.symbol(), &combination).into();
                    result.push(self.rewriter.rewrite(&value));
                }
            }
        }

        self.cache.insert((sort_id, size), result.clone());
        result
    }
}
