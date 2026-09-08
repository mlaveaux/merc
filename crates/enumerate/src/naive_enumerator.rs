use ahash::AHashSet;
use ahash::HashMap;
use ahash::HashMapExt;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_sabre::RewriteEngine;

use crate::enumeration_plan::EnumerationPlanId;
use crate::enumeration_plan::EnumerationPlans;
use crate::enumeration_plan::SortEnumerability;
use crate::enumerator::bool_literal;
use crate::enumerator::cartesian_product;

/// A deliberately simple, unoptimized enumerator, for differential and
/// property-based testing against [`Enumerator`](crate::Enumerator).
///
/// Materialises every ground term of each variable's sort up to a size bound,
/// substitutes *all* variables at once, and rewrites once per combination: no
/// incremental normalisation, no early rejection, no one-point rule, no
/// fairness scheme.
///
/// Only ever finds solutions reachable within `max_size` constructor
/// applications per variable, so it is a *reference for bounded goals*, not a
/// general substitute enumeration.
pub struct NaiveEnumerator<'a, R: RewriteEngine> {
    rewriter: &'a mut R,
    plans: &'a EnumerationPlans,
    max_size: u32,
    /// Memoises `terms_up_to_size` per `(sort, size)`, since the same sort is
    /// revisited once per co-enumerated variable of that sort and once per
    /// recursive constructor argument.
    cache: HashMap<(EnumerationPlanId, u32), Vec<DataExpression>>,
}

impl<'a, R: RewriteEngine> NaiveEnumerator<'a, R> {
    /// Builds a naive enumerator that considers ground terms with at most
    /// `max_size` constructor applications per variable.
    pub fn new(rewriter: &'a mut R, plans: &'a EnumerationPlans, max_size: u32) -> Self {
        NaiveEnumerator {
            rewriter,
            plans,
            max_size,
            cache: HashMap::new(),
        }
    }

    /// Returns every ground substitution of `vars` for which `rewrite(body, σ)`
    /// is the `Bool` literal `true`, as a *set*.
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
    /// applications.
    fn terms_up_to_size(&mut self, sort_id: EnumerationPlanId, size: u32) -> Vec<DataExpression> {
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
                // arguments: a deliberate over-approximation.
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
