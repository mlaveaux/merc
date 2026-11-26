use merc_utilities::BinaryChain;
use merc_utilities::ChainIter;
use merc_utilities::ChainStack;
use merc_utilities::unique_chain_operands;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Op {
    And,
    Or,
}

#[derive(PartialEq, Eq, Hash)]
enum Expr {
    Var(&'static str),
    Binary(Op, Box<Expr>, Box<Expr>),
}

impl<'a> BinaryChain for &'a Expr {
    type Op = Op;

    fn binary_operands(&self, op: Op) -> Option<(&'a Expr, &'a Expr)> {
        match *self {
            Expr::Binary(node_op, lhs, rhs) if *node_op == op => Some((lhs, rhs)),
            _ => None,
        }
    }
}

fn binary(op: Op, lhs: Expr, rhs: Expr) -> Expr {
    Expr::Binary(op, Box::new(lhs), Box::new(rhs))
}

fn names<'a>(operands: impl Iterator<Item = &'a Expr>) -> Vec<&'static str> {
    operands
        .map(|operand| match operand {
            Expr::Var(name) => *name,
            Expr::Binary(..) => "<binary>",
        })
        .collect()
}

#[test]
fn test_chain_iter_yields_operands_in_written_order() {
    let left_nested = binary(Op::And, binary(Op::And, Expr::Var("A"), Expr::Var("B")), Expr::Var("C"));
    let right_nested = binary(Op::And, Expr::Var("A"), binary(Op::And, Expr::Var("B"), Expr::Var("C")));

    let mut storage = ChainStack::<&Expr>::new();
    assert_eq!(
        names(ChainIter::new(&left_nested, Op::And, &mut storage)),
        ["A", "B", "C"]
    );
    assert_eq!(
        names(ChainIter::new(&right_nested, Op::And, &mut storage)),
        ["A", "B", "C"]
    );
}

#[test]
fn test_chain_iter_stops_at_other_connectives() {
    let expr = binary(Op::And, Expr::Var("A"), binary(Op::Or, Expr::Var("B"), Expr::Var("C")));

    let mut storage = ChainStack::<&Expr>::new();
    assert_eq!(names(ChainIter::new(&expr, Op::And, &mut storage)), ["A", "<binary>"]);
    assert_eq!(names(ChainIter::new(&expr, Op::Or, &mut storage)), ["<binary>"]);
}

/// The lower bound of the size hint never exceeds the number of remaining operands, and the
/// iterator stays exhausted once it returns `None`.
#[test]
fn test_chain_iter_size_hint_is_a_lower_bound() {
    let exprs = [
        (Expr::Var("A"), 1),
        (
            binary(Op::And, Expr::Var("A"), binary(Op::And, Expr::Var("B"), Expr::Var("C"))),
            3,
        ),
        (
            binary(
                Op::And,
                binary(Op::And, Expr::Var("A"), Expr::Var("B")),
                binary(Op::And, Expr::Var("C"), Expr::Var("D")),
            ),
            4,
        ),
    ];

    let mut storage = ChainStack::<&Expr>::new();
    for (expr, operands) in &exprs {
        let mut remaining = *operands;
        let mut iter = ChainIter::new(expr, Op::And, &mut storage);
        loop {
            let (lower, upper) = iter.size_hint();
            assert!(
                lower <= remaining,
                "lower bound {lower} exceeds {remaining} remaining operands"
            );
            assert!(upper.is_none_or(|upper| upper >= remaining));
            if iter.next().is_none() {
                break;
            }
            remaining -= 1;
        }
        assert_eq!(remaining, 0);
        assert!(iter.next().is_none());
    }
}

#[test]
fn test_unique_chain_operands_skips_duplicates() {
    let expr = binary(
        Op::And,
        binary(Op::And, Expr::Var("A"), Expr::Var("B")),
        binary(Op::And, Expr::Var("A"), binary(Op::Or, Expr::Var("B"), Expr::Var("C"))),
    );

    assert_eq!(
        names(unique_chain_operands(&expr, Op::And).into_iter()),
        ["A", "B", "<binary>"]
    );
}
