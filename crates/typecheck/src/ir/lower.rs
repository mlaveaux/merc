use std::ops::ControlFlow;

use log::trace;

use merc_syntax::DataExpr;
use merc_syntax::DataExprBinaryOp;
use merc_syntax::DataExprKind;
use merc_syntax::Span;
use merc_syntax::Traverse;
use merc_syntax::UntypedDataSpecification;

/// The Appendix-B name of the function update operation, see
/// `signature::standard_sorts::function_update_text`.
const FUNCTION_UPDATE_NAME: &str = "@func_update";

/// Lowers every data expression in the equations of `spec` with
/// [lower_data_expr], so later passes only see [DataExprKind::Application] for
/// operators.
pub(crate) fn lower_data_expressions(spec: &mut UntypedDataSpecification) {
    for eqn_spec in &mut spec.equation_declarations {
        for equation in &mut eqn_spec.equations {
            if let Some(condition) = &mut equation.condition {
                lower_in_place(condition);
            }

            lower_in_place(&mut equation.lhs);
            lower_in_place(&mut equation.rhs);
        }
    }

    debug_assert!(
        spec.equation_declarations
            .iter()
            .flat_map(|eqn_spec| &eqn_spec.equations)
            .all(|equation| {
                equation.condition.as_ref().is_none_or(is_lowered)
                    && is_lowered(&equation.lhs)
                    && is_lowered(&equation.rhs)
            }),
        "every equation expression must be lowered"
    );
}

/// Rewrites the built-in operator nodes of a data expression into applications
/// of the equally-named function symbol, bottom-up:
///
///   - `Binary`/`Unary` become `Application` of the operator's surface name
///     (`x == y` becomes `==(x, y)`), matching the Appendix-B declarations.
///   - `List` literals become cons chains (`[x, y]` becomes `|>(x, |>(y, []))`).
///   - `FunctionUpdate` becomes an `@func_update` application.
///
/// Literals (`Number`, `Bool`, and the empty/enumerated set and bag forms) are
/// kept as dedicated nodes: sort inference treats them specially, constraining
/// their sort structurally instead of through a declared symbol. The result
/// satisfies [is_lowered]; lowering is idempotent.
pub(crate) fn lower_data_expr(mut expr: DataExpr) -> DataExpr {
    expr.transform(|expr| {
        // The node is taken out so that its parts can be moved into the replacement.
        let (node, span) = std::mem::replace(expr, DataExprKind::EmptyList.into()).into_parts();
        *expr = match node {
            DataExprKind::Binary { op, lhs, rhs } => apply(op.to_string(), vec![*lhs, *rhs], span),
            DataExprKind::Unary { op, expr } => apply(op.to_string(), vec![*expr], span),
            DataExprKind::List(elements) => elements
                .into_iter()
                .rev()
                .fold(DataExprKind::EmptyList.spanned(span.clone()), |tail, head| {
                    apply(DataExprBinaryOp::Cons.to_string(), vec![head, tail], span.clone())
                }),
            DataExprKind::FunctionUpdate { expr, update } => apply(
                FUNCTION_UPDATE_NAME.to_string(),
                vec![*expr, update.expr, update.update],
                span,
            ),
            node => node.spanned(span),
        };
    });

    expr
}

/// Returns true when the expression contains none of the nodes that
/// [lower_data_expr] rewrites; the postcondition of lowering and the
/// precondition of Phase-3 sort inference.
pub(crate) fn is_lowered(expr: &DataExpr) -> bool {
    expr.visit(|expr| match &expr.node {
        DataExprKind::Binary { .. }
        | DataExprKind::Unary { .. }
        | DataExprKind::List(_)
        | DataExprKind::FunctionUpdate { .. } => ControlFlow::Break(()),
        _ => ControlFlow::Continue(()),
    })
    .is_none()
}

fn apply(name: String, arguments: Vec<DataExpr>, span: Span) -> DataExpr {
    DataExprKind::Application {
        function: Box::new(DataExprKind::Id(name).spanned(span.clone())),
        arguments,
    }
    .spanned(span)
}

fn lower_in_place(expr: &mut DataExpr) {
    let owned = std::mem::replace(expr, DataExprKind::EmptyList.into());
    // The original text is only rendered when trace logging is enabled.
    let original = log::log_enabled!(log::Level::Trace).then(|| owned.to_string());
    *expr = lower_data_expr(owned);
    if let Some(original) = original {
        let lowered = expr.to_string();
        if lowered != original {
            trace!("lower: '{original}' lowered to '{lowered}'");
        }
    }
}

#[cfg(test)]
mod tests {
    use rand::rngs::StdRng;
    use test_case::test_case;

    use merc_syntax::DataExpr;
    use merc_syntax::DataExprKind;
    use merc_syntax::UntypedDataSpecification;
    use merc_syntax::random_boolean_data_expression;
    use merc_syntax::random_integer_data_expression;
    use merc_utilities::random_test;

    use crate::DataSpecification;
    use crate::is_lowered;
    use crate::lower_data_expr;
    use crate::lower_data_expressions;

    /// Parses `text` as the right-hand side of an equation.
    fn parse_expr(text: &str) -> DataExpr {
        let spec = UntypedDataSpecification::parse(&format!("map f: Bool; eqn f = {text};")).unwrap();
        spec.equation_declarations[0].equations[0].rhs.clone()
    }

    /// The printed form of the lowered expression.
    fn lowered(text: &str) -> String {
        let expr = lower_data_expr(parse_expr(text));
        assert!(is_lowered(&expr), "lowering {text} left an operator node: {expr:?}");
        expr.to_string()
    }

    #[test_case("x && y", "&&(x, y)"; "conj")]
    #[test_case("x || y", "||(x, y)"; "disj")]
    #[test_case("x => y", "=>(x, y)"; "implies")]
    #[test_case("x == y", "==(x, y)"; "equal")]
    #[test_case("x != y", "!=(x, y)"; "not equal")]
    #[test_case("x < y", "<(x, y)"; "less than")]
    #[test_case("x <= y", "<=(x, y)"; "less equal")]
    #[test_case("x > y", ">(x, y)"; "greater than")]
    #[test_case("x >= y", ">=(x, y)"; "greater equal")]
    #[test_case("x |> y", "|>(x, y)"; "cons")]
    #[test_case("x <| y", "<|(x, y)"; "snoc")]
    #[test_case("x in y", "in(x, y)"; "membership")]
    #[test_case("x ++ y", "++(x, y)"; "concat")]
    #[test_case("x . y", ".(x, y)"; "at")]
    #[test_case("x + y", "+(x, y)"; "add")]
    #[test_case("x - y", "-(x, y)"; "subtract")]
    #[test_case("x * y", "*(x, y)"; "multiply")]
    #[test_case("x / y", "/(x, y)"; "divide")]
    #[test_case("x div y", "div(x, y)"; "int div")]
    #[test_case("x mod y", "mod(x, y)"; "modulo")]
    fn test_lower_binary_operator(text: &str, expected: &str) {
        assert_eq!(lowered(text), expected);
    }

    #[test_case("!x", "!(x)"; "negation")]
    #[test_case("-x", "-(x)"; "unary minus")]
    #[test_case("#x", "#(x)"; "size")]
    #[test_case("-5", "-(5)"; "negative number literal parses as unary minus")]
    fn test_lower_unary_operator(text: &str, expected: &str) {
        assert_eq!(lowered(text), expected);
    }

    #[test_case("[x, y]", "|>(x, |>(y, []))"; "two elements")]
    #[test_case("[x]", "|>(x, [])"; "one element")]
    #[test_case("[]", "[]"; "empty list")]
    #[test_case("[[x], []]", "|>(|>(x, []), |>([], []))"; "nested lists")]
    #[test_case("[x + y]", "|>(+(x, y), [])"; "operator inside element")]
    fn test_lower_list_literal(text: &str, expected: &str) {
        assert_eq!(lowered(text), expected);
    }

    #[test]
    fn test_lower_function_update() {
        assert_eq!(lowered("f[x -> y]"), "@func_update(f, x, y)");
        assert_eq!(
            lowered("f[x -> y][a -> b]"),
            "@func_update(@func_update(f, x, y), a, b)"
        );
    }

    #[test_case("lambda n: Nat . n + 1", "(lambda n: Nat . +(n, 1))"; "lambda body")]
    #[test_case("forall n: Nat . n == n", "(forall n: Nat . ==(n, n))"; "quantifier body")]
    #[test_case("x + y whr y = z ++ w end", "+(x, y) whr y = ++(z, w) end"; "whr expression and assignments")]
    #[test_case("{ x + y }", "{ +(x, y) }"; "set element")]
    #[test_case("{ x: n + 1 }", "{ x: +(n, 1) }"; "bag element and multiplicity")]
    #[test_case("{ n: Nat | n < m }", "{ n: Nat | <(n, m) }"; "comprehension predicate")]
    #[test_case("g(x + y)", "g(+(x, y))"; "application arguments")]
    #[test_case("f[x -> y](z)", "@func_update(f, x, y)(z)"; "application function position")]
    fn test_lower_recurses(text: &str, expected: &str) {
        assert_eq!(lowered(text), expected);
    }

    /// Literals stay dedicated nodes (inference constrains them structurally),
    /// and `true` remains a [DataExprKind::Bool], not an identifier.
    #[test_case("true"; "boolean literal")]
    #[test_case("5"; "number literal")]
    #[test_case("{}"; "empty set")]
    #[test_case("{:}"; "empty bag")]
    fn test_lower_keeps_literals(text: &str) {
        assert_eq!(lowered(text), text);
    }

    #[test]
    fn test_boolean_literal_is_not_an_identifier() {
        assert!(matches!(
            lower_data_expr(parse_expr("true")).node,
            DataExprKind::Bool(true)
        ));
    }

    #[test]
    fn test_lower_data_expressions_covers_conditions() {
        let mut spec = UntypedDataSpecification::parse("map f: Bool; var n: Nat; eqn n < 2 -> f = n == n;").unwrap();
        lower_data_expressions(&mut spec);
        assert_eq!(
            spec.equation_declarations[0].equations[0].to_string(),
            "<(n, 2) -> f = ==(n, n)"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_lowering_is_idempotent() {
        let spec =
            UntypedDataSpecification::parse("map f: Bool; var b: Bool; c: Bool; n: Nat; m: Nat; eqn f = b;").unwrap();
        let variables = spec.equation_declarations[0].variables.clone();

        random_test(100, |rng: &mut StdRng| {
            for expr in [
                random_boolean_data_expression(rng, &variables),
                random_integer_data_expression(rng, &variables),
            ] {
                let once = lower_data_expr(expr);
                assert!(is_lowered(&once), "not lowered: {once}");
                assert_eq!(lower_data_expr(once.clone()), once, "lowering must be idempotent");
            }
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_from_untyped_lowers_user_and_system_equations() {
        let spec = DataSpecification::from_untyped(
            UntypedDataSpecification::parse(
                "
                sort D = struct c(n: Nat) | d;
                map f: List(Nat) -> Nat;
                var l: List(Nat);
                eqn f(l) = #l + 1;
            ",
            )
            .unwrap(),
        )
        .unwrap();

        for eqn_spec in spec
            .data_specification()
            .equation_declarations
            .iter()
            .chain(&spec.system_defined_specification().equation_declarations)
        {
            for equation in &eqn_spec.equations {
                assert!(equation.condition.as_ref().is_none_or(is_lowered), "{equation}");
                assert!(is_lowered(&equation.lhs), "{equation}");
                assert!(is_lowered(&equation.rhs), "{equation}");
            }
        }
    }
}

#[cfg(test)]
mod stack_depth_probe {
    //! `lower_data_expr` is one of the three `crates/typecheck` call sites
    //! `review/stack-overflow-recursion.md` named as depending on `Traverse::transform`'s own
    //! stack safety without having a regression test of its own; this is that test. The tree here
    //! is built directly, bypassing the parser (which has its own, unrelated recursion limit).
    use merc_syntax::DataExprKind;
    use merc_syntax::DataExprUnaryOp;
    use merc_syntax::Span;

    use super::is_lowered;
    use super::lower_data_expr;

    #[test]
    fn deeply_nested_unary_does_not_overflow_the_stack() {
        let mut expr = DataExprKind::Bool(true).spanned(Span::default());
        for _ in 0..100_000 {
            expr = DataExprKind::Unary {
                op: DataExprUnaryOp::Negation,
                expr: Box::new(expr),
            }
            .spanned(Span::default());
        }

        let lowered = lower_data_expr(expr);
        assert!(is_lowered(&lowered));
    }
}
