use merc_utilities::BinaryChain;
use merc_utilities::Boolean;
use merc_utilities::BooleanChain;
use merc_utilities::ChainStack;
use merc_utilities::optimized_and;
use merc_utilities::optimized_conjunction;
use merc_utilities::optimized_conjunction_chain;
use merc_utilities::optimized_disjunction;
use merc_utilities::optimized_disjunction_chain;
use merc_utilities::optimized_implies;
use merc_utilities::optimized_not;
use merc_utilities::optimized_or;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    And,
    Or,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Expr {
    True,
    False,
    Var(&'static str),
    Not(Box<Expr>),
    Binary(Op, Box<Expr>, Box<Expr>),
    Imp(Box<Expr>, Box<Expr>),
}

impl Boolean for Expr {
    fn true_constant() -> Self {
        Expr::True
    }

    fn false_constant() -> Self {
        Expr::False
    }

    fn is_true(&self) -> bool {
        *self == Expr::True
    }

    fn is_false(&self) -> bool {
        *self == Expr::False
    }

    fn negated(&self) -> Option<Self> {
        match self {
            Expr::Not(inner) => Some((**inner).clone()),
            _ => None,
        }
    }

    fn make_not(self) -> Self {
        Expr::Not(Box::new(self))
    }

    fn make_and(self, rhs: Self) -> Self {
        Expr::Binary(Op::And, Box::new(self), Box::new(rhs))
    }

    fn make_or(self, rhs: Self) -> Self {
        Expr::Binary(Op::Or, Box::new(self), Box::new(rhs))
    }

    fn make_imp(self, rhs: Self) -> Self {
        Expr::Imp(Box::new(self), Box::new(rhs))
    }
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

impl BooleanChain for &Expr {
    type Owned = Expr;

    fn conjunction() -> Op {
        Op::And
    }

    fn disjunction() -> Op {
        Op::Or
    }

    fn into_owned(self) -> Expr {
        self.clone()
    }
}

fn var(name: &'static str) -> Expr {
    Expr::Var(name)
}

#[test]
fn test_conjunction_chain_absorbs_nested_false() {
    let expr = var("A").make_and(var("B").make_and(Expr::False.make_and(var("C"))));

    let mut storage = ChainStack::<&Expr>::new();
    assert_eq!(optimized_conjunction_chain(&expr, &mut storage), Expr::False);
}

#[test]
fn test_conjunction_chain_drops_nested_true() {
    let expr = var("A").make_and(Expr::True.make_and(var("B")));

    let mut storage = ChainStack::<&Expr>::new();
    assert_eq!(
        optimized_conjunction_chain(&expr, &mut storage),
        var("A").make_and(var("B"))
    );
}

#[test]
fn test_disjunction_chain_keeps_other_connectives() {
    // The nested conjunction is an operand of the disjunction, so its `false` is not absorbing.
    let conjunction = var("B").make_and(Expr::False);
    let expr = Expr::False.make_or(var("A").make_or(conjunction.clone()));

    let mut storage = ChainStack::<&Expr>::new();
    assert_eq!(
        optimized_disjunction_chain(&expr, &mut storage),
        var("A").make_or(conjunction)
    );
    assert_eq!(
        optimized_disjunction_chain(&var("A").make_or(Expr::True), &mut storage),
        Expr::True
    );
}

/// The truth value of `expr` when variable `A` has value `a` and every other variable `b`.
fn eval(expr: &Expr, a: bool, b: bool) -> bool {
    match expr {
        Expr::True => true,
        Expr::False => false,
        Expr::Var(name) => {
            if *name == "A" {
                a
            } else {
                b
            }
        }
        Expr::Not(inner) => !eval(inner, a, b),
        Expr::Binary(Op::And, lhs, rhs) => eval(lhs, a, b) && eval(rhs, a, b),
        Expr::Binary(Op::Or, lhs, rhs) => eval(lhs, a, b) || eval(rhs, a, b),
        Expr::Imp(lhs, rhs) => !eval(lhs, a, b) || eval(rhs, a, b),
    }
}

/// Constants, variables, a negation and a double negation.
fn operands() -> Vec<Expr> {
    vec![
        Expr::True,
        Expr::False,
        var("A"),
        var("B"),
        var("A").make_not(),
        var("A").make_not().make_not(),
        var("A").make_and(var("B")),
    ]
}

/// Asserts that `actual` has the same truth value as `expected` under every assignment.
fn assert_equivalent(actual: &Expr, expected: impl Fn(bool, bool) -> bool, description: &str) {
    for a in [false, true] {
        for b in [false, true] {
            assert_eq!(
                eval(actual, a, b),
                expected(a, b),
                "{description} gave {actual:?}, wrong for A = {a}, B = {b}"
            );
        }
    }
}

/// Every optimized connective is equivalent to the connective itself, for all operand pairs.
#[test]
fn test_optimized_connectives_preserve_meaning() {
    for lhs in operands() {
        assert_equivalent(
            &optimized_not(lhs.clone()),
            |a, b| !eval(&lhs, a, b),
            &format!("!{lhs:?}"),
        );

        for rhs in operands() {
            let description = format!("{lhs:?}, {rhs:?}");
            assert_equivalent(
                &optimized_and(lhs.clone(), rhs.clone()),
                |a, b| eval(&lhs, a, b) && eval(&rhs, a, b),
                &format!("and {description}"),
            );
            assert_equivalent(
                &optimized_or(lhs.clone(), rhs.clone()),
                |a, b| eval(&lhs, a, b) || eval(&rhs, a, b),
                &format!("or {description}"),
            );
            assert_equivalent(
                &optimized_implies(lhs.clone(), rhs.clone()),
                |a, b| !eval(&lhs, a, b) || eval(&rhs, a, b),
                &format!("implies {description}"),
            );
        }
    }
}

/// Constant operands, equal operands and double negations never produce a new connective.
#[test]
fn test_optimized_connectives_simplify() {
    let x = var("A");
    let y = var("B");

    assert_eq!(optimized_not(Expr::True), Expr::False);
    assert_eq!(optimized_not(Expr::False), Expr::True);
    assert_eq!(optimized_not(x.clone().make_not()), x);
    assert_eq!(optimized_not(x.clone()), x.clone().make_not());

    for (lhs, rhs) in [(Expr::True, x.clone()), (x.clone(), Expr::True), (x.clone(), x.clone())] {
        assert_eq!(optimized_and(lhs, rhs), x);
    }
    for (lhs, rhs) in [(Expr::False, x.clone()), (x.clone(), Expr::False)] {
        assert_eq!(optimized_and(lhs, rhs), Expr::False);
    }
    assert_eq!(optimized_and(x.clone(), y.clone()), x.clone().make_and(y.clone()));

    for (lhs, rhs) in [
        (Expr::False, x.clone()),
        (x.clone(), Expr::False),
        (x.clone(), x.clone()),
    ] {
        assert_eq!(optimized_or(lhs, rhs), x);
    }
    for (lhs, rhs) in [(Expr::True, x.clone()), (x.clone(), Expr::True)] {
        assert_eq!(optimized_or(lhs, rhs), Expr::True);
    }
    assert_eq!(optimized_or(x.clone(), y.clone()), x.clone().make_or(y.clone()));

    assert_eq!(optimized_implies(Expr::True, x.clone()), x);
    for (lhs, rhs) in [
        (Expr::False, x.clone()),
        (x.clone(), Expr::True),
        (x.clone(), x.clone()),
    ] {
        assert_eq!(optimized_implies(lhs, rhs), Expr::True);
    }
    assert_eq!(optimized_implies(x.clone(), Expr::False), x.clone().make_not());
    assert_eq!(optimized_implies(x.clone().make_not(), Expr::False), x);
    assert_eq!(optimized_implies(x.clone(), y.clone()), x.make_imp(y));
}

#[test]
fn test_optimized_conjunction_and_disjunction() {
    assert_eq!(optimized_conjunction(Vec::<Expr>::new()), Expr::True);
    assert_eq!(optimized_disjunction(Vec::<Expr>::new()), Expr::False);

    assert_eq!(
        optimized_conjunction([Expr::True, var("A"), Expr::True, var("B")]),
        var("A").make_and(var("B"))
    );
    assert_eq!(
        optimized_disjunction([Expr::False, var("A"), Expr::False, var("B")]),
        var("A").make_or(var("B"))
    );

    // The first absorbing operand ends the iteration.
    let mut seen = 0;
    let terms = [var("A"), Expr::False, var("B")].into_iter().inspect(|_| seen += 1);
    assert_eq!(optimized_conjunction(terms), Expr::False);
    assert_eq!(seen, 2);

    let mut seen = 0;
    let terms = [var("A"), Expr::True, var("B")].into_iter().inspect(|_| seen += 1);
    assert_eq!(optimized_disjunction(terms), Expr::True);
    assert_eq!(seen, 2);
}
