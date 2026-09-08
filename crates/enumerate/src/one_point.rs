#![forbid(unsafe_code)]

use std::ops::ControlFlow;
use std::rc::Rc;

use ahash::AHashSet;
use merc_aterm::Term;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_data::DataVariable;
use merc_data::DataVariableRef;
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

/// Which quantifier's one-point identity [`apply_one_point_rule`] may use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OnePointPolarity {
    Existential,
    Universal,
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
///
/// See `docs/developer/enumeration.md` on the merc website for what this
/// static, once-per-goal pass does and does not reach.
pub(crate) fn apply_one_point_rule<R: RewriteEngine>(
    rewriter: &mut R,
    arena: &mut BindingArena,
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

        bindings = arena.extend(bindings, Rc::new(variable), value);
        body = rewriter.rewrite_with(&body, &arena.substitution(bindings));
    }

    (vars, body, bindings)
}

/// Finds the first top-level (dis)equation of `body` fixing some `x ∈ vars` to
/// a term `e` mentioning none of `vars` (including `x` itself), and returns
/// `(x, e)`. Which connective is split on, and which shape counts, is decided
/// by `polarity`.
type BinaryDecomposer = fn(&DataExpression) -> Option<(DataExpression, DataExpression)>;

fn find_one_point_equation(
    vars: &[DataVariable],
    body: &DataExpression,
    polarity: OnePointPolarity,
) -> Option<(DataVariable, DataExpression)> {
    let (connective, extract): (BinaryDecomposer, BinaryDecomposer) = match polarity {
        OnePointPolarity::Existential => (is_and, as_equation),
        OnePointPolarity::Universal => (is_or, as_disequation),
    };

    for branch in split_on(body, connective) {
        let Some((lhs, rhs)) = extract(&branch) else {
            continue;
        };

        if let Some(pair) = one_point_candidate(vars, &lhs, &rhs) {
            return Some(pair);
        }

        if let Some(pair) = one_point_candidate(vars, &rhs, &lhs) {
            return Some(pair);
        }
    }
    None
}

/// Returns the two sides of `term` if it states an equality (`e1 == e2`).
fn as_equation(term: &DataExpression) -> Option<(DataExpression, DataExpression)> {
    is_equal(term)
}

/// Returns the two sides of the equality `term` *denies*.
fn as_disequation(term: &DataExpression) -> Option<(DataExpression, DataExpression)> {
    if let Some(sides) = is_not_equal(term) {
        return Some(sides);
    }

    if let Some((antecedent, _)) = is_implies(term) {
        return as_equation(&antecedent);
    }

    if let Some(operand) = is_not(term) {
        return as_equation(&operand);
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

/// Yields `body`'s top-level `&&`-conjuncts, left to right, flattening nested
/// conjunctions lazily as they're consumed. A non-`&&` term yields itself as a
/// single leaf.
pub(crate) fn split_conjuncts(body: &DataExpression) -> impl Iterator<Item = DataExpression> {
    split_on(body, is_and)
}

/// Yields the leaves of `body`'s top-level `connective` chain, left to right,
/// flattening lazily as they're consumed. A term for which `connective`
/// returns `None` yields itself as a single leaf.
///
/// Iterative rather than recursive: such a chain can be arbitrarily deep.
fn split_on(body: &DataExpression, connective: BinaryDecomposer) -> impl Iterator<Item = DataExpression> {
    let mut stack = vec![body.clone()];
    std::iter::from_fn(move || {
        while let Some(term) = stack.pop() {
            if let Some((lhs, rhs)) = connective(&term) {
                stack.push(rhs);
                stack.push(lhs);
            } else {
                return Some(term);
            }
        }
        None
    })
}

/// Returns whether `term` mentions any variable in `vars`.
fn mentions_any(term: &DataExpression, vars: &[DataVariable]) -> bool {
    let free = free_variables(term);
    vars.iter().any(|v| free.contains(v))
}

/// Returns every variable occurring free in `term`.
pub(crate) fn free_variables(term: &DataExpression) -> AHashSet<DataVariable> {
    let mut free = AHashSet::new();

    let _: Option<()> = visit_data_expr(&term.copy(), (), |expr: &DataExpressionRef<'_>, context| {
        if is_data_variable(expr) {
            free.insert(DataVariableRef::from(Term::copy(expr)).protect());
        }
        ControlFlow::Continue(Step::Into(context))
    });

    free
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
    use merc_data::make_equal;
    use merc_data::make_implies;
    use merc_data::make_not;
    use merc_data::make_or;
    use merc_sabre::RewriteSpecification;
    use merc_sabre::Rule;
    use merc_sabre::SabreRewriter;

    use crate::binding::BindingArena;
    use crate::binding::BindingChain;

    use super::OnePointPolarity;
    use super::apply_one_point_rule;

    // A small, hand-built rewrite system standing in for the `==`/`&&`
    // fragment of the `Bool` prelude.

    fn d_sort() -> SortExpression {
        SortExpression::from(BasicSort::new("D"))
    }

    fn bool_sort() -> SortExpression {
        SortExpression::from(BasicSort::new("Bool"))
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
            Rule::new(eq(x.clone(), x), true_lit()),
            Rule::new(and(true_lit(), y.clone()), y),
            Rule::new(and(false_lit(), bool_variable("y").into()), false_lit()),
        ]);
        SabreRewriter::new(&spec)
    }

    fn eq(lhs: DataExpression, rhs: DataExpression) -> DataExpression {
        make_equal(d_sort(), lhs, rhs)
    }

    fn and(lhs: DataExpression, rhs: DataExpression) -> DataExpression {
        make_and(lhs, rhs)
    }

    fn or(lhs: DataExpression, rhs: DataExpression) -> DataExpression {
        make_or(lhs, rhs)
    }

    fn implies(lhs: DataExpression, rhs: DataExpression) -> DataExpression {
        make_implies(lhs, rhs)
    }

    fn not(argument: DataExpression) -> DataExpression {
        make_not(argument)
    }

    /// Resolves `variable`'s binding out of `bindings` to a ground value.
    fn resolved(
        rewriter: &mut SabreRewriter,
        arena: &mut BindingArena,
        bindings: BindingChain,
        variable: DataVariable,
    ) -> DataExpression {
        let mut values = Vec::new();
        arena.resolve_all(rewriter, bindings, &[variable], &mut values);
        values.pop().expect("one variable resolves to one value")
    }

    #[test]
    fn test_universal_binds_a_negated_equality_disjunct() {
        // `∀n. (!(n == five) || φ(n))` ≡ `φ(five)`.
        let mut rewriter = rewriter();
        let n = variable("n");
        let body = or(not(eq(n.clone().into(), constant("five"))), true_lit());

        let mut arena = BindingArena::default();
        let (vars, _residual, bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut arena,
            vec![n.clone()],
            body,
            OnePointPolarity::Universal,
        );

        assert!(vars.is_empty(), "the universal rule must bind `n`");
        assert_eq!(resolved(&mut rewriter, &mut arena, bindings, n), constant("five"));
    }

    #[test]
    fn test_universal_binds_an_implication_antecedent() {
        // `∀n. (n == five => φ(n))` ≡ `φ(five)`.
        let mut rewriter = rewriter();
        let n = variable("n");
        let body = implies(eq(n.clone().into(), constant("five")), true_lit());

        let mut arena = BindingArena::default();
        let (vars, _residual, bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut arena,
            vec![n.clone()],
            body,
            OnePointPolarity::Universal,
        );

        assert!(vars.is_empty(), "the universal rule must bind `n`");
        assert_eq!(resolved(&mut rewriter, &mut arena, bindings, n), constant("five"));
    }

    #[test]
    fn test_universal_ignores_a_plain_equality_conjunct() {
        // The existential shape: binding `n` here would narrow `∀n. n == five`
        // to the one point that satisfies it, turning a false quantifier true.
        let mut rewriter = rewriter();
        let n = variable("n");
        let body = and(eq(n.clone().into(), constant("five")), true_lit());

        let mut arena = BindingArena::default();
        let (vars, residual, _bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut arena,
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
        let body = or(not(eq(n.clone().into(), constant("five"))), true_lit());

        let mut arena = BindingArena::default();
        let (vars, residual, _bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut arena,
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
        let body = and(eq(n.clone().into(), constant("five")), true_lit());

        let mut arena = BindingArena::default();
        let (vars, residual, bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut arena,
            vec![n.clone()],
            body,
            OnePointPolarity::Existential,
        );

        assert!(vars.is_empty());
        assert_eq!(residual, true_lit());
        let mut resolved = Vec::new();
        arena.resolve_all(&mut rewriter, bindings, &[n], &mut resolved);
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
        let body = and(
            eq(n.clone().into(), constant("five")),
            eq(m.clone().into(), n.clone().into()),
        );

        let mut arena = BindingArena::default();
        let (vars, residual, bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut arena,
            vec![n.clone(), m.clone()],
            body,
            OnePointPolarity::Existential,
        );

        assert!(vars.is_empty());
        assert_eq!(residual, true_lit());
        let mut resolved = Vec::new();
        arena.resolve_all(&mut rewriter, bindings, &[n, m], &mut resolved);
        assert_eq!(resolved, vec![constant("five"), constant("five")]);
    }

    #[test]
    fn test_no_conjunct_leaves_variable_untouched() {
        let mut rewriter = rewriter();
        let n = variable("n");
        let body: DataExpression = DataApplication::with_args(&f_symbol(), &[DataExpression::from(n.clone())]).into();

        let mut arena = BindingArena::default();
        let (vars, residual, _bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut arena,
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
        let f_n: DataExpression = DataApplication::with_args(&f_symbol(), &[DataExpression::from(n.clone())]).into();
        let body = eq(n.clone().into(), f_n);

        let mut arena = BindingArena::default();
        let (vars, residual, _bindings) = apply_one_point_rule(
            &mut rewriter,
            &mut arena,
            vec![n.clone()],
            body.clone(),
            OnePointPolarity::Existential,
        );

        assert_eq!(vars, vec![n]);
        assert_eq!(residual, body);
    }
}
