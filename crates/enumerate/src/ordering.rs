#![forbid(unsafe_code)]

use std::cmp::Reverse;

use ahash::HashMap;
use ahash::HashMapExt;
use merc_data::DataExpression;
use merc_data::DataVariable;

use crate::one_point::free_variables;
use crate::one_point::split_conjuncts;

/// Per-variable `(solo_count, mention_count)` computed by
/// [`compute_variable_ranks`]: how many of `body`'s top-level `&&`-conjuncts
/// mention a variable, and how many it is the *sole* survivor of.
pub(crate) type VariableRanks = HashMap<DataVariable, (usize, usize)>;

/// Computes, for each of `vars`, how many of `body`'s top-level
/// `&&`-conjuncts still mention it, and how many it is the *only* one of
/// `vars` still free in — the input [`apply_variable_ranks`] needs to put
/// variables constrained by more of the goal first: binding a sole survivor
/// makes its conjunct ground immediately, so the search's reject predicate
/// can prune the branch a step sooner.
///
/// A static approximation: it counts how many of `vars` each conjunct still
/// mentions, never what a conjunct evaluates to.
pub(crate) fn compute_variable_ranks(vars: &[DataVariable], body: &DataExpression) -> VariableRanks {
    let mut ranks: VariableRanks = HashMap::new();

    for conjunct in split_conjuncts(body) {
        let free = free_variables(&conjunct);
        let mentioned: Vec<&DataVariable> = vars.iter().filter(|v| free.contains(*v)).collect();
        for variable in &mentioned {
            ranks.entry((*variable).clone()).or_insert((0, 0)).1 += 1;
        }
        if let [only] = mentioned.as_slice() {
            ranks.entry((*only).clone()).or_insert((0, 0)).0 += 1;
        }
    }

    ranks
}

/// Reorders `vars` by `ranks` (see [`compute_variable_ranks`]): a higher solo
/// count sorts first, ties broken by mention count, remaining ties keeping
/// `vars`'s original relative order.
pub(crate) fn apply_variable_ranks(vars: Vec<DataVariable>, ranks: &VariableRanks) -> Vec<DataVariable> {
    if vars.len() <= 1 {
        return vars;
    }

    let mut indexed: Vec<(usize, DataVariable)> = vars.into_iter().enumerate().collect();
    indexed.sort_by_key(|(original_index, variable)| {
        let (solo, mentions) = ranks.get(variable).copied().unwrap_or((0, 0));
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
    use merc_data::make_and;

    use super::apply_variable_ranks;
    use super::compute_variable_ranks;

    fn order_variables_by_constraints(vars: Vec<DataVariable>, body: &DataExpression) -> Vec<DataVariable> {
        let ranks = compute_variable_ranks(&vars, body);
        apply_variable_ranks(vars, &ranks)
    }

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
        make_and(lhs, rhs)
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
