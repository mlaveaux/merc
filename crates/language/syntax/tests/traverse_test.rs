//! Tests of the [Traverse] trait through its public API, on every node type.
//!
//! The trees built by hand (no parser) run under miri; the ones parsed with pest do not, since pest
//! calls into psm, which miri does not support.

use std::convert::Infallible;
use std::fmt::Display;
use std::ops::ControlFlow;

use merc_syntax::ActFrm;
use merc_syntax::ActFrmKind;
use merc_syntax::ComplexSort;
use merc_syntax::DataExpr;
use merc_syntax::DataExprBinaryOp;
use merc_syntax::DataExprKind;
use merc_syntax::DataExprUnaryOp;
use merc_syntax::MixedNode;
use merc_syntax::ModalityOperator;
use merc_syntax::PbesExpr;
use merc_syntax::PbesExprKind;
use merc_syntax::PresExpr;
use merc_syntax::PresExprKind;
use merc_syntax::ProcessExpr;
use merc_syntax::ProcessExprKind;
use merc_syntax::Recursion;
use merc_syntax::RegFrm;
use merc_syntax::RegFrmKind;
use merc_syntax::SortExpression;
use merc_syntax::SortExpressionKind;
use merc_syntax::StateFrm;
use merc_syntax::StateFrmKind;
use merc_syntax::StateFrmUnaryOp;
use merc_syntax::Traverse;
use merc_syntax::UntypedDataSpecification;
use merc_syntax::UntypedPbes;
use merc_syntax::UntypedPres;
use merc_syntax::UntypedProcessSpecification;
use merc_syntax::UntypedStateFrmSpec;
use merc_utilities::Step;

/// Depth of the deep-tree tests: deep enough to overflow the default 2 MiB test thread stack under
/// native recursion, reduced under miri, which is far slower.
const DEPTH: usize = if cfg!(miri) { 200 } else { 100_000 };

/// Number of children of the wide-tree tests.
const WIDTH: usize = if cfg!(miri) { 50 } else { 10_000 };

fn id(name: &str) -> DataExpr {
    DataExprKind::Id(name.to_string()).into()
}

fn app(function: &str, arguments: Vec<DataExpr>) -> DataExpr {
    DataExprKind::Application {
        function: Box::new(id(function)),
        arguments,
    }
    .into()
}

fn not(expr: DataExpr) -> DataExpr {
    DataExprKind::Unary {
        op: DataExprUnaryOp::Negation,
        expr: Box::new(expr),
    }
    .into()
}

fn and(lhs: DataExpr, rhs: DataExpr) -> DataExpr {
    DataExprKind::Binary {
        op: DataExprBinaryOp::Conj,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    }
    .into()
}

/// `f(g(a), (! b) && c)`, printed as `f(g(a), ((! b) && c))`.
fn sample() -> DataExpr {
    app("f", vec![app("g", vec![id("a")]), and(not(id("b")), id("c"))])
}

/// `!!...!x` with `depth` negations.
fn deep_negation(depth: usize) -> DataExpr {
    let mut expr = id("x");
    for _ in 0..depth {
        expr = not(expr);
    }
    expr
}

/// The pre-order of `node` according to [Traverse::visit], each node printed.
fn visit_order<N: Traverse + Display>(node: &N) -> Vec<String> {
    let mut order = Vec::new();
    node.visit::<(), _>(|node| {
        order.push(node.to_string());
        ControlFlow::Continue(())
    });
    order
}

/// The pre-order of `node` according to [Traverse::apply], which walks the mutable children.
/// Without replacements, `apply` must return the tree unchanged.
fn apply_order<N: Traverse + Display + Clone + PartialEq + std::fmt::Debug>(node: &N) -> Vec<String> {
    let mut order = Vec::new();
    let result = node.clone().apply::<Infallible, _>(|node| {
        order.push(node.to_string());
        Ok(None)
    });
    assert_eq!(result.as_ref(), Ok(node));
    order
}

/// The post-order of `node` according to [Traverse::transform].
fn transform_order<N: Traverse + Display + Clone + Default>(node: &N) -> Vec<String> {
    let mut order = Vec::new();
    let mut node = node.clone();
    node.transform(|node| order.push(node.to_string()));
    order
}

/// Checks the pre-order of [Traverse::visit] and [Traverse::apply] and the post-order of
/// [Traverse::transform] of `node`, and that without rewriting all three leave the tree unchanged.
#[track_caller]
fn check_orders<N>(node: &N, preorder: &[&str], postorder: &[&str])
where
    N: Traverse + Display + Clone + Default + PartialEq + std::fmt::Debug,
{
    assert_eq!(visit_order(node), preorder, "visit order");
    assert_eq!(apply_order(node), preorder, "apply order");
    assert_eq!(transform_order(node), postorder, "transform order");

    let mut transformed = node.clone();
    transformed.transform(|_| {});
    assert_eq!(&transformed, node, "a no-op transform must reassemble the same tree");
}

#[test]
fn test_visit_apply_and_transform_orders_on_a_data_expression() {
    check_orders(
        &sample(),
        &[
            "f(g(a), ((! b) && c))",
            "f",
            "g(a)",
            "g",
            "a",
            "((! b) && c)",
            "(! b)",
            "b",
            "c",
        ],
        &[
            "f",
            "g",
            "a",
            "g(a)",
            "b",
            "(! b)",
            "c",
            "((! b) && c)",
            "f(g(a), ((! b) && c))",
        ],
    );
}

#[test]
fn test_visit_break_stops_at_once_and_returns_the_value() {
    let expr = sample();

    let mut visited = Vec::new();
    let found = expr.visit(|node| {
        visited.push(node.to_string());
        match &node.node {
            DataExprKind::Id(name) if name == "a" => ControlFlow::Break(name.clone()),
            _ => ControlFlow::Continue(()),
        }
    });

    assert_eq!(found.as_deref(), Some("a"));
    assert_eq!(visited, ["f(g(a), ((! b) && c))", "f", "g(a)", "g", "a"]);
}

#[test]
fn test_visit_without_break_returns_none() {
    assert_eq!(sample().visit::<(), _>(|_| ControlFlow::Continue(())), None);
}

#[test]
fn test_try_visit_stops_at_the_first_error() {
    let expr = sample();

    let mut visited = 0;
    let result: Result<Option<()>, String> = expr.try_visit(|node| {
        visited += 1;
        match &node.node {
            DataExprKind::Id(name) if name == "g" => Err(format!("error at {name}")),
            _ => Ok(ControlFlow::Continue(())),
        }
    });

    assert_eq!(result, Err("error at g".to_string()));
    assert_eq!(visited, 4);
}

#[test]
fn test_visit_with_prune_skips_only_the_children_of_the_pruned_node() {
    let expr = sample();

    let mut visited = Vec::new();
    let result = expr.visit_with::<(), (), Infallible, _>((), |node, context| {
        visited.push(node.to_string());
        Ok(ControlFlow::Continue(match &node.node {
            DataExprKind::Application { function, .. } if function.to_string() == "g" => Step::Prune,
            _ => Step::Into(context),
        }))
    });

    assert_eq!(result, Ok(None));
    assert_eq!(
        visited,
        ["f(g(a), ((! b) && c))", "f", "g(a)", "((! b) && c)", "(! b)", "b", "c"]
    );
}

#[test]
fn test_visit_with_threads_the_context_from_parent_to_children() {
    let expr = sample();

    let mut depths = Vec::new();
    let result = expr.visit_with::<usize, (), Infallible, _>(0, |node, depth| {
        if let DataExprKind::Id(name) = &node.node {
            depths.push((name.clone(), depth));
        }
        Ok(ControlFlow::Continue(Step::Into(depth + 1)))
    });

    assert_eq!(result, Ok(None));
    assert_eq!(
        depths,
        [
            ("f".to_string(), 1),
            ("g".to_string(), 2),
            ("a".to_string(), 2),
            ("b".to_string(), 3),
            ("c".to_string(), 2)
        ]
    );
}

#[test]
fn test_visit_with_reports_break_and_error() {
    let expr = sample();

    let found = expr.visit_with::<(), &str, Infallible, _>((), |node, _| {
        Ok(match &node.node {
            DataExprKind::Unary { .. } => ControlFlow::Break("unary"),
            _ => ControlFlow::Continue(Step::Into(())),
        })
    });
    assert_eq!(found, Ok(Some("unary")));

    let failed = expr.visit_with::<(), Infallible, &str, _>((), |node, _| match &node.node {
        DataExprKind::Binary { .. } => Err("binary"),
        _ => Ok(ControlFlow::Continue(Step::Into(()))),
    });
    assert_eq!(failed, Err("binary"));
}

#[test]
fn test_visit_children_visits_the_subtrees_below_the_root_only() {
    let expr = sample();

    let mut visited = Vec::new();
    let outcome: Recursion<Infallible, Infallible> = expr.visit_children((), &mut |node, context| {
        visited.push(node.to_string());
        Ok(ControlFlow::Continue(Step::Into(context)))
    });

    assert!(matches!(outcome, ControlFlow::Continue(())));
    assert_eq!(visited, ["f", "g(a)", "g", "a", "((! b) && c)", "(! b)", "b", "c"]);
}

#[test]
fn test_apply_replaces_every_match_and_does_not_descend_into_a_replacement() {
    // The replacement of `a` contains `a` again, which must not be replaced a second time.
    let expr = app("f", vec![id("a"), and(id("a"), id("b"))]);

    let mut seen = Vec::new();
    let result = expr.apply::<Infallible, _>(|node| {
        seen.push(node.to_string());
        assert!(seen.len() < 100, "apply descends into the replacements");
        Ok(match &node.node {
            DataExprKind::Id(name) if name == "a" => Some(app("h", vec![id("a")])),
            _ => None,
        })
    });

    assert_eq!(
        result.expect("the callback cannot fail").to_string(),
        "f(h(a), (h(a) && b))"
    );
    assert_eq!(seen, ["f(a, (a && b))", "f", "a", "(a && b)", "a", "b"]);
}

#[test]
fn test_apply_of_the_root_replaces_the_whole_tree() {
    let result = sample().apply::<Infallible, _>(|_| Ok(Some(id("z"))));
    assert_eq!(result, Ok(id("z")));
}

#[test]
fn test_apply_mut_stops_at_the_first_error() {
    let mut expr = app("f", vec![id("a"), id("b"), id("c")]);

    let result = expr.apply_mut(|node| match &node.node {
        DataExprKind::Id(name) if name == "a" => Ok(Some(id("x"))),
        DataExprKind::Id(name) if name == "b" => Err("b"),
        DataExprKind::Id(name) if name == "c" => Ok(Some(id("y"))),
        _ => Ok(None),
    });

    assert_eq!(result, Err("b"));
    assert_eq!(expr.to_string(), "f(x, b, c)");
}

#[test]
fn test_apply_with_breaks_and_keeps_what_was_rewritten() {
    let mut expr = app("f", vec![id("a"), id("b"), id("a")]);

    let found = expr.apply_with::<usize, &str, Infallible, _>(0, |node, depth| {
        Ok(match &node.node {
            DataExprKind::Id(name) if name == "a" => ControlFlow::Continue(Step::Replace(id(&format!("a{depth}")))),
            DataExprKind::Id(name) if name == "b" => ControlFlow::Break("stopped"),
            _ => ControlFlow::Continue(Step::Into(depth + 1)),
        })
    });

    assert_eq!(found, Ok(Some("stopped")));
    assert_eq!(expr.to_string(), "f(a1, b, a)");
}

#[test]
fn test_apply_with_prune_leaves_the_subtree_unchanged() {
    let mut expr = app("f", vec![app("g", vec![id("a")]), id("a")]);

    let result = expr.apply_with::<(), Infallible, Infallible, _>((), |node, context| {
        Ok(ControlFlow::Continue(match &node.node {
            DataExprKind::Application { function, .. } if function.to_string() == "g" => Step::Prune,
            DataExprKind::Id(name) if name == "a" => Step::Replace(id("z")),
            _ => Step::Into(context),
        }))
    });

    assert_eq!(result, Ok(None));
    assert_eq!(expr.to_string(), "f(g(a), z)");
}

#[test]
fn test_apply_children_never_replaces_the_root() {
    let mut expr = app("f", vec![id("a"), id("b")]);

    let outcome: Recursion<Infallible, Infallible> = expr.apply_children((), &mut |node, context| {
        Ok(ControlFlow::Continue(match &node.node {
            DataExprKind::Id(name) => Step::Replace(id(&name.to_uppercase())),
            _ => Step::Into(context),
        }))
    });

    assert!(matches!(outcome, ControlFlow::Continue(())));
    assert_eq!(expr.to_string(), "F(A, B)");

    let outcome: Recursion<Infallible, Infallible> =
        expr.apply_children((), &mut |_, _| Ok(ControlFlow::Continue(Step::Replace(id("z")))));
    assert!(matches!(outcome, ControlFlow::Continue(())));
    assert_eq!(expr.to_string(), "z(z, z)");
}

/// An event of a scoped walk: `enter`/`exit` of a printed node with its context.
#[derive(Debug, PartialEq)]
enum Event {
    Enter(String, usize),
    Exit(String, usize),
}

fn enter(node: &str, context: usize) -> Event {
    Event::Enter(node.to_string(), context)
}

fn exit(node: &str, context: usize) -> Event {
    Event::Exit(node.to_string(), context)
}

#[test]
fn test_visit_scoped_exits_every_node_innermost_first_including_pruned_ones() {
    let expr = app("f", vec![app("g", vec![id("a")]), id("b")]);

    let mut events = Vec::new();
    let result = expr.visit_scoped::<usize, Vec<Event>, (), Infallible, _, _>(
        0,
        &mut events,
        |node, depth, events| {
            events.push(Event::Enter(node.to_string(), depth));
            Ok(ControlFlow::Continue(match &node.node {
                DataExprKind::Application { function, .. } if function.to_string() == "g" => Step::Prune,
                _ => Step::Into(depth + 1),
            }))
        },
        |node, depth, events| events.push(Event::Exit(node.to_string(), depth)),
    );

    assert_eq!(result, Ok(None));
    assert_eq!(
        events,
        [
            enter("f(g(a), b)", 0),
            enter("f", 1),
            exit("f", 1),
            enter("g(a)", 1),
            exit("g(a)", 1),
            enter("b", 1),
            exit("b", 1),
            exit("f(g(a), b)", 0),
        ]
    );
}

#[test]
fn test_visit_scoped_unwinds_the_open_scopes_on_break_and_on_error() {
    let expr = app("f", vec![app("g", vec![id("a"), id("c")]), id("b")]);

    for fail in [false, true] {
        let mut events = Vec::new();
        let result = expr.visit_scoped::<usize, Vec<Event>, &str, &str, _, _>(
            0,
            &mut events,
            |node, depth, events| {
                events.push(Event::Enter(node.to_string(), depth));
                match &node.node {
                    DataExprKind::Id(name) if name == "a" && fail => Err("failed"),
                    DataExprKind::Id(name) if name == "a" => Ok(ControlFlow::Break("stopped")),
                    _ => Ok(ControlFlow::Continue(Step::Into(depth + 1))),
                }
            },
            |node, depth, events| events.push(Event::Exit(node.to_string(), depth)),
        );

        assert_eq!(result, if fail { Err("failed") } else { Ok(Some("stopped")) });
        // The stopping node itself was never entered successfully, so it is not exited; `c` and
        // `b` were never entered at all.
        assert_eq!(
            events,
            [
                enter("f(g(a, c), b)", 0),
                enter("f", 1),
                exit("f", 1),
                enter("g(a, c)", 1),
                enter("g", 2),
                exit("g", 2),
                enter("a", 2),
                exit("g(a, c)", 1),
                exit("f(g(a, c), b)", 0),
            ]
        );
    }
}

#[test]
fn test_apply_scoped_rewrites_in_place_and_exits_innermost_first() {
    let mut expr = app("f", vec![app("g", vec![id("a")]), id("b")]);

    // `state` is the stack of open scopes; `exit` records which scope it closes.
    let mut state: (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
    let result = expr.apply_scoped::<usize, (Vec<usize>, Vec<usize>), (), Infallible, _, _>(
        0,
        &mut state,
        |node, depth, (open, _)| {
            open.push(depth);
            Ok(ControlFlow::Continue(match &mut node.node {
                DataExprKind::Id(name) => {
                    *name = format!("{name}{depth}");
                    Step::Into(depth + 1)
                }
                DataExprKind::Application { function, .. } if function.to_string() == "g" => Step::Prune,
                _ => Step::Into(depth + 1),
            }))
        },
        |depth, (open, closed)| {
            assert_eq!(open.pop(), Some(depth), "exit must close the innermost open scope");
            closed.push(depth);
        },
    );

    assert_eq!(result, Ok(None));
    assert_eq!(expr.to_string(), "f1(g(a), b1)");
    assert!(state.0.is_empty());
    assert_eq!(state.1, [1, 1, 1, 0]);
}

#[test]
fn test_apply_scoped_unwinds_the_open_scopes_on_break() {
    let mut expr = app("f", vec![app("g", vec![id("a")]), id("b")]);

    let mut closed = Vec::new();
    let result = expr.apply_scoped::<usize, Vec<usize>, &str, Infallible, _, _>(
        0,
        &mut closed,
        |node, depth, _| {
            Ok(match &node.node {
                DataExprKind::Id(name) if name == "a" => ControlFlow::Break("stopped"),
                _ => ControlFlow::Continue(Step::Into(depth + 1)),
            })
        },
        |depth, closed| closed.push(depth),
    );

    assert_eq!(result, Ok(Some("stopped")));
    // `f` (depth 1) and `g` (depth 2) closed normally, then `g(a)` and the root are unwound.
    assert_eq!(closed, [1, 2, 1, 0]);
}

#[test]
fn test_transform_sees_rewritten_children_and_rewrites_the_root() {
    let mut expr = sample();

    let mut seen = Vec::new();
    expr.transform(|node| {
        seen.push(node.to_string());
        if let DataExprKind::Id(name) = &mut node.node {
            *name = name.to_uppercase();
        }
    });

    assert_eq!(expr.to_string(), "F(G(A), ((! B) && C))");
    assert_eq!(
        seen,
        [
            "f",
            "g",
            "a",
            "G(A)",
            "b",
            "(! B)",
            "c",
            "((! B) && C)",
            "F(G(A), ((! B) && C))"
        ]
    );
}

#[test]
fn test_transform_children_rewrites_the_children_but_not_the_root() {
    let mut expr = app("f", vec![id("a"), not(id("b"))]);

    let mut seen = Vec::new();
    let result: Result<(), Infallible> = expr.transform_children(&mut |node| {
        seen.push(node.to_string());
        *node = id("z");
        Ok(())
    });

    assert_eq!(result, Ok(()));
    assert_eq!(seen, ["f", "a", "b", "(! z)"]);
    assert_eq!(expr.to_string(), "z(z, z)");
}

#[test]
fn test_try_transform_stops_at_the_first_error() {
    let mut expr = app("f", vec![id("a"), id("b"), id("c")]);

    let mut seen = Vec::new();
    let result: Result<(), &str> = expr.try_transform(&mut |node| {
        seen.push(node.to_string());
        match &node.node {
            DataExprKind::Id(name) if name == "b" => Err("b"),
            _ => Ok(()),
        }
    });

    assert_eq!(result, Err("b"));
    assert_eq!(seen, ["f", "a", "b"]);
}

/// Prints a [MixedNode] with the name of its node type.
fn describe(node: MixedNode<'_>) -> String {
    match node {
        MixedNode::SortExpression(node) => format!("sort {node}"),
        MixedNode::DataExpr(node) => format!("data {node}"),
        MixedNode::ProcessExpr(node) => format!("process {node}"),
        MixedNode::StateFrm(node) => format!("state {node}"),
        MixedNode::RegFrm(node) => format!("reg {node}"),
        MixedNode::ActFrm(node) => format!("act {node}"),
        MixedNode::PbesExpr(node) => format!("pbes {node}"),
        MixedNode::PresExpr(node) => format!("pres {node}"),
    }
}

/// `[!true . false*](! true)`, built without the parser.
fn modality() -> StateFrm {
    let regular: RegFrm = RegFrmKind::Sequence {
        lhs: Box::new(RegFrmKind::Action(ActFrmKind::Negation(Box::new(ActFrmKind::True.into())).into()).into()),
        rhs: Box::new(RegFrmKind::Iteration(Box::new(RegFrmKind::Action(ActFrmKind::False.into()).into())).into()),
    }
    .into();
    StateFrmKind::Modality {
        operator: ModalityOperator::Box,
        formula: regular,
        expr: Box::new(
            StateFrmKind::Unary {
                op: StateFrmUnaryOp::Negation,
                expr: Box::new(StateFrmKind::True.into()),
            }
            .into(),
        ),
    }
    .into()
}

#[test]
fn test_visit_mixed_crosses_into_regular_and_action_formulas_in_preorder() {
    let formula = modality();

    let mut visited = Vec::new();
    let result = formula.visit_mixed::<Infallible>(|node| {
        visited.push(describe(node));
        ControlFlow::Continue(())
    });

    assert_eq!(result, None);
    // The regular formula of a modality comes before its state formula operand.
    assert_eq!(
        visited,
        [
            "state [((!true) . (false)*)](! true)",
            "reg ((!true) . (false)*)",
            "reg (!true)",
            "act (!true)",
            "act true",
            "reg (false)*",
            "reg false",
            "act false",
            "state (! true)",
            "state true",
        ]
    );
}

#[test]
fn test_try_visit_mixed_stops_at_break_and_error() {
    let formula = modality();

    let mut visited = 0;
    let found: Result<Option<String>, Infallible> = formula.try_visit_mixed(|node| {
        visited += 1;
        Ok(match node {
            MixedNode::ActFrm(act) => ControlFlow::Break(act.to_string()),
            _ => ControlFlow::Continue(()),
        })
    });
    assert_eq!(found, Ok(Some("(!true)".to_string())));
    assert_eq!(visited, 4);

    let failed: Result<Option<Infallible>, &str> = formula.try_visit_mixed(|node| match node {
        MixedNode::RegFrm(_) => Err("reg"),
        _ => Ok(ControlFlow::Continue(())),
    });
    assert_eq!(failed, Err("reg"));
}

#[test]
fn test_deep_tree_visit_and_visit_with() {
    let expr = deep_negation(DEPTH);

    let mut count = 0;
    assert_eq!(
        expr.visit::<(), _>(|_| {
            count += 1;
            ControlFlow::Continue(())
        }),
        None
    );
    assert_eq!(count, DEPTH + 1);

    let leaf_depth = expr.visit_with::<usize, usize, Infallible, _>(0, |node, depth| {
        Ok(match &node.node {
            DataExprKind::Id(_) => ControlFlow::Break(depth),
            _ => ControlFlow::Continue(Step::Into(depth + 1)),
        })
    });
    assert_eq!(leaf_depth, Ok(Some(DEPTH)));

    let mut children = 0;
    let outcome: Recursion<Infallible, Infallible> = expr.visit_children((), &mut |_, context| {
        children += 1;
        Ok(ControlFlow::Continue(Step::Into(context)))
    });
    assert!(matches!(outcome, ControlFlow::Continue(())));
    assert_eq!(children, DEPTH);
}

#[test]
fn test_deep_tree_visit_scoped_exits_innermost_first_also_after_break() {
    let expr = deep_negation(DEPTH);

    // Without stopping: every node is exited, deepest first.
    let mut exits = Vec::new();
    let result = expr.visit_scoped::<usize, Vec<usize>, (), Infallible, _, _>(
        0,
        &mut exits,
        |_, depth, _| Ok(ControlFlow::Continue(Step::Into(depth + 1))),
        |_, depth, exits| exits.push(depth),
    );
    assert_eq!(result, Ok(None));
    assert!(exits.iter().copied().eq((0..=DEPTH).rev()));

    // Stopping at the leaf: every enclosing node is still exited, deepest first.
    let mut exits = Vec::new();
    let result = expr.visit_scoped::<usize, Vec<usize>, (), Infallible, _, _>(
        0,
        &mut exits,
        |node, depth, _| {
            Ok(match &node.node {
                DataExprKind::Id(_) => ControlFlow::Break(()),
                _ => ControlFlow::Continue(Step::Into(depth + 1)),
            })
        },
        |_, depth, exits| exits.push(depth),
    );
    assert_eq!(result, Ok(Some(())));
    assert!(exits.iter().copied().eq((0..DEPTH).rev()));
}

#[test]
fn test_deep_tree_apply_and_apply_scoped() {
    let mut expr = deep_negation(DEPTH);

    expr.apply_mut::<Infallible, _>(|node| {
        Ok(match &node.node {
            DataExprKind::Id(_) => Some(id("y")),
            _ => None,
        })
    })
    .expect("the callback cannot fail");

    let mut closed = 0;
    let result = expr.apply_scoped::<(), usize, (), Infallible, _, _>(
        (),
        &mut closed,
        |node, (), _| {
            if let DataExprKind::Id(name) = &mut node.node {
                name.push('!');
            }
            Ok(ControlFlow::Continue(Step::Into(())))
        },
        |(), closed| *closed += 1,
    );
    assert_eq!(result, Ok(None));
    assert_eq!(closed, DEPTH + 1);

    let leaf = expr.visit(|node| match &node.node {
        DataExprKind::Id(name) => ControlFlow::Break(name.clone()),
        _ => ControlFlow::Continue(()),
    });
    assert_eq!(leaf.as_deref(), Some("y!"));
}

#[test]
fn test_deep_tree_transform_is_bottom_up() {
    let mut expr = deep_negation(DEPTH);

    let mut ids = 0;
    let mut first = None;
    expr.transform(|node| {
        if first.is_none() {
            first = Some(matches!(node.node, DataExprKind::Id(_)));
        }
        if let DataExprKind::Id(name) = &mut node.node {
            ids += 1;
            *name = "y".to_string();
        }
    });
    assert_eq!(first, Some(true), "the leaf is transformed first");
    assert_eq!(ids, 1);

    let mut children = 0;
    let result: Result<(), Infallible> = expr.transform_children(&mut |_| {
        children += 1;
        Ok(())
    });
    assert_eq!(result, Ok(()));
    assert_eq!(children, DEPTH);

    let leaf = expr.visit(|node| match &node.node {
        DataExprKind::Id(name) => ControlFlow::Break(name.clone()),
        _ => ControlFlow::Continue(()),
    });
    assert_eq!(leaf.as_deref(), Some("y"));
}

/// Builds a chain of `depth` nodes on top of `leaf`, visits and transforms it, and drops it.
/// `other_type_nodes` counts the nodes of another type that only [Traverse::visit_mixed] reaches.
#[track_caller]
fn check_deep<N: Traverse + Default>(leaf: N, wrap: impl Fn(N) -> N, depth: usize, other_type_nodes: usize) {
    let mut node = leaf;
    for _ in 0..depth {
        node = wrap(node);
    }

    let mut count = 0;
    node.visit::<(), _>(|_| {
        count += 1;
        ControlFlow::Continue(())
    });
    assert_eq!(count, depth + 1, "visit");

    let mut count = 0;
    node.transform(|_| count += 1);
    assert_eq!(count, depth + 1, "transform");

    let mut count = 0;
    let result = node.visit_mixed::<Infallible>(|_| {
        count += 1;
        ControlFlow::Continue(())
    });
    assert!(result.is_none());
    assert_eq!(count, depth + 1 + other_type_nodes, "visit_mixed");
}

#[test]
fn test_deep_trees_of_every_node_type() {
    check_deep(
        SortExpression::from(SortExpressionKind::Reference("S".to_string())),
        |sort| SortExpressionKind::Complex(ComplexSort::List, Box::new(sort)).into(),
        DEPTH,
        0,
    );
    check_deep(id("x"), not, DEPTH, 0);
    check_deep(
        ProcessExpr::from(ProcessExprKind::Tau),
        |process| {
            ProcessExprKind::Sum {
                variables: Vec::new(),
                operand: Box::new(process),
            }
            .into()
        },
        DEPTH,
        0,
    );
    check_deep(
        StateFrm::from(StateFrmKind::True),
        |formula| {
            StateFrmKind::Unary {
                op: StateFrmUnaryOp::Negation,
                expr: Box::new(formula),
            }
            .into()
        },
        DEPTH,
        0,
    );
    check_deep(
        RegFrm::from(RegFrmKind::Action(ActFrmKind::True.into())),
        |formula| RegFrmKind::Iteration(Box::new(formula)).into(),
        DEPTH,
        1,
    );
    check_deep(
        ActFrm::from(ActFrmKind::True),
        |formula| ActFrmKind::Negation(Box::new(formula)).into(),
        DEPTH,
        0,
    );
    check_deep(
        PbesExpr::from(PbesExprKind::True),
        |expr| PbesExprKind::Negation(Box::new(expr)).into(),
        DEPTH,
        0,
    );
    check_deep(
        PresExpr::from(PresExprKind::True),
        |expr| PresExprKind::Negation(Box::new(expr)).into(),
        DEPTH,
        0,
    );
}

#[test]
fn test_deep_modalities_are_crossed_by_visit_mixed() {
    let mut formula = StateFrm::from(StateFrmKind::True);
    for _ in 0..DEPTH {
        formula = StateFrmKind::Modality {
            operator: ModalityOperator::Diamond,
            formula: RegFrmKind::Action(ActFrmKind::True.into()).into(),
            expr: Box::new(formula),
        }
        .into();
    }

    let mut actions = 0;
    formula.visit_mixed::<Infallible>(|node| {
        if let MixedNode::ActFrm(_) = node {
            actions += 1;
        }
        ControlFlow::Continue(())
    });
    assert_eq!(actions, DEPTH);
}

#[test]
fn test_wide_tree_children_are_visited_in_order() {
    let names: Vec<String> = (0..WIDTH).map(|i| format!("x{i}")).collect();
    let expr = app("f", names.iter().map(|name| id(name)).collect());

    let mut visited = Vec::new();
    expr.visit::<(), _>(|node| {
        if let DataExprKind::Id(name) = &node.node {
            visited.push(name.clone());
        }
        ControlFlow::Continue(())
    });
    assert_eq!(visited[0], "f");
    assert_eq!(visited[1..], names[..]);

    let mut transformed = Vec::new();
    let mut expr = expr;
    expr.transform(|node| {
        if let DataExprKind::Id(name) = &mut node.node {
            transformed.push(name.clone());
            name.push('\'');
        }
    });
    assert_eq!(transformed, visited);

    let DataExprKind::Application { arguments, .. } = &expr.node else {
        panic!("the root is still an application");
    };
    assert!(
        arguments
            .iter()
            .zip(&names)
            .all(|(argument, name)| argument.to_string() == format!("{name}'"))
    );
}

#[test]
fn test_repeated_subtrees_are_each_visited_and_rewritten() {
    // Syntax trees own their children, so equal subtrees are distinct occurrences.
    let shared = and(id("a"), not(id("a")));
    let expr = app("f", vec![shared.clone(), shared.clone(), shared]);

    let mut occurrences = 0;
    expr.visit::<(), _>(|node| {
        if matches!(&node.node, DataExprKind::Id(name) if name == "a") {
            occurrences += 1;
        }
        ControlFlow::Continue(())
    });
    assert_eq!(occurrences, 6);

    let result = expr
        .apply::<Infallible, _>(|node| {
            Ok(match &node.node {
                DataExprKind::Unary { .. } => Some(id("n")),
                _ => None,
            })
        })
        .expect("the callback cannot fail");
    assert_eq!(result.to_string(), "f((a && n), (a && n), (a && n))");
}

/// Parses a state formula, for example `mu X. [a]X`.
fn state_formula(input: &str) -> StateFrm {
    UntypedStateFrmSpec::parse(input)
        .expect("the state formula should parse")
        .formula
}

/// Parses a regular formula by putting it inside a modality, for example `a . b*`.
fn regular_formula(input: &str) -> RegFrm {
    match state_formula(&format!("[{input}]true")).into_node() {
        StateFrmKind::Modality { formula, .. } => formula,
        other => panic!("expected a modality, got {other:?}"),
    }
}

/// Parses an action formula, for example `a && b`.
fn action_formula(input: &str) -> ActFrm {
    match regular_formula(input).into_node() {
        RegFrmKind::Action(act_frm) => act_frm,
        other => panic!("expected an action formula, got {other:?}"),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_orders_on_a_parsed_sort_expression() {
    let sort = UntypedDataSpecification::parse("sort S = A # List(B) -> struct c(x: C) | d;")
        .expect("the sort expression should parse")
        .sort_declarations
        .remove(0)
        .expr
        .expect("the declaration is an alias");

    check_orders(
        &sort,
        &[
            "((A # List(B)) -> struct c(x : C) | d)",
            "(A # List(B))",
            "A",
            "List(B)",
            "B",
            "struct c(x : C) | d",
            "C",
        ],
        &[
            "A",
            "B",
            "List(B)",
            "(A # List(B))",
            "C",
            "struct c(x : C) | d",
            "((A # List(B)) -> struct c(x : C) | d)",
        ],
    );
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_orders_on_a_parsed_data_expression() {
    let expr = DataExpr::parse(
        "f([a], {b}, {c: 1}, { x: Nat | p }, lambda y: Nat . -q, g[h -> i], j whr k = l end, forall z: Nat . m)",
    )
    .expect("the data expression should parse");

    check_orders(
        &expr,
        &[
            "f([a], { b }, { c: 1 }, { x: Nat | p }, (lambda y: Nat . (- q)), g[h -> i], j whr k = l end, (forall z: Nat . m))",
            "f",
            "[a]",
            "a",
            "{ b }",
            "b",
            "{ c: 1 }",
            "c",
            "1",
            "{ x: Nat | p }",
            "p",
            "(lambda y: Nat . (- q))",
            "(- q)",
            "q",
            "g[h -> i]",
            "g",
            "h",
            "i",
            "j whr k = l end",
            "j",
            "l",
            "(forall z: Nat . m)",
            "m",
        ],
        &[
            "f",
            "a",
            "[a]",
            "b",
            "{ b }",
            "c",
            "1",
            "{ c: 1 }",
            "p",
            "{ x: Nat | p }",
            "q",
            "(- q)",
            "(lambda y: Nat . (- q))",
            "g",
            "h",
            "i",
            "g[h -> i]",
            "j",
            "l",
            "j whr k = l end",
            "m",
            "(forall z: Nat . m)",
            "f([a], { b }, { c: 1 }, { x: Nat | p }, (lambda y: Nat . (- q)), g[h -> i], j whr k = l end, (forall z: Nat . m))",
        ],
    );
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_orders_on_a_parsed_process_expression() {
    let spec = UntypedProcessSpecification::parse(
        "act a, b, c; init sum n: Nat . (b -> a <> c) . hide({a}, allow({b}, block({c}, rename({a -> b}, comm({a | b -> c}, a @ 1))))) || dist x: Bool[1/2] . tau + delta;",
    )
    .expect("the process specification should parse");

    check_orders(
        &spec.init.expect("the specification has an init"),
        &[
            "((sum n: Nat . ((((b) -> (a) <> (c)) . hide({a}, allow({b}, block({c}, rename({a -> b}, comm({a|b -> c}, (a)@(1))))))) || (dist x: Bool [(1 / 2)] . tau))) + delta)",
            "(sum n: Nat . ((((b) -> (a) <> (c)) . hide({a}, allow({b}, block({c}, rename({a -> b}, comm({a|b -> c}, (a)@(1))))))) || (dist x: Bool [(1 / 2)] . tau)))",
            "((((b) -> (a) <> (c)) . hide({a}, allow({b}, block({c}, rename({a -> b}, comm({a|b -> c}, (a)@(1))))))) || (dist x: Bool [(1 / 2)] . tau))",
            "(((b) -> (a) <> (c)) . hide({a}, allow({b}, block({c}, rename({a -> b}, comm({a|b -> c}, (a)@(1)))))))",
            "((b) -> (a) <> (c))",
            "a",
            "c",
            "hide({a}, allow({b}, block({c}, rename({a -> b}, comm({a|b -> c}, (a)@(1))))))",
            "allow({b}, block({c}, rename({a -> b}, comm({a|b -> c}, (a)@(1)))))",
            "block({c}, rename({a -> b}, comm({a|b -> c}, (a)@(1))))",
            "rename({a -> b}, comm({a|b -> c}, (a)@(1)))",
            "comm({a|b -> c}, (a)@(1))",
            "(a)@(1)",
            "a",
            "(dist x: Bool [(1 / 2)] . tau)",
            "tau",
            "delta",
        ],
        &[
            "a",
            "c",
            "((b) -> (a) <> (c))",
            "a",
            "(a)@(1)",
            "comm({a|b -> c}, (a)@(1))",
            "rename({a -> b}, comm({a|b -> c}, (a)@(1)))",
            "block({c}, rename({a -> b}, comm({a|b -> c}, (a)@(1))))",
            "allow({b}, block({c}, rename({a -> b}, comm({a|b -> c}, (a)@(1)))))",
            "hide({a}, allow({b}, block({c}, rename({a -> b}, comm({a|b -> c}, (a)@(1))))))",
            "(((b) -> (a) <> (c)) . hide({a}, allow({b}, block({c}, rename({a -> b}, comm({a|b -> c}, (a)@(1)))))))",
            "tau",
            "(dist x: Bool [(1 / 2)] . tau)",
            "((((b) -> (a) <> (c)) . hide({a}, allow({b}, block({c}, rename({a -> b}, comm({a|b -> c}, (a)@(1))))))) || (dist x: Bool [(1 / 2)] . tau))",
            "(sum n: Nat . ((((b) -> (a) <> (c)) . hide({a}, allow({b}, block({c}, rename({a -> b}, comm({a|b -> c}, (a)@(1))))))) || (dist x: Bool [(1 / 2)] . tau)))",
            "delta",
            "((sum n: Nat . ((((b) -> (a) <> (c)) . hide({a}, allow({b}, block({c}, rename({a -> b}, comm({a|b -> c}, (a)@(1))))))) || (dist x: Bool [(1 / 2)] . tau))) + delta)",
        ],
    );
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_orders_on_a_parsed_state_formula() {
    let formula = state_formula("mu X. [a]X && (forall n: Nat . !delay => sup m: Nat . val(m) * X)");

    check_orders(
        &formula,
        &[
            "(mu X . ([a]X && (forall n: Nat . ((! delay) => (sup m: Nat . (val(m) * X))))))",
            "([a]X && (forall n: Nat . ((! delay) => (sup m: Nat . (val(m) * X)))))",
            "[a]X",
            "X",
            "(forall n: Nat . ((! delay) => (sup m: Nat . (val(m) * X))))",
            "((! delay) => (sup m: Nat . (val(m) * X)))",
            "(! delay)",
            "delay",
            "(sup m: Nat . (val(m) * X))",
            "(val(m) * X)",
            "X",
        ],
        &[
            "X",
            "[a]X",
            "delay",
            "(! delay)",
            "X",
            "(val(m) * X)",
            "(sup m: Nat . (val(m) * X))",
            "((! delay) => (sup m: Nat . (val(m) * X)))",
            "(forall n: Nat . ((! delay) => (sup m: Nat . (val(m) * X))))",
            "([a]X && (forall n: Nat . ((! delay) => (sup m: Nat . (val(m) * X)))))",
            "(mu X . ([a]X && (forall n: Nat . ((! delay) => (sup m: Nat . (val(m) * X))))))",
        ],
    );
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_orders_on_a_parsed_regular_formula() {
    check_orders(
        &regular_formula("a . (b* + c+)"),
        &["(a . ((b)* + (c)+))", "a", "((b)* + (c)+)", "(b)*", "b", "(c)+", "c"],
        &["a", "b", "(b)*", "c", "(c)+", "((b)* + (c)+)", "(a . ((b)* + (c)+))"],
    );
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_orders_on_a_parsed_action_formula() {
    check_orders(
        &action_formula("a && (!b || exists n: Nat . c(n) @ 1) => true"),
        &[
            "((a && ((!b) || (exists n: Nat . (c(n))@(1)))) => true)",
            "(a && ((!b) || (exists n: Nat . (c(n))@(1))))",
            "a",
            "((!b) || (exists n: Nat . (c(n))@(1)))",
            "(!b)",
            "b",
            "(exists n: Nat . (c(n))@(1))",
            "(c(n))@(1)",
            "c(n)",
            "true",
        ],
        &[
            "a",
            "b",
            "(!b)",
            "c(n)",
            "(c(n))@(1)",
            "(exists n: Nat . (c(n))@(1))",
            "((!b) || (exists n: Nat . (c(n))@(1)))",
            "(a && ((!b) || (exists n: Nat . (c(n))@(1))))",
            "true",
            "((a && ((!b) || (exists n: Nat . (c(n))@(1)))) => true)",
        ],
    );
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_orders_on_a_parsed_pbes_expression() {
    let pbes = UntypedPbes::parse("pbes mu X = forall n: Nat . (val(n < 3) => !X) && true; init X;")
        .expect("the PBES should parse");

    check_orders(
        &pbes.equations[0].formula,
        &[
            "(forall n: Nat . ((val((n < 3)) => (! X)) && true))",
            "((val((n < 3)) => (! X)) && true)",
            "(val((n < 3)) => (! X))",
            "val((n < 3))",
            "(! X)",
            "X",
            "true",
        ],
        &[
            "val((n < 3))",
            "X",
            "(! X)",
            "(val((n < 3)) => (! X))",
            "true",
            "((val((n < 3)) => (! X)) && true)",
            "(forall n: Nat . ((val((n < 3)) => (! X)) && true))",
        ],
    );
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_orders_on_a_parsed_pres_expression() {
    let pres =
        UntypedPres::parse("pres mu X = sup n: Nat . (val(2) * X + condsm(X, -X, eqinf(X * val(3)))) && true; init X;")
            .expect("the PRES should parse");

    check_orders(
        &pres.equations[0].formula,
        &[
            "(sup n: Nat . (((val(2) * X) + condsm(X, (- X), eqinf((X * val(3))))) && true))",
            "(((val(2) * X) + condsm(X, (- X), eqinf((X * val(3))))) && true)",
            "((val(2) * X) + condsm(X, (- X), eqinf((X * val(3)))))",
            "(val(2) * X)",
            "X",
            "condsm(X, (- X), eqinf((X * val(3))))",
            "X",
            "(- X)",
            "X",
            "eqinf((X * val(3)))",
            "(X * val(3))",
            "X",
            "true",
        ],
        &[
            "X",
            "(val(2) * X)",
            "X",
            "X",
            "(- X)",
            "X",
            "(X * val(3))",
            "eqinf((X * val(3)))",
            "condsm(X, (- X), eqinf((X * val(3))))",
            "((val(2) * X) + condsm(X, (- X), eqinf((X * val(3)))))",
            "true",
            "(((val(2) * X) + condsm(X, (- X), eqinf((X * val(3))))) && true)",
            "(sup n: Nat . (((val(2) * X) + condsm(X, (- X), eqinf((X * val(3))))) && true))",
        ],
    );
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_visit_mixed_on_a_parsed_modality() {
    let formula = state_formula("[a . b*]X");

    let mut visited = Vec::new();
    formula.visit_mixed::<Infallible>(|node| {
        visited.push(describe(node));
        ControlFlow::Continue(())
    });

    // The modality's own operand `X` is only reached after its whole regular formula.
    assert_eq!(
        visited,
        [
            "state [(a . (b)*)]X",
            "reg (a . (b)*)",
            "reg a",
            "act a",
            "reg (b)*",
            "reg b",
            "act b",
            "state X",
        ]
    );
}

/// A realistic use of the scoped walk: a fixpoint variable is in scope in the body of its
/// fixpoint only, so an inner fixpoint's variable is invisible to a sibling formula.
#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_visit_scoped_tracks_the_fixpoint_variables_in_scope() {
    let formula = state_formula("mu X. (mu Y. X) && Z");

    // The scoped state may borrow from the tree being walked.
    let mut state_vars: Vec<&str> = Vec::new();
    let mut scope_at_each_id: Vec<(&str, Vec<&str>)> = Vec::new();

    let result = formula.visit_scoped::<(), Vec<&str>, Infallible, Infallible, _, _>(
        (),
        &mut state_vars,
        |formula, context, state_vars| {
            if let StateFrmKind::FixedPoint { variable, .. } = &formula.node {
                state_vars.push(variable.identifier.node.as_str());
            }
            if let StateFrmKind::Id(name, _) = &formula.node {
                scope_at_each_id.push((name.node.as_str(), state_vars.clone()));
            }
            Ok(ControlFlow::Continue(Step::Into(context)))
        },
        |formula, _context, state_vars| {
            if let StateFrmKind::FixedPoint { .. } = &formula.node {
                state_vars.pop();
            }
        },
    );

    assert_eq!(result, Ok(None));
    assert_eq!(scope_at_each_id, [("X", vec!["X", "Y"]), ("Z", vec!["X"])]);
    assert!(state_vars.is_empty());
}
