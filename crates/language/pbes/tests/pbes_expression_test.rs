use merc_pbes::PbesExpression;
use merc_pbes::PropositionalVariableInstantiation;
use merc_pbes::split_and;
use merc_pbes::split_or;

fn instantiation(name: &str) -> PbesExpression {
    PropositionalVariableInstantiation::new(name, &[]).into()
}

/// The operands are returned left to right, once each, and only the given connective is split.
#[test]
fn test_split_and_or() {
    let (x, y, z) = (instantiation("X"), instantiation("Y"), instantiation("Z"));
    let or = PbesExpression::or(&y, &z);
    let expr = PbesExpression::and(&PbesExpression::and(&x, &or), &PbesExpression::and(&x, &z));

    let operands: Vec<_> = split_and(&expr.copy()).into_iter().map(|e| e.protect()).collect();
    assert_eq!(operands, [x.clone(), or.clone(), z.clone()]);

    let operands: Vec<_> = split_or(&or.copy()).into_iter().map(|e| e.protect()).collect();
    assert_eq!(operands, [y, z]);

    let operands: Vec<_> = split_or(&expr.copy()).into_iter().map(|e| e.protect()).collect();
    assert_eq!(operands, [expr]);
}

/// `e(n+1) = e(n) && e(n)` is linear in size as a term graph, so splitting it must be too.
#[test]
fn test_split_shared_conjunction() {
    let x = instantiation("X");
    let mut expr = x.clone();
    for _ in 0..128 {
        expr = PbesExpression::and(&expr, &expr);
    }

    let operands: Vec<_> = split_and(&expr.copy()).into_iter().map(|e| e.protect()).collect();
    assert_eq!(operands, [x]);
}
