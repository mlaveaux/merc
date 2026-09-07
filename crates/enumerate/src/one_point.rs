use std::ops::ControlFlow;

use ahash::AHashSet;
use ahash::HashMap;
use ahash::HashMapExt;
use merc_aterm::Term;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_data::DataVariable;
use merc_data::DataVariableRef;
use merc_data::is_data_application;
use merc_data::is_data_variable;
use merc_data::visit_data_expr;
use merc_sabre::RewriteEngine;
use merc_utilities::Step;

use crate::binding::BindingChain;

/// Applies the one-point rule to `(vars, body)` to a fixpoint: repeatedly finds
/// a top-level `&&`-conjunct `x == e` / `e == x` where `x` is one of the
/// still-unbound variables and `e` mentions none of them, binds `x := e`
/// directly instead of enumerating its sort, and rewrites the residual body
/// under that binding — which is what lets a *chain* of one-point conjuncts (`n
/// == 5 && m == n + 1`) resolve in one pass: after `n` is substituted away the
/// second conjunct becomes `m == 5 + 1`, itself a one-point conjunct on the
/// next iteration.
///
/// Returns the narrowed variable list, the rewritten residual body, and the
/// accumulated bindings.
///
/// `body` must already be in normal form. Every subterm of a normal form is
/// itself a normal form, which is what makes the `e` side of a matched conjunct
/// usable directly as a substitution image without re-rewriting it.
pub(crate) fn apply_one_point_rule<R: RewriteEngine>(
    rewriter: &mut R,
    mut vars: Vec<DataVariable>,
    mut body: DataExpression,
) -> (Vec<DataVariable>, DataExpression, BindingChain) {
    let mut bindings = BindingChain::default();

    while let Some((variable, value)) = find_one_point_conjunct(&vars, &body) {
        let position = vars
            .iter()
            .position(|v| *v == variable)
            .expect("find_one_point_conjunct only returns variables from `vars`");
        vars.remove(position);

        let mut singleton = HashMap::new();
        singleton.insert(variable.clone(), value.clone());
        body = rewriter.rewrite_with(&body, &singleton);

        bindings = bindings.extend(variable, value);
    }

    (vars, body, bindings)
}

/// Finds the first top-level conjunct of `body` of the shape `x == e` / `e ==
/// x` where `x ∈ vars` and `e` mentions none of `vars` (including `x`
/// itself), and returns `(x, e)`.
fn find_one_point_conjunct(vars: &[DataVariable], body: &DataExpression) -> Option<(DataVariable, DataExpression)> {
    for conjunct in split_conjuncts(body) {
        if !is_data_application(&conjunct) || conjunct.data_arguments().len() != 2 {
            continue;
        }
        if conjunct.data_function_symbol().name().value() != "==" {
            continue;
        }

        let lhs = conjunct.data_arg(0).protect();
        let rhs = conjunct.data_arg(1).protect();

        if let Some(pair) = one_point_candidate(vars, &lhs, &rhs) {
            return Some(pair);
        }
        if let Some(pair) = one_point_candidate(vars, &rhs, &lhs) {
            return Some(pair);
        }
    }
    None
}

/// If `side` is one of `vars` and `other` mentions none of `vars`, returns
/// `(side, other)`.
fn one_point_candidate(
    vars: &[DataVariable],
    side: &DataExpression,
    other: &DataExpression,
) -> Option<(DataVariable, DataExpression)> {
    if !is_data_variable(side) {
        return None;
    }
    let variable: DataVariable = side.clone().into();
    if !vars.contains(&variable) || mentions_any(other, vars) {
        return None;
    }
    Some((variable, other.clone()))
}

/// Splits `body` into its top-level `&&`-conjuncts, recursively flattening
/// nested conjunctions left to right. A non-`&&` term is a single-element
/// result.
fn split_conjuncts(body: &DataExpression) -> Vec<DataExpression> {
    let mut result = Vec::new();
    let mut stack = vec![body.clone()];
    while let Some(term) = stack.pop() {
        if is_data_application(&term)
            && term.data_arguments().len() == 2
            && term.data_function_symbol().name().value() == "&&"
        {
            stack.push(term.data_arg(1).protect());
            stack.push(term.data_arg(0).protect());
        } else {
            result.push(term);
        }
    }
    result
}

/// Returns whether `term` mentions any variable in `vars`.
fn mentions_any(term: &DataExpression, vars: &[DataVariable]) -> bool {
    let mut free = AHashSet::new();
    let _: Option<()> = visit_data_expr(&term.copy(), (), |expr: &DataExpressionRef<'_>, context| {
        if is_data_variable(expr) {
            free.insert(DataVariableRef::from(Term::copy(expr)).protect());
        }
        ControlFlow::Continue(Step::Into(context))
    });
    vars.iter().any(|v| free.contains(v))
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
    use merc_sabre::RewriteSpecification;
    use merc_sabre::Rule;
    use merc_sabre::SabreRewriter;

    use super::apply_one_point_rule;

    // A small, hand-built rewrite system standing in for the `==`/`&&`
    // fragment of the `Bool` prelude.

    fn d_sort() -> SortExpression {
        SortExpression::from(BasicSort::new("D"))
    }

    fn bool_sort() -> SortExpression {
        SortExpression::from(BasicSort::new("Bool"))
    }

    fn eq_symbol() -> DataFunctionSymbol {
        let sort: SortExpression = SortArrow::new(&[d_sort(), d_sort()], bool_sort()).into();
        DataFunctionSymbol::with_sort("==", sort.copy())
    }

    fn and_symbol() -> DataFunctionSymbol {
        let sort: SortExpression = SortArrow::new(&[bool_sort(), bool_sort()], bool_sort()).into();
        DataFunctionSymbol::with_sort("&&", sort.copy())
    }

    fn f_symbol() -> DataFunctionSymbol {
        let sort: SortExpression = SortArrow::new(&[d_sort()], d_sort()).into();
        DataFunctionSymbol::with_sort("f", sort.copy())
    }

    fn true_lit() -> DataExpression {
        DataFunctionSymbol::with_sort("true", bool_sort().copy()).into()
    }

    fn false_lit() -> DataExpression {
        DataFunctionSymbol::with_sort("false", bool_sort().copy()).into()
    }

    fn constant(name: &str) -> DataExpression {
        DataFunctionSymbol::with_sort(name, d_sort().copy()).into()
    }

    fn variable(name: &str) -> DataVariable {
        DataVariable::with_sort(name, d_sort().copy())
    }

    fn bool_variable(name: &str) -> DataVariable {
        DataVariable::with_sort(name, bool_sort().copy())
    }

    /// `eq(x, x) = true`, `and(true, y) = y`, `and(false, y) = false`.
    fn rewriter() -> SabreRewriter {
        let x: DataExpression = variable("x").into();
        let y: DataExpression = bool_variable("y").into();

        let spec = RewriteSpecification::new(vec![
            Rule::new(
                DataApplication::with_args(&eq_symbol(), &[x.clone(), x]).into(),
                true_lit(),
            ),
            Rule::new(
                DataApplication::with_args(&and_symbol(), &[true_lit(), y.clone()]).into(),
                y,
            ),
            Rule::new(
                DataApplication::with_args(&and_symbol(), &[false_lit(), bool_variable("y").into()]).into(),
                false_lit(),
            ),
        ]);
        SabreRewriter::new(&spec)
    }

    fn eq(lhs: DataExpression, rhs: DataExpression) -> DataExpression {
        DataApplication::with_args(&eq_symbol(), &[lhs, rhs]).into()
    }

    fn and(lhs: DataExpression, rhs: DataExpression) -> DataExpression {
        DataApplication::with_args(&and_symbol(), &[lhs, rhs]).into()
    }

    #[test]
    fn test_single_conjunct_binds_directly() {
        let mut rewriter = rewriter();
        let n = variable("n");
        let body = and(eq(n.clone().into(), constant("five")), true_lit());

        let (vars, residual, bindings) = apply_one_point_rule(&mut rewriter, vec![n.clone()], body);

        assert!(vars.is_empty());
        assert_eq!(residual, true_lit());
        assert_eq!(bindings.resolve_all(&[n]), vec![constant("five")]);
    }

    #[test]
    fn test_chained_conjuncts_resolve_in_one_pass() {
        // `m` only becomes a one-point conjunct (`m == five`) after `n` has
        // been eliminated and the body re-rewritten; the fixpoint loop must
        // catch it too, in the same call.
        let mut rewriter = rewriter();
        let n = variable("n");
        let m = variable("m");
        let body = and(
            eq(n.clone().into(), constant("five")),
            eq(m.clone().into(), n.clone().into()),
        );

        let (vars, residual, bindings) = apply_one_point_rule(&mut rewriter, vec![n.clone(), m.clone()], body);

        assert!(vars.is_empty());
        assert_eq!(residual, true_lit());
        assert_eq!(bindings.resolve_all(&[n, m]), vec![constant("five"), constant("five")]);
    }

    #[test]
    fn test_no_conjunct_leaves_variable_untouched() {
        let mut rewriter = rewriter();
        let n = variable("n");
        let body: DataExpression = DataApplication::with_args(&f_symbol(), &[DataExpression::from(n.clone())]).into();

        let (vars, residual, _bindings) = apply_one_point_rule(&mut rewriter, vec![n.clone()], body.clone());

        assert_eq!(vars, vec![n]);
        assert_eq!(residual, body);
    }

    #[test]
    fn test_self_referential_conjunct_is_not_a_one_point_rule() {
        // `n == f(n)` mentions `n` on both sides; binding it would be circular.
        let mut rewriter = rewriter();
        let n = variable("n");
        let f_n: DataExpression = DataApplication::with_args(&f_symbol(), &[DataExpression::from(n.clone())]).into();
        let body = eq(n.clone().into(), f_n);

        let (vars, residual, _bindings) = apply_one_point_rule(&mut rewriter, vec![n.clone()], body.clone());

        assert_eq!(vars, vec![n]);
        assert_eq!(residual, body);
    }
}
