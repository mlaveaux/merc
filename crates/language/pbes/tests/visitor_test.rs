use std::collections::HashMap;
use std::convert::Infallible;
use std::ops::ControlFlow;

use merc_utilities::Step;
use merc_utilities::Visit;

use merc_pbes::PbesExpression;
use merc_pbes::PbesExpressionRef;
use merc_pbes::PbesExpressionVisitor;
use merc_pbes::PbesFold;
use merc_pbes::PropositionalVariableInstantiation;
use merc_pbes::PropositionalVariableInstantiationRef;
use merc_pbes::fold_pbes_expr;
use merc_pbes::visit_pbes_expr;

fn instantiation(name: &str) -> PbesExpression {
    PropositionalVariableInstantiation::new(name, &[]).into()
}

/// `(X && Y) || !Z`
fn example() -> PbesExpression {
    PbesExpression::or(
        &PbesExpression::and(&instantiation("X"), &instantiation("Y")),
        &PbesExpression::not(&instantiation("Z")),
    )
}

/// Collects the name of every instantiation in the order in which it is visited.
#[derive(Default)]
struct InstantiationNames {
    names: Vec<String>,
}

impl PbesExpressionVisitor for InstantiationNames {
    type Context = ();
    type Break = Infallible;
    type Error = Infallible;

    fn visit_instantiation(
        &mut self,
        instantiation: &PropositionalVariableInstantiationRef<'_>,
        context: (),
    ) -> Visit<Infallible, (), Infallible, Infallible> {
        self.names.push(instantiation.name().value().to_string());
        Ok(ControlFlow::Continue(Step::Into(context)))
    }
}

#[test]
fn test_visit_order_is_left_to_right() {
    let mut visitor = InstantiationNames::default();
    assert!(visitor.visit(&example().copy(), ()).is_none());
    assert_eq!(visitor.names, ["X", "Y", "Z"]);
}

#[test]
fn test_visit_breaks_early() {
    let mut visited = 0;
    let found = visit_pbes_expr(&example().copy(), (), |expr, context| {
        visited += 1;
        if format!("{expr}") == "Y" {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(Step::Into(context))
        }
    });

    assert!(found.is_some());
    assert_eq!(visited, 4, "expected ||, &&, X and Y to be visited");
}

#[test]
fn test_visit_prunes_and_threads_context() {
    // The context is the depth of the node; everything below the `&&` is skipped.
    let mut depths = Vec::new();
    let result: Option<Infallible> = visit_pbes_expr(&example().copy(), 0, |expr: &PbesExpressionRef<'_>, depth| {
        depths.push((format!("{expr}"), depth));
        if format!("{expr}") == "X && Y" {
            ControlFlow::Continue(Step::Prune)
        } else {
            ControlFlow::Continue(Step::Into(depth + 1))
        }
    });

    assert!(result.is_none());
    assert_eq!(
        depths,
        [
            ("(X && Y) || !Z".to_string(), 0),
            ("X && Y".to_string(), 1),
            ("!Z".to_string(), 1),
            ("Z".to_string(), 2),
        ]
    );
}

/// Every distinct subexpression is folded once, operands before the node that contains them,
/// and the cache is reused by later folds.
#[test]
fn test_fold_is_bottom_up_and_memoised() {
    let shared = PbesExpression::and(&instantiation("X"), &instantiation("Y"));
    let expr = PbesExpression::or(&shared, &PbesExpression::not(&shared));

    let mut folded = Vec::new();
    let mut cache = HashMap::new();
    let size = |expr: &PbesExpressionRef<'_>, node: PbesFold<'_, usize>, folded: &mut Vec<String>| {
        folded.push(format!("{expr}"));
        match node {
            PbesFold::True | PbesFold::False | PbesFold::Data(_) | PbesFold::Instantiation(_) => 1,
            PbesFold::Not(body) | PbesFold::Forall(_, body) | PbesFold::Exists(_, body) => body + 1,
            PbesFold::And(lhs, rhs) | PbesFold::Or(lhs, rhs) | PbesFold::Imp(lhs, rhs) => lhs + rhs + 1,
        }
    };

    // The size of the tree, even though `shared` is folded only once.
    let result = fold_pbes_expr(&expr.copy(), &mut cache, |expr, node| size(expr, node, &mut folded));
    assert_eq!(result, 8);
    assert_eq!(folded, ["X", "Y", "X && Y", "!(X && Y)", "(X && Y) || !(X && Y)"]);

    folded.clear();
    let result = fold_pbes_expr(&shared.copy(), &mut cache, |expr, node| size(expr, node, &mut folded));
    assert_eq!(result, 3);
    assert!(folded.is_empty());
}
