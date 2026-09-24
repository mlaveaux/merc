#![forbid(unsafe_code)]

use std::ops::ControlFlow;

use merc_aterm::Term;
use merc_data::AndExpressionRef;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_data::DataVariable;
use merc_data::DataVariableRef;
use merc_data::EqualExpressionRef;
use merc_data::ImpliesExpressionRef;
use merc_data::NotEqualExpressionRef;
use merc_data::NotExpressionRef;
use merc_data::OrExpressionRef;
use merc_data::is_and;
use merc_data::is_data_variable;
use merc_data::is_equal;
use merc_data::is_implies;
use merc_data::is_not;
use merc_data::is_not_equal;
use merc_data::is_or;
use merc_data::visit_data_expr;
use merc_sabre::RewriteEngine;
use merc_utilities::Step;

use crate::binding::BindingArena;
use crate::binding::BindingChain;
use crate::binding::BindingGuard;
use crate::binding::SingleSubstitution;

/// Which quantifier's one-point identity [`apply_one_point_rule`] may use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OnePointPolarity {
    Existential,
    Universal,
}

/// Runs [`apply_one_point_rule`] to a fixpoint under sum/existential polarity.
///
/// `condition` is the goal body to search for one-point (dis)equations over
/// `vars`, rewritten to its residual form in place. `rest` is *not* searched
/// but still has each eliminated variable's binding substituted in. This is
/// useful for summands for example.
///
/// `condition` must already be in normal form; see [`apply_one_point_rule`].
pub fn simplify_one_point<R: RewriteEngine>(
    rewriter: &mut R,
    vars: &[DataVariable],
    condition: &mut DataExpression,
    rest: &mut [DataExpression],
) {
    let mut arena = BindingArena::default();
    let mut guard = arena.chain.write();
    let (_remaining_vars, residual, bindings) = apply_one_point_rule(
        rewriter,
        &mut guard,
        vars.to_vec(),
        condition.clone(),
        OnePointPolarity::Existential,
    );
    *condition = residual;

    // Every binding's value mentions only variables outside `vars`.
    let substitution = BindingArena::substitution(&guard, bindings);
    for term in rest {
        *term = rewriter.rewrite_with(term, &substitution);
    }
}

/// Applies the one-point rule for `polarity` to `(vars, body)` to a fixpoint:
/// repeatedly finds a top-level (dis)equation `x == e` / `e == x` fixing one of
/// the still-unbound variables to a term mentioning none of them, binds
/// `x := e` directly instead of enumerating its sort, and rewrites the residual
/// body under that binding.
///
/// Returns the narrowed variable list, the rewritten residual body, and the
/// accumulated bindings.
///
/// `body` must already be in normal form: every subterm of a normal form is
/// itself one, which is what makes the `e` side of a matched (dis)equation
/// usable directly as a substitution image without re-rewriting it.
pub(crate) fn apply_one_point_rule<R: RewriteEngine>(
    rewriter: &mut R,
    guard: &mut BindingGuard<'_>,
    mut vars: Vec<DataVariable>,
    mut body: DataExpression,
    polarity: OnePointPolarity,
) -> (Vec<DataVariable>, DataExpression, BindingChain) {
    let mut bindings = BindingChain::default();

    while let Some((variable, value)) = find_one_point_equation(&vars, &body, polarity) {
        let position = vars
            .iter()
            .position(|v| *v == variable)
            .expect("find_one_point_equation only returns variables from `vars`");
        vars.remove(position);

        bindings = BindingArena::extend(guard, bindings, &variable.copy(), &value.copy());
        body = rewriter.rewrite_with(&body, &SingleSubstitution::new(variable.copy(), value.copy()));
    }

    (vars, body, bindings)
}

/// Finds the first top-level (dis)equation of `body` fixing some `x ∈ vars` to
/// a term `e` mentioning none of `vars` (including `x` itself), and returns
/// `(x, e)`. Which connective is split on, and which shape counts, is decided
/// by `polarity`.
fn find_one_point_equation<'a>(
    vars: &[DataVariable],
    body: &'a DataExpression,
    polarity: OnePointPolarity,
) -> Option<(DataVariable, DataExpression)> {
    let equation = |term: DataExpressionRef<'a>| {
        is_equal(&term).then(|| {
            let equal = EqualExpressionRef::from(Term::copy(&term));
            (equal.lhs(), equal.rhs())
        })
    };

    // Iterative rather than recursive: a chain of connectives can be arbitrarily deep.
    let mut stack = vec![body.copy()];
    while let Some(term) = stack.pop() {
        let sides = match polarity {
            OnePointPolarity::Existential if is_and(&term) => {
                let and = AndExpressionRef::from(Term::copy(&term));
                stack.push(and.rhs());
                stack.push(and.lhs());
                continue;
            }
            OnePointPolarity::Universal if is_or(&term) => {
                let or = OrExpressionRef::from(Term::copy(&term));
                stack.push(or.rhs());
                stack.push(or.lhs());
                continue;
            }
            OnePointPolarity::Existential => equation(term),
            // The shapes that deny an equality: `e1 != e2`, `e1 == e2 => φ` and `!(e1 == e2)`.
            OnePointPolarity::Universal if is_not_equal(&term) => {
                let not_equal = NotEqualExpressionRef::from(Term::copy(&term));
                Some((not_equal.lhs(), not_equal.rhs()))
            }
            OnePointPolarity::Universal if is_implies(&term) => {
                equation(ImpliesExpressionRef::from(Term::copy(&term)).lhs())
            }
            OnePointPolarity::Universal if is_not(&term) => {
                equation(NotExpressionRef::from(Term::copy(&term)).operand())
            }
            OnePointPolarity::Universal => None,
        };

        let Some((lhs, rhs)) = sides else {
            continue;
        };

        for (side, other) in [(&lhs, &rhs), (&rhs, &lhs)] {
            if !is_data_variable(side) {
                continue;
            }

            let variable = DataVariableRef::from(Term::copy(side));
            if vars.iter().any(|v| v.copy() == variable) && !mentions_any(other, vars) {
                return Some((variable.protect(), other.protect()));
            }
        }
    }
    None
}

/// Returns whether `term` mentions any variable in `vars`.
fn mentions_any(term: &DataExpressionRef<'_>, vars: &[DataVariable]) -> bool {
    visit_data_expr(term, (), |expr, context| {
        if is_data_variable(expr) {
            let variable = DataVariableRef::from(Term::copy(expr));
            if vars.iter().any(|v| v.copy() == variable) {
                return ControlFlow::Break(());
            }
        }
        ControlFlow::Continue(Step::Into(context))
    })
    .is_some()
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
    use merc_data::bool_literal;
    use merc_data::make_and;
    use merc_data::make_equal;
    use merc_data::make_implies;
    use merc_data::make_not;
    use merc_data::make_or;
    use merc_sabre::RewriteSpecification;
    use merc_sabre::Rule;
    use merc_sabre::SabreRewriter;

    use crate::binding::BindingArena;

    use super::OnePointPolarity;
    use super::apply_one_point_rule;

    fn d_sort() -> SortExpression {
        SortExpression::from(BasicSort::new("D"))
    }

    fn constant(name: &str) -> DataExpression {
        DataFunctionSymbol::with_sort(name, d_sort().copy()).into()
    }

    fn variable(name: &str) -> DataVariable {
        DataVariable::with_sort(name, d_sort().copy())
    }

    /// A small, hand-built rewrite system standing in for the `==`/`&&`
    /// fragment of the `Bool` prelude: `eq(x, x) = true`, `and(true, y) = y`,
    /// `and(false, y) = false`.
    fn rewriter() -> SabreRewriter {
        let x: DataExpression = variable("x").into();
        let bool_sort = SortExpression::from(BasicSort::new("Bool"));
        let y: DataExpression = DataVariable::with_sort("y", bool_sort.copy()).into();

        let spec = RewriteSpecification::new(vec![
            Rule::new(make_equal(d_sort(), x.clone(), x), bool_literal(true)),
            Rule::new(make_and(bool_literal(true), y.clone()), y.clone()),
            Rule::new(make_and(bool_literal(false), y), bool_literal(false)),
        ]);
        SabreRewriter::new(&spec)
    }

    #[test]
    fn test_universal_binds_a_negated_equality_disjunct() {
        // `∀n. (!(n == five) || φ(n))` ≡ `φ(five)`.
        let mut rewriter = rewriter();
        let n = variable("n");
        let body = make_or(
            make_not(make_equal(d_sort(), n.clone().into(), constant("five"))),
            bool_literal(true),
        );

        let mut arena = BindingArena::default();
        let mut guard = arena.chain.write();
        let (vars, _residual, bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut guard,
            vec![n.clone()],
            body,
            OnePointPolarity::Universal,
        );

        assert!(vars.is_empty(), "the universal rule must bind `n`");
        let mut resolved = Vec::new();
        BindingArena::resolve(&guard, &mut rewriter, &mut arena.scratch, bindings, &[n], &mut resolved);
        assert_eq!(resolved, vec![constant("five")]);
    }

    #[test]
    fn test_universal_binds_an_implication_antecedent() {
        // `∀n. (n == five => φ(n))` ≡ `φ(five)`.
        let mut rewriter = rewriter();
        let n = variable("n");
        let body = make_implies(
            make_equal(d_sort(), n.clone().into(), constant("five")),
            bool_literal(true),
        );

        let mut arena = BindingArena::default();
        let mut guard = arena.chain.write();
        let (vars, _residual, bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut guard,
            vec![n.clone()],
            body,
            OnePointPolarity::Universal,
        );

        assert!(vars.is_empty(), "the universal rule must bind `n`");
        let mut resolved = Vec::new();
        BindingArena::resolve(&guard, &mut rewriter, &mut arena.scratch, bindings, &[n], &mut resolved);
        assert_eq!(resolved, vec![constant("five")]);
    }

    #[test]
    fn test_universal_ignores_a_plain_equality_conjunct() {
        // The existential shape: binding `n` here would narrow `∀n. n == five`
        // to the one point that satisfies it, turning a false quantifier true.
        let mut rewriter = rewriter();
        let n = variable("n");
        let body = make_and(
            make_equal(d_sort(), n.clone().into(), constant("five")),
            bool_literal(true),
        );

        let mut arena = BindingArena::default();
        let mut guard = arena.chain.write();
        let (vars, residual, _bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut guard,
            vec![n.clone()],
            body.clone(),
            OnePointPolarity::Universal,
        );

        assert_eq!(vars, vec![n]);
        assert_eq!(residual, body);
    }

    #[test]
    fn test_existential_ignores_a_negated_equality_disjunct() {
        // The dual of the test above: `∃n. (!(n == five) || φ)` is not `φ(five)`.
        let mut rewriter = rewriter();
        let n = variable("n");
        let body = make_or(
            make_not(make_equal(d_sort(), n.clone().into(), constant("five"))),
            bool_literal(true),
        );

        let mut arena = BindingArena::default();
        let mut guard = arena.chain.write();
        let (vars, residual, _bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut guard,
            vec![n.clone()],
            body.clone(),
            OnePointPolarity::Existential,
        );

        assert_eq!(vars, vec![n]);
        assert_eq!(residual, body);
    }

    #[test]
    fn test_single_conjunct_binds_directly() {
        let mut rewriter = rewriter();
        let n = variable("n");
        let body = make_and(
            make_equal(d_sort(), n.clone().into(), constant("five")),
            bool_literal(true),
        );

        let mut arena = BindingArena::default();
        let mut guard = arena.chain.write();
        let (vars, residual, bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut guard,
            vec![n.clone()],
            body,
            OnePointPolarity::Existential,
        );

        assert!(vars.is_empty());
        assert_eq!(residual, bool_literal(true));
        let mut resolved = Vec::new();
        BindingArena::resolve(&guard, &mut rewriter, &mut arena.scratch, bindings, &[n], &mut resolved);
        assert_eq!(resolved, vec![constant("five")]);
    }

    #[test]
    fn test_chained_conjuncts_resolve_in_one_pass() {
        // `m` only becomes a one-point conjunct (`m == five`) after `n` has
        // been eliminated and the body re-rewritten; the fixpoint loop must
        // catch it too, in the same call.
        let mut rewriter = rewriter();
        let n = variable("n");
        let m = variable("m");
        let body = make_and(
            make_equal(d_sort(), n.clone().into(), constant("five")),
            make_equal(d_sort(), m.clone().into(), n.clone().into()),
        );

        let mut arena = BindingArena::default();
        let mut guard = arena.chain.write();
        let (vars, residual, bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut guard,
            vec![n.clone(), m.clone()],
            body,
            OnePointPolarity::Existential,
        );

        assert!(vars.is_empty());
        assert_eq!(residual, bool_literal(true));
        let mut resolved = Vec::new();
        BindingArena::resolve(
            &guard,
            &mut rewriter,
            &mut arena.scratch,
            bindings,
            &[n, m],
            &mut resolved,
        );
        assert_eq!(resolved, vec![constant("five"), constant("five")]);
    }

    #[test]
    fn test_no_conjunct_leaves_variable_untouched() {
        let mut rewriter = rewriter();
        let n = variable("n");
        let f_sort: SortExpression = SortArrow::new(&[d_sort()], d_sort()).into();
        let f = DataFunctionSymbol::with_sort("f", f_sort.copy());
        let body: DataExpression = DataApplication::with_args(&f, &[DataExpression::from(n.clone())]).into();

        let mut arena = BindingArena::default();
        let mut guard = arena.chain.write();
        let (vars, residual, _bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut guard,
            vec![n.clone()],
            body.clone(),
            OnePointPolarity::Existential,
        );

        assert_eq!(vars, vec![n]);
        assert_eq!(residual, body);
    }

    #[test]
    fn test_self_referential_conjunct_is_not_a_one_point_rule() {
        // `n == f(n)` mentions `n` on both sides; binding it would be circular.
        let mut rewriter = rewriter();
        let n = variable("n");
        let f_sort: SortExpression = SortArrow::new(&[d_sort()], d_sort()).into();
        let f = DataFunctionSymbol::with_sort("f", f_sort.copy());
        let f_n: DataExpression = DataApplication::with_args(&f, &[DataExpression::from(n.clone())]).into();
        let body = make_equal(d_sort(), n.clone().into(), f_n);

        let mut arena = BindingArena::default();
        let mut guard = arena.chain.write();
        let (vars, residual, _bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut guard,
            vec![n.clone()],
            body.clone(),
            OnePointPolarity::Existential,
        );

        assert_eq!(vars, vec![n]);
        assert_eq!(residual, body);
    }
}
