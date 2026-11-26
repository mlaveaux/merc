#![forbid(unsafe_code)]

use std::ops::ControlFlow;

use merc_data::BoolConnective;
use merc_data::BoolKind;
use merc_data::DataExpression;
use merc_data::DataExpressionKind;
use merc_data::DataExpressionRef;
use merc_data::DataVariable;
use merc_data::visit_data_expr;
use merc_sabre::RewriteEngine;
use merc_sabre::utilities::SliceSubstitution;
use merc_utilities::ChainIter;
use merc_utilities::ChainStack;
use merc_utilities::Step;

use crate::QuantifierKind;
use crate::binding::BindingArena;
use crate::binding::BindingChain;
use crate::binding::BindingGuard;
use crate::binding::SingleSubstitution;

/// Applies [the one-point rule](https://mercorg.github.io/merc-website/developer/rewriting/enumeration/#the-one-point-rule) to a fixpoint,
/// for a quantifier of `kind` over `vars`, or a `sum` as [`QuantifierKind::Exists`].
///
/// The [`Enumerator`](crate::Enumerator) never applies the rule itself, so this is a
/// preprocessing step for the caller.
///
/// `condition` is the goal body to search for one-point (dis)equations over
/// `vars`, rewritten to its residual form in place. `rest` is *not* searched
/// but still has each eliminated variable's binding substituted in. This is
/// useful for summands for example. Returns the variables that were not
/// eliminated, in `vars` order.
///
/// `condition` must already be in normal form, so that the `e` of a matched
/// equation can be substituted without rewriting it again.
pub fn simplify_one_point<R: RewriteEngine>(
    rewriter: &mut R,
    kind: QuantifierKind,
    vars: &[DataVariable],
    condition: &mut DataExpression,
    rest: &mut [DataExpression],
) -> Vec<DataVariable> {
    let mut arena = BindingArena::default();
    arena.chain.write(|guard| {
        let (remaining_vars, residual, bindings) =
            apply_one_point_rule(rewriter, guard, vars.to_vec(), condition.clone(), kind);
        *condition = residual;

        // Every binding's value mentions only variables outside `vars`, so
        // the chain needs no resolving and applies as a plain substitution.
        let eliminated: Vec<DataVariable> = vars.iter().filter(|v| !remaining_vars.contains(v)).cloned().collect();
        let values: Vec<DataExpression> = eliminated
            .iter()
            .map(|v| {
                BindingArena::lookup(guard, bindings, &v.copy())
                    .expect("an eliminated variable is bound")
                    .protect()
            })
            .collect();
        let substitution = SliceSubstitution::new(&eliminated, &values);
        for term in rest {
            *term = rewriter.rewrite_with(term, &substitution);
        }

        remaining_vars
    })
}

/// Applies the one-point rule for `kind` to `(vars, body)` to a fixpoint:
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
/// See [the one-point rule](https://mercorg.github.io/merc-website/developer/rewriting/enumeration/#the-one-point-rule).
fn apply_one_point_rule<R: RewriteEngine>(
    rewriter: &mut R,
    guard: &mut BindingGuard<'_, '_>,
    mut vars: Vec<DataVariable>,
    mut body: DataExpression,
    kind: QuantifierKind,
) -> (Vec<DataVariable>, DataExpression, BindingChain) {
    let mut bindings = BindingChain::default();

    while let Some((variable, value)) = find_one_point_equation(&vars, &body, kind) {
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
/// by `kind`.
fn find_one_point_equation<'a>(
    vars: &[DataVariable],
    body: &'a DataExpression,
    kind: QuantifierKind,
) -> Option<(DataVariable, DataExpression)> {
    let equation = |term: DataExpressionRef<'a>| match term.bool_kind() {
        BoolKind::Equal(lhs, rhs) => Some((lhs, rhs)),
        _ => None,
    };

    // The connective whose operands are searched: a conjunct of an existential, or a disjunct of a
    // universal, quantifier.
    let connective = match kind {
        QuantifierKind::Exists => BoolConnective::And,
        QuantifierKind::Forall => BoolConnective::Or,
    };

    let mut storage = ChainStack::<DataExpressionRef<'a>>::new();
    for term in ChainIter::new(body.copy(), connective, &mut storage) {
        let sides = match (kind, term.bool_kind()) {
            (QuantifierKind::Exists, _) => equation(term),
            // The shapes that deny an equality: `e1 != e2`, `e1 == e2 => φ` and `!(e1 == e2)`.
            (QuantifierKind::Forall, BoolKind::NotEqual(lhs, rhs)) => Some((lhs, rhs)),
            (QuantifierKind::Forall, BoolKind::Implies(operand, _) | BoolKind::Not(operand)) => equation(operand),
            (QuantifierKind::Forall, _) => None,
        };

        let Some((lhs, rhs)) = sides else {
            continue;
        };

        for (side, other) in [(&lhs, &rhs), (&rhs, &lhs)] {
            let DataExpressionKind::Variable(variable) = side.kind() else {
                continue;
            };

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
        if let DataExpressionKind::Variable(variable) = expr.kind()
            && vars.iter().any(|v| v.copy() == variable)
        {
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(Step::Into(context))
    })
    .is_some()
}
