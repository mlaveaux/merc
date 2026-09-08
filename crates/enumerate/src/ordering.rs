use std::cmp::Reverse;

use ahash::HashMap;
use ahash::HashMapExt;
use merc_data::DataExpression;
use merc_data::DataVariable;

use crate::one_point::free_variables;
use crate::one_point::split_conjuncts;

/// Reorders `vars` so that variables constrained by more of `body`'s top-level
/// `&&`-conjuncts — and especially those that are the *only* one of `vars`
/// still free in some conjunct — are enumerated first: binding a sole survivor
/// makes its conjunct ground immediately, so the search's reject predicate can
/// prune the branch a step sooner.
///
/// A static approximation, computed once per goal from the (already
/// one-point-reduced) body: it counts how many of `vars` each conjunct still
/// mentions, never what a conjunct evaluates to. Ties keep their relative order
/// from `vars`, so a goal it can say nothing about is left alone.
pub(crate) fn order_variables_by_constraints(vars: Vec<DataVariable>, body: &DataExpression) -> Vec<DataVariable> {
    if vars.len() <= 1 {
        return vars;
    }

    let mut mention_count: HashMap<DataVariable, usize> = HashMap::new();
    let mut solo_count: HashMap<DataVariable, usize> = HashMap::new();

    for conjunct in split_conjuncts(body) {
        let free = free_variables(&conjunct);
        let mentioned: Vec<&DataVariable> = vars.iter().filter(|v| free.contains(*v)).collect();
        for variable in &mentioned {
            *mention_count.entry((*variable).clone()).or_insert(0) += 1;
        }
        if let [only] = mentioned.as_slice() {
            *solo_count.entry((*only).clone()).or_insert(0) += 1;
        }
    }

    let mut indexed: Vec<(usize, DataVariable)> = vars.into_iter().enumerate().collect();
    indexed.sort_by_key(|(original_index, variable)| {
        let solo = solo_count.get(variable).copied().unwrap_or(0);
        let mentions = mention_count.get(variable).copied().unwrap_or(0);
        (Reverse(solo), Reverse(mentions), *original_index)
    });
    indexed.into_iter().map(|(_, variable)| variable).collect()
}

#[cfg(test)]
mod tests {
    use merc_data::BasicSort;
    use merc_data::DataApplication;
    use merc_data::DataExpression;
    use merc_data::DataFunctionSymbol;
    use merc_data::DataVariable;
    use merc_data::SortArrow;
    use merc_data::SortExpression;

    use super::order_variables_by_constraints;

    fn d_sort() -> SortExpression {
        SortExpression::from(BasicSort::new("D"))
    }

    fn bool_sort() -> SortExpression {
        SortExpression::from(BasicSort::new("Bool"))
    }

    fn variable(name: &str) -> DataVariable {
        DataVariable::with_sort(name, d_sort().copy())
    }

    fn and(lhs: DataExpression, rhs: DataExpression) -> DataExpression {
        let sort: SortExpression = SortArrow::new(&[bool_sort(), bool_sort()], bool_sort()).into();
        DataApplication::with_args(&DataFunctionSymbol::with_sort("&&", sort.copy()), &[lhs, rhs]).into()
    }

    fn unary_pred(name: &str, argument: DataExpression) -> DataExpression {
        let sort: SortExpression = SortArrow::new(&[d_sort()], bool_sort()).into();
        DataApplication::with_args(&DataFunctionSymbol::with_sort(name, sort.copy()), &[argument]).into()
    }

    fn binary_pred(name: &str, lhs: DataExpression, rhs: DataExpression) -> DataExpression {
        let sort: SortExpression = SortArrow::new(&[d_sort(), d_sort()], bool_sort()).into();
        DataApplication::with_args(&DataFunctionSymbol::with_sort(name, sort.copy()), &[lhs, rhs]).into()
    }

    #[test]
    fn test_variable_appearing_in_more_conjuncts_comes_first() {
        // `n` is constrained by both conjuncts, `m` only by the second.
        let n = variable("n");
        let m = variable("m");
        let body = and(
            unary_pred("p", n.clone().into()),
            binary_pred("q", n.clone().into(), m.clone().into()),
        );

        let ordered = order_variables_by_constraints(vec![m.clone(), n.clone()], &body);

        assert_eq!(ordered, vec![n, m]);
    }

    #[test]
    fn test_solo_survivor_outranks_mere_mention_count() {
        // `m` is the sole survivor of one conjunct (binding it makes that
        // conjunct ground); `n` merely co-occurs with `m` in two others but is
        // never alone. The solo conjunct should still win.
        let n = variable("n");
        let m = variable("m");
        let body = and(
            and(
                binary_pred("q", n.clone().into(), m.clone().into()),
                binary_pred("r", n.clone().into(), m.clone().into()),
            ),
            unary_pred("p", m.clone().into()),
        );

        let ordered = order_variables_by_constraints(vec![n.clone(), m.clone()], &body);

        assert_eq!(ordered, vec![m, n]);
    }

    #[test]
    fn test_no_constraints_preserves_original_order() {
        let n = variable("n");
        let m = variable("m");
        let unrelated = unary_pred("p", DataFunctionSymbol::with_sort("c", d_sort().copy()).into());

        let ordered = order_variables_by_constraints(vec![n.clone(), m.clone()], &unrelated);

        assert_eq!(ordered, vec![n, m]);
    }

    #[test]
    fn test_single_variable_is_returned_unchanged() {
        let n = variable("n");
        let body = unary_pred("p", n.clone().into());

        let ordered = order_variables_by_constraints(vec![n.clone()], &body);

        assert_eq!(ordered, vec![n]);
    }
}
