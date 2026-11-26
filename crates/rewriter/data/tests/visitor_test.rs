use std::convert::Infallible;
use std::ops::ControlFlow;

use ahash::AHashSet;
use merc_data::BinderType;
use merc_data::DataAbstraction;
use merc_data::DataAbstractionRef;
use merc_data::DataApplication;
use merc_data::DataApplicationRef;
use merc_data::DataExpression;
use merc_data::DataExpressionKind;
use merc_data::DataExpressionRef;
use merc_data::DataExpressionVisitor;
use merc_data::DataFunctionSymbol;
use merc_data::DataFunctionSymbolRef;
use merc_data::DataVariable;
use merc_data::DataVariableRef;
use merc_data::DataWhereClause;
use merc_data::DataWhereClauseRef;
use merc_data::DataWhrDecl;
use merc_data::MachineNumber;
use merc_data::MachineNumberRef;
use merc_data::SortExpression;
use merc_data::try_visit_data_expr;
use merc_data::visit_data_expr;
use merc_utilities::Step;
use merc_utilities::Visit;

/// A short label for a node: the name of a variable or function symbol, the head symbol of an
/// application, the value of a machine number, or the kind of a binder or where clause.
fn label(expr: &DataExpressionRef<'_>) -> String {
    match expr.kind() {
        DataExpressionKind::Variable(variable) => variable.name().to_string(),
        DataExpressionKind::FunctionSymbol(symbol) => symbol.name().value().to_string(),
        DataExpressionKind::MachineNumber(number) => number.to_string(),
        DataExpressionKind::Application(application) => application.data_function_symbol().name().value().to_string(),
        DataExpressionKind::Abstraction(_) => "binder".to_string(),
        DataExpressionKind::WhereClause(_) => "whr".to_string(),
    }
}

fn apply(head: &str, arguments: &[DataExpression]) -> DataExpression {
    DataApplication::with_args(&DataFunctionSymbol::new(head), arguments).into()
}

/// `f(t, t)` with the shared subterm `t = g(x)`.
fn shared_term() -> DataExpression {
    let t = apply("g", &[DataVariable::new("x").into()]);
    apply("f", &[t.clone(), t])
}

/// `e_n` with `e_0 = a` and `e_{i+1} = f(e_i, e_i)`: `n + 1` distinct subterms that unfold to a
/// tree of `2^(n+1) - 1` nodes.
fn exponential_dag(n: usize) -> DataExpression {
    let mut term: DataExpression = DataFunctionSymbol::new("a").into();
    for _ in 0..n {
        term = apply("f", &[term.clone(), term]);
    }
    term
}

/// `f(f(...f(x)...))` with `depth` applications.
fn deep_chain(depth: usize) -> DataExpression {
    let mut term: DataExpression = DataVariable::new("x").into();
    for _ in 0..depth {
        term = apply("f", &[term]);
    }
    term
}

/// Deep enough to overflow the stack of a recursive traversal, but small enough for miri.
const DEPTH: usize = if cfg!(miri) { 100 } else { 100_000 };

/// The nodes in the order a top-down, left-to-right traversal must visit them, computed by a
/// recursive reference model on the term structure.
fn preorder(expr: &DataExpressionRef<'_>, out: &mut Vec<String>) {
    out.push(label(expr));
    match expr.kind() {
        DataExpressionKind::Application(_) => {
            for argument in expr.data_arguments() {
                preorder(&argument, out);
            }
        }
        DataExpressionKind::Abstraction(binder) => preorder(&binder.body(), out),
        DataExpressionKind::WhereClause(clause) => {
            for declaration in clause.declarations().iter() {
                preorder(&declaration.expression(), out);
            }
            preorder(&clause.body(), out);
        }
        _ => {}
    }
}

fn visit_all(expr: &DataExpression) -> Vec<String> {
    let mut visited = Vec::new();
    let result: Option<Infallible> = visit_data_expr(&expr.copy(), (), |expr, context| {
        visited.push(label(expr));
        ControlFlow::Continue(Step::Into(context))
    });
    assert!(result.is_none());
    visited
}

/// Records which method was called on which node, so that a test can check that every node kind
/// is dispatched to its own method.
#[derive(Default)]
struct Recorder {
    calls: Vec<String>,
}

impl<'a> DataExpressionVisitor<'a> for Recorder {
    type Context = ();
    type Break = Infallible;
    type Error = Infallible;

    fn visit_variable(
        &mut self,
        variable: &DataVariableRef<'a>,
        context: (),
    ) -> Visit<Infallible, (), Infallible, Infallible> {
        self.calls.push(format!("variable {}", variable.name()));
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    fn visit_function_symbol(
        &mut self,
        function_symbol: &DataFunctionSymbolRef<'a>,
        context: (),
    ) -> Visit<Infallible, (), Infallible, Infallible> {
        self.calls.push(format!("symbol {}", function_symbol.name().value()));
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    fn visit_machine_number(
        &mut self,
        number: &MachineNumberRef<'a>,
        context: (),
    ) -> Visit<Infallible, (), Infallible, Infallible> {
        self.calls.push(format!("number {number}"));
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    fn visit_application(
        &mut self,
        application: &DataApplicationRef<'a>,
        context: (),
    ) -> Visit<Infallible, (), Infallible, Infallible> {
        self.calls.push(format!(
            "application {}",
            application.data_function_symbol().name().value()
        ));
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    fn visit_binder(
        &mut self,
        _binder: &DataAbstractionRef<'a>,
        context: (),
    ) -> Visit<Infallible, (), Infallible, Infallible> {
        self.calls.push("binder".to_string());
        Ok(ControlFlow::Continue(Step::Into(context)))
    }

    fn visit_where_clause(
        &mut self,
        _clause: &DataWhereClauseRef<'a>,
        context: (),
    ) -> Visit<Infallible, (), Infallible, Infallible> {
        self.calls.push("whr".to_string());
        Ok(ControlFlow::Continue(Step::Into(context)))
    }
}

/// `(lambda x. f(x, y)) whr y = b, z = 7`: a where clause with two declarations whose body is a
/// binder.
fn binder_in_where_clause() -> DataExpression {
    let x = DataVariable::with_sort("x", SortExpression::bool().copy());
    let y = DataVariable::with_sort("y", SortExpression::bool().copy());
    let z = DataVariable::with_sort("z", SortExpression::bool().copy());
    let body = apply("f", &[x.clone().into(), y.clone().into()]);
    let lambda: DataExpression = DataAbstraction::new(BinderType::Lambda, std::slice::from_ref(&x), body).into();
    DataWhereClause::new(
        lambda,
        &[
            DataWhrDecl::new(y, DataFunctionSymbol::new("b").into()),
            DataWhrDecl::new(z, MachineNumber::new(7).into()),
        ],
    )
    .into()
}

#[test]
fn test_visit_dispatches_every_kind_top_down() {
    let term = binder_in_where_clause();

    let mut recorder = Recorder::default();
    assert!(recorder.visit(&term.copy(), ()).is_none());

    // The declaration values come before the body, and the variables bound by the where clause
    // (`y`, `z`) and the binder (`x`) are only visited where they occur in a body.
    assert_eq!(
        recorder.calls,
        [
            "whr",
            "symbol b",
            "number 7",
            "binder",
            "application f",
            "variable x",
            "variable y"
        ]
    );
}

#[test]
fn test_visit_order_matches_a_recursive_model() {
    let terms = [
        DataExpression::from_string("f(g(a), b, h(c, d(e)))").unwrap(),
        shared_term(),
        exponential_dag(4),
        binder_in_where_clause(),
        apply("f", &[binder_in_where_clause(), shared_term()]),
    ];

    for term in &terms {
        let mut expected = Vec::new();
        preorder(&term.copy(), &mut expected);
        assert_eq!(visit_all(term), expected, "for {term}");
    }
}

/// The visitor does not deduplicate shared subterms by itself: `t` is visited once per path.
#[test]
fn test_visit_walks_a_shared_subterm_once_per_path() {
    assert_eq!(visit_all(&shared_term()), ["f", "g", "x", "g", "x"]);
}

#[test]
fn test_visit_breaks_at_the_first_match_on_a_shared_term() {
    let term = shared_term();

    let mut visited = Vec::new();
    let found = visit_data_expr(&term.copy(), (), |expr, context| {
        visited.push(label(expr));
        if let DataExpressionKind::Variable(variable) = expr.kind() {
            ControlFlow::Break(variable.protect())
        } else {
            ControlFlow::Continue(Step::Into(context))
        }
    });

    assert_eq!(found, Some(DataVariable::new("x")));
    // The second occurrence of `g(x)` is never reached.
    assert_eq!(visited, ["f", "g", "x"]);
}

#[test]
fn test_visit_breaks_at_a_given_depth_of_a_deep_chain() {
    let term = deep_chain(DEPTH);

    for stop in [0, 1, DEPTH / 2, DEPTH] {
        let mut calls = 0;
        let found = visit_data_expr(&term.copy(), 0usize, |_expr, depth| {
            calls += 1;
            if depth == stop {
                ControlFlow::Break(depth)
            } else {
                ControlFlow::Continue(Step::Into(depth + 1))
            }
        });

        assert_eq!(found, Some(stop));
        assert_eq!(calls, stop + 1);
    }
}

#[test]
fn test_visit_reaches_the_leaf_of_a_deep_chain_with_the_threaded_depth() {
    let term = deep_chain(DEPTH);

    let mut calls = 0;
    let leaf = visit_data_expr(&term.copy(), 0usize, |expr, depth| {
        calls += 1;
        if let DataExpressionKind::Variable(variable) = expr.kind() {
            ControlFlow::Break((variable.name().to_string(), depth))
        } else {
            ControlFlow::Continue(Step::Into(depth + 1))
        }
    });

    assert_eq!(leaf, Some(("x".to_string(), DEPTH)));
    assert_eq!(calls, DEPTH + 1);
}

#[test]
fn test_visit_prune_skips_the_children_on_every_path() {
    let term = shared_term();

    let mut visited = Vec::new();
    let result: Option<Infallible> = visit_data_expr(&term.copy(), (), |expr, context| {
        visited.push(label(expr));
        if label(expr) == "g" {
            ControlFlow::Continue(Step::Prune)
        } else {
            ControlFlow::Continue(Step::Into(context))
        }
    });

    assert!(result.is_none());
    assert_eq!(visited, ["f", "g", "g"]);
}

/// Pruning the subterms seen before, as the documentation recommends, makes the traversal linear
/// in the term graph: the callback runs once per edge plus once for the root.
#[test]
fn test_visit_pruning_seen_subterms_is_linear_on_an_exponential_dag() {
    const N: usize = 60;
    let term = exponential_dag(N);

    let mut seen = AHashSet::new();
    let mut calls = 0;
    let result: Option<Infallible> = visit_data_expr(&term.copy(), (), |expr, context| {
        calls += 1;
        if seen.insert(expr.copy()) {
            ControlFlow::Continue(Step::Into(context))
        } else {
            ControlFlow::Continue(Step::Prune)
        }
    });

    assert!(result.is_none());
    assert_eq!(seen.len(), N + 1);
    assert_eq!(calls, 2 * N + 1);
}

#[test]
fn test_visit_prunes_binder_and_where_clause_bodies() {
    let term = apply("f", &[binder_in_where_clause(), binder_in_where_clause()]);

    // Pruning the where clause skips its declarations and its body.
    let mut visited = Vec::new();
    let _: Option<Infallible> = visit_data_expr(&term.copy(), (), |expr, context| {
        visited.push(label(expr));
        match expr.kind() {
            DataExpressionKind::WhereClause(_) => ControlFlow::Continue(Step::Prune),
            _ => ControlFlow::Continue(Step::Into(context)),
        }
    });
    assert_eq!(visited, ["f", "whr", "whr"]);

    // Pruning the binder skips its body, but not the declarations of the enclosing where clause.
    let mut visited = Vec::new();
    let _: Option<Infallible> = visit_data_expr(&term.copy(), (), |expr, context| {
        visited.push(label(expr));
        match expr.kind() {
            DataExpressionKind::Abstraction(_) => ControlFlow::Continue(Step::Prune),
            _ => ControlFlow::Continue(Step::Into(context)),
        }
    });
    assert_eq!(visited, ["f", "whr", "b", "7", "binder", "whr", "b", "7", "binder"]);
}

#[test]
fn test_visit_threads_the_context_per_path_on_a_shared_term() {
    let term = apply("h", &[shared_term(), DataFunctionSymbol::new("b").into()]);

    let mut depths = Vec::new();
    let result: Option<Infallible> = visit_data_expr(&term.copy(), 0, |expr, depth| {
        depths.push((label(expr), depth));
        ControlFlow::Continue(Step::Into(depth + 1))
    });

    assert!(result.is_none());
    let expected = [("h", 0), ("f", 1), ("g", 2), ("x", 3), ("g", 2), ("x", 3), ("b", 1)];
    assert_eq!(depths, expected.map(|(label, depth)| (label.to_string(), depth)));
}

/// The context given to `Step::Into` at a binder or a where clause reaches every declaration and
/// the body, so a visitor can track the variables in scope without a stack of its own.
#[test]
fn test_visit_threads_the_context_into_binders_and_where_clauses() {
    let term = binder_in_where_clause();

    // The context counts the binders and where clauses above a node.
    let mut scopes = Vec::new();
    let result: Option<Infallible> = visit_data_expr(&term.copy(), 0, |expr, scope| {
        scopes.push((label(expr), scope));
        match expr.kind() {
            DataExpressionKind::Abstraction(_) | DataExpressionKind::WhereClause(_) => {
                ControlFlow::Continue(Step::Into(scope + 1))
            }
            _ => ControlFlow::Continue(Step::Into(scope)),
        }
    });

    assert!(result.is_none());
    let expected = [
        ("whr", 0),
        ("b", 1),
        ("7", 1),
        ("binder", 1),
        ("f", 2),
        ("x", 2),
        ("y", 2),
    ];
    assert_eq!(scopes, expected.map(|(label, scope)| (label.to_string(), scope)));
}

/// A visitor that stops at the first binder or where clause, through their own methods.
struct FirstScope;

impl<'a> DataExpressionVisitor<'a> for FirstScope {
    type Context = ();
    type Break = &'static str;
    type Error = Infallible;

    fn visit_binder(
        &mut self,
        _binder: &DataAbstractionRef<'a>,
        _context: (),
    ) -> Visit<Infallible, (), &'static str, Infallible> {
        Ok(ControlFlow::Break("binder"))
    }

    fn visit_where_clause(
        &mut self,
        _clause: &DataWhereClauseRef<'a>,
        _context: (),
    ) -> Visit<Infallible, (), &'static str, Infallible> {
        Ok(ControlFlow::Break("whr"))
    }
}

#[test]
fn test_visit_breaks_from_binder_and_where_clause_methods() {
    let term = binder_in_where_clause();
    assert_eq!(FirstScope.visit(&term.copy(), ()), Some("whr"));

    let exists: DataExpression = DataAbstraction::new(
        BinderType::Exists,
        &[DataVariable::new("x")],
        DataVariable::new("x").into(),
    )
    .into();
    assert_eq!(FirstScope.visit(&exists.copy(), ()), Some("binder"));

    assert_eq!(FirstScope.visit(&shared_term().copy(), ()), None);
}

#[test]
fn test_try_visit_stops_at_the_first_error() {
    let term = shared_term();

    let mut visited = Vec::new();
    let result: Result<Option<()>, String> = try_visit_data_expr(&term.copy(), (), |expr, context| {
        visited.push(label(expr));
        if label(expr) == "x" {
            Err("found x".to_string())
        } else {
            Ok(ControlFlow::Continue(Step::Into(context)))
        }
    });

    assert_eq!(result, Err("found x".to_string()));
    assert_eq!(visited, ["f", "g", "x"]);
}
