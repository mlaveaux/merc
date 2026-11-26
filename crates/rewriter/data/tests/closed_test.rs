use ahash::AHashSet;
use merc_data::BinderType;
use merc_data::DataAbstraction;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataFunctionSymbol;
use merc_data::DataVariable;
use merc_data::collect_variables;
use merc_data::is_closed;
use merc_data::mark_occurrences;

fn parse(text: &str) -> DataExpression {
    let variables = AHashSet::from_iter(["x".to_string(), "y".to_string(), "z".to_string()]);
    DataExpression::from_string_untyped(text, &variables).unwrap()
}

/// `e_n` with `e_0 = leaf` and `e_{i+1} = f(e_i, b)` followed by `f(e_i, e_i)`, so `leaf` and `b`
/// are both reached along `2^n` paths.
fn exponential_dag(leaf: DataExpression, n: usize) -> DataExpression {
    let f = DataFunctionSymbol::new("f");
    let b: DataExpression = DataFunctionSymbol::new("b").into();
    let mut term = leaf;
    for _ in 0..n {
        let with_b: DataExpression = DataApplication::with_args(&f, &[term, b.clone()]).into();
        term = DataApplication::with_args(&f, &[with_b.clone(), with_b]).into();
    }
    term
}

#[test]
fn test_is_closed() {
    assert!(is_closed(&parse("s(s(a), b)")));
    assert!(is_closed(&parse("a")));
    assert!(!is_closed(&parse("s(s(x), b)")));
    assert!(!is_closed(&parse("s(b, s(b, x))")));
    assert!(!is_closed(&parse("x")));
}

/// Occurrences of a bound variable count, as documented.
#[test]
fn test_is_closed_counts_bound_variables() {
    let x = DataVariable::new("x");
    let lambda: DataExpression =
        DataAbstraction::new(BinderType::Lambda, std::slice::from_ref(&x), x.clone().into()).into();
    assert!(!is_closed(&lambda));

    // A binder whose body does not mention a variable is closed.
    let constant: DataExpression =
        DataAbstraction::new(BinderType::Lambda, &[x], DataFunctionSymbol::new("a").into()).into();
    assert!(is_closed(&constant));
}

/// Shared subterms are visited once, so a term that unfolds to `2^100` nodes is decided quickly.
#[test]
fn test_is_closed_on_an_exponential_dag() {
    assert!(is_closed(&exponential_dag(DataFunctionSymbol::new("a").into(), 50)));
    assert!(!is_closed(&exponential_dag(DataVariable::new("x").into(), 50)));
}

#[test]
fn test_mark_occurrences_finds_only_occurring_variables() {
    let term = parse("s(x, s(x, z))");
    let candidates = [DataVariable::new("x"), DataVariable::new("y"), DataVariable::new("z")];

    let mut occurs = [false; 3];
    mark_occurrences(&term, &candidates, &mut occurs);
    assert_eq!(occurs, [true, false, true]);
}

#[test]
fn test_mark_occurrences_keeps_earlier_marks_and_ignores_the_tail() {
    let term = parse("s(x, b)");
    let candidates = [DataVariable::new("x"), DataVariable::new("y")];

    // An entry that is already set stays set, and entries past `variables.len()` are untouched.
    let mut occurs = [false, true, false];
    mark_occurrences(&term, &candidates, &mut occurs);
    assert_eq!(occurs, [true, true, false]);
}

#[test]
fn test_mark_occurrences_on_an_exponential_dag() {
    let term = exponential_dag(DataVariable::new("y").into(), 50);
    let candidates = [DataVariable::new("x"), DataVariable::new("y")];

    let mut occurs = [false; 2];
    mark_occurrences(&term, &candidates, &mut occurs);
    assert_eq!(occurs, [false, true]);
}

#[test]
#[should_panic]
fn test_mark_occurrences_panics_on_a_short_occurs() {
    let mut occurs = [false; 1];
    mark_occurrences(
        &parse("x"),
        &[DataVariable::new("x"), DataVariable::new("y")],
        &mut occurs,
    );
}

#[test]
fn test_collect_variables_collects_each_variable_once() {
    let mut collected = AHashSet::new();
    collect_variables(&parse("s(x, s(y, x))"), &mut collected);
    assert_eq!(
        collected,
        AHashSet::from_iter([DataVariable::new("x"), DataVariable::new("y")])
    );

    // Variables are added to the ones already present.
    collect_variables(&exponential_dag(DataVariable::new("z").into(), 50), &mut collected);
    assert_eq!(
        collected,
        AHashSet::from_iter([DataVariable::new("x"), DataVariable::new("y"), DataVariable::new("z")])
    );

    let mut none = AHashSet::new();
    collect_variables(&parse("s(a, b)"), &mut none);
    assert!(none.is_empty());
}
