use merc_data::BasicSort;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataFunctionSymbol;
use merc_data::DataVariable;
use merc_data::SortArrow;
use merc_data::SortExpression;
use merc_enumerate::QuantifierKind;
use merc_enumerate::simplify_one_point;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;
use merc_sabre::Rule;
use merc_sabre::SabreRewriter;

fn d_sort() -> SortExpression {
    SortExpression::from(BasicSort::new("D"))
}

fn constant(name: &str) -> DataExpression {
    DataFunctionSymbol::with_sort(name, d_sort().copy()).into()
}

fn variable(name: &str) -> DataVariable {
    DataVariable::with_sort(name, d_sort().copy())
}

fn equal(lhs: &DataExpression, rhs: &DataExpression) -> DataExpression {
    DataExpression::equal(&d_sort(), lhs, rhs)
}

/// `f(argument)` for a unary `f: D -> D`.
fn f(argument: DataExpression) -> DataExpression {
    let f_sort: SortExpression = SortArrow::new(&[d_sort()], &d_sort()).into();
    DataApplication::with_args(&DataFunctionSymbol::with_sort("f", f_sort.copy()), &[argument]).into()
}

/// A small, hand-built rewrite system standing in for the `==`/`&&` fragment of the `Bool`
/// prelude: `eq(x, x) = true`, `and(true, y) = y`, `and(false, y) = false`.
fn rewriter() -> SabreRewriter {
    let x: DataExpression = variable("x").into();
    let y: DataExpression = DataVariable::with_sort("y", SortExpression::bool().copy()).into();

    let spec = RewriteSpecification::new(vec![
        Rule::new(equal(&x, &x), DataExpression::bool_literal(true)),
        Rule::new(DataExpression::and(&DataExpression::bool_literal(true), &y), y.clone()),
        Rule::new(
            DataExpression::and(&DataExpression::bool_literal(false), &y),
            DataExpression::bool_literal(false),
        ),
    ]);
    SabreRewriter::new(&spec)
}

/// Runs [simplify_one_point] and returns the remaining variables, the residual condition, and
/// `vars` with the eliminated ones replaced by their value.
fn simplify(
    kind: QuantifierKind,
    vars: &[DataVariable],
    condition: DataExpression,
) -> (Vec<DataVariable>, DataExpression, Vec<DataExpression>) {
    let mut condition = condition;
    let mut rest: Vec<DataExpression> = vars.iter().map(|v| v.clone().into()).collect();
    let remaining = simplify_one_point(&mut rewriter(), kind, vars, &mut condition, &mut rest);
    (remaining, condition, rest)
}

#[test]
fn test_universal_binds_a_negated_equality_disjunct() {
    // `∀n. (!(n == five) || φ(n))` ≡ `φ(five)`, here with `φ(n) = (n == n)`.
    let n = variable("n");
    let n_expr: DataExpression = n.clone().into();
    let body = DataExpression::or(
        &DataExpression::negation(&equal(&n_expr, &constant("five"))),
        &equal(&n_expr, &n_expr),
    );

    let (vars, residual, values) = simplify(QuantifierKind::Forall, std::slice::from_ref(&n), body);

    assert!(vars.is_empty(), "the universal rule must bind `n`");
    assert_eq!(values, [constant("five")]);
    // The residual is the body with `n := five`, rewritten.
    let five_five = equal(&constant("five"), &constant("five"));
    let expected = DataExpression::or(&DataExpression::negation(&five_five), &five_five);
    assert_eq!(residual, rewriter().rewrite(&expected));
}

#[test]
fn test_universal_binds_an_implication_antecedent() {
    // `∀n. (n == five => φ(n))` ≡ `φ(five)`.
    let n = variable("n");
    let body = DataExpression::implies(
        &equal(&n.clone().into(), &constant("five")),
        &DataExpression::bool_literal(false),
    );

    let (vars, residual, values) = simplify(QuantifierKind::Forall, std::slice::from_ref(&n), body);

    assert!(vars.is_empty(), "the universal rule must bind `n`");
    assert_eq!(values, [constant("five")]);
    let expected = DataExpression::implies(
        &equal(&constant("five"), &constant("five")),
        &DataExpression::bool_literal(false),
    );
    assert_eq!(residual, rewriter().rewrite(&expected));
}

#[test]
fn test_universal_ignores_a_plain_equality_conjunct() {
    // The existential shape: binding `n` here would narrow `∀n. n == five` to the one point that
    // satisfies it, turning a false quantifier true.
    let n = variable("n");
    let body = DataExpression::and(
        &equal(&n.clone().into(), &constant("five")),
        &DataExpression::bool_literal(true),
    );

    let (vars, residual, values) = simplify(QuantifierKind::Forall, std::slice::from_ref(&n), body.clone());

    assert_eq!(vars, std::slice::from_ref(&n));
    assert_eq!(residual, body);
    assert_eq!(values, [DataExpression::from(n)]);
}

#[test]
fn test_existential_ignores_a_negated_equality_disjunct() {
    // The dual of the test above: `∃n. (!(n == five) || φ)` is not `φ(five)`.
    let n = variable("n");
    let body = DataExpression::or(
        &DataExpression::negation(&equal(&n.clone().into(), &constant("five"))),
        &DataExpression::bool_literal(true),
    );

    let (vars, residual, values) = simplify(QuantifierKind::Exists, std::slice::from_ref(&n), body.clone());

    assert_eq!(vars, std::slice::from_ref(&n));
    assert_eq!(residual, body);
    assert_eq!(values, [DataExpression::from(n)]);
}

#[test]
fn test_existential_binds_an_equality_conjunct_on_either_side() {
    let n = variable("n");
    for equation in [
        equal(&n.clone().into(), &constant("five")),
        equal(&constant("five"), &n.clone().into()),
    ] {
        let body = DataExpression::and(&equation, &DataExpression::bool_literal(true));

        let (vars, residual, values) = simplify(QuantifierKind::Exists, std::slice::from_ref(&n), body);

        assert!(vars.is_empty());
        assert_eq!(residual, DataExpression::bool_literal(true));
        assert_eq!(values, [constant("five")]);
    }
}

#[test]
fn test_chained_conjuncts_resolve_in_one_pass() {
    // `m` only becomes a one-point conjunct (`m == five`) after `n` has been eliminated and the
    // body re-rewritten; the fixpoint loop must catch it too, in the same call.
    let n = variable("n");
    let m = variable("m");
    let body = DataExpression::and(
        &equal(&n.clone().into(), &constant("five")),
        &equal(&m.clone().into(), &n.clone().into()),
    );

    let (vars, residual, values) = simplify(QuantifierKind::Exists, &[n, m], body);

    assert!(vars.is_empty());
    assert_eq!(residual, DataExpression::bool_literal(true));
    assert_eq!(values, [constant("five"), constant("five")]);
}

#[test]
fn test_only_the_eliminated_variables_are_removed_in_order() {
    // `∃a, n, b. n == five && f(a) == f(b)`: only `n` is fixed.
    let a = variable("a");
    let n = variable("n");
    let b = variable("b");
    let other = equal(&f(a.clone().into()), &f(b.clone().into()));
    let body = DataExpression::and(&equal(&n.clone().into(), &constant("five")), &other);

    let (vars, residual, values) = simplify(QuantifierKind::Exists, &[a.clone(), n, b.clone()], body);

    assert_eq!(vars, [a.clone(), b.clone()]);
    assert_eq!(residual, other);
    assert_eq!(values, [a.into(), constant("five"), b.into()]);
}

/// A value may mention a variable that is not quantified here, such as one of an enclosing
/// quantifier.
#[test]
fn test_value_may_mention_a_free_variable() {
    let n = variable("n");
    let x: DataExpression = variable("x").into();
    let body = DataExpression::and(
        &equal(&n.clone().into(), &f(x.clone())),
        &DataExpression::bool_literal(true),
    );

    let (vars, residual, values) = simplify(QuantifierKind::Exists, std::slice::from_ref(&n), body);

    assert!(vars.is_empty());
    assert_eq!(residual, DataExpression::bool_literal(true));
    assert_eq!(values, [f(x)]);
}

#[test]
fn test_no_conjunct_leaves_variable_untouched() {
    let n = variable("n");
    let body = f(n.clone().into());

    let (vars, residual, values) = simplify(QuantifierKind::Exists, std::slice::from_ref(&n), body.clone());

    assert_eq!(vars, std::slice::from_ref(&n));
    assert_eq!(residual, body);
    assert_eq!(values, [DataExpression::from(n)]);
}

#[test]
fn test_self_referential_conjunct_is_not_a_one_point_rule() {
    // `n == f(n)` mentions `n` on both sides; binding it would be circular.
    let n = variable("n");
    let body = equal(&n.clone().into(), &f(n.clone().into()));

    let (vars, residual, _values) = simplify(QuantifierKind::Exists, std::slice::from_ref(&n), body.clone());

    assert_eq!(vars, [n]);
    assert_eq!(residual, body);
}

/// An equation between two quantified variables does not fix either of them.
#[test]
fn test_equation_between_quantified_variables_is_not_a_one_point_rule() {
    let n = variable("n");
    let m = variable("m");
    let body = equal(&n.clone().into(), &m.clone().into());

    let (vars, residual, _values) = simplify(QuantifierKind::Exists, &[n.clone(), m.clone()], body.clone());

    assert_eq!(vars, [n, m]);
    assert_eq!(residual, body);
}
