use ahash::AHashSet;
use merc_aterm::ATerm;
use merc_aterm::ATermInt;
use merc_aterm::Term;
use merc_data::DataApplication;
use merc_data::DataEquation;
use merc_data::DataExpression;
use merc_data::DataFunctionSymbol;
use merc_data::DataVariable;
use merc_data::SortExpression;
use merc_data::is_data_application;
use merc_data::is_data_function_symbol;
use merc_data::is_data_machine_number;
use merc_data::is_data_variable;

#[test]
fn test_function_symbol_with_sort() {
    let sort = SortExpression::nat();
    let f = DataFunctionSymbol::with_sort("f", sort.copy());

    assert_eq!(f.name(), "f");
    assert_eq!(f.sort().protect(), sort);
}

#[test]
fn test_print() {
    let a = DataFunctionSymbol::new("a");
    assert_eq!(format!("{a}"), "a");

    let f = DataFunctionSymbol::new("f");
    let x = DataVariable::new("x");
    let appl = DataApplication::with_args(&f, &[DataExpression::from(a), x.into()]);
    assert_eq!(format!("{appl}"), "f(a, x)");
}

#[test]
fn test_recognizers() {
    let a = DataFunctionSymbol::new("a");
    let f = DataFunctionSymbol::new("f");
    let appl: ATerm = DataApplication::with_args(&f, std::slice::from_ref(&a)).into();
    let a: ATerm = a.into();
    let x: ATerm = DataVariable::new("x").into();

    assert!(is_data_application(&appl));
    assert!(!is_data_function_symbol(&appl));
    assert!(is_data_function_symbol(&a));
    assert!(!is_data_application(&a));
    assert!(!is_data_variable(&a));
    assert!(is_data_variable(&x));
    assert!(!is_data_function_symbol(&x));
}

#[test]
fn test_data_arguments() {
    let a: DataExpression = DataFunctionSymbol::new("a").into();
    let b: DataExpression = DataFunctionSymbol::new("b").into();
    let f = DataFunctionSymbol::new("f");
    let appl = DataApplication::with_args(&f, &[a.clone(), b.clone()]);

    let arguments: Vec<DataExpression> = appl
        .data_arguments()
        .map(|arg| DataExpression::from(arg.protect()))
        .collect();
    assert_eq!(arguments, [a.clone(), b.clone()]);

    let data_expr: DataExpression = appl.into();
    let arguments: Vec<DataExpression> = data_expr.data_arguments().map(|arg| arg.protect()).collect();
    assert_eq!(arguments, [a.clone(), b]);
    assert_eq!(data_expr.data_function_symbol().protect(), f);

    // A constant has no arguments and is its own head symbol.
    assert_eq!(a.data_arguments().count(), 0);
    assert_eq!(a.data_function_symbol().name(), "a");
}

#[test]
fn test_from_string() {
    let expression = DataExpression::from_string("s(s(a, b), c)").unwrap();

    assert_eq!(expression.data_function_symbol().name(), "s");
    assert_eq!(expression.data_arguments().count(), 2);
    assert_eq!(expression.data_arg(0).data_function_symbol().name(), "s");
    assert_eq!(expression.data_arg(0).data_arg(0).data_function_symbol().name(), "a");
    assert_eq!(expression.data_arg(0).data_arg(1).data_function_symbol().name(), "b");
    assert_eq!(expression.data_arg(1).data_function_symbol().name(), "c");
    assert_eq!(format!("{expression}"), "s(s(a, b), c)");
}

#[test]
fn test_machine_number() {
    let term: ATerm = ATermInt::new(42).into();
    assert!(is_data_machine_number(&term));

    let expr: DataExpression = term.into();
    assert_eq!(format!("{expr}"), "42");
    // Machine numbers have no data sub-expressions.
    assert_eq!(expr.data_arguments().count(), 0);
}

#[test]
fn test_variable_sort() {
    let var = DataVariable::new("x");
    assert_eq!(var.name(), "x");

    let expr: DataExpression = var.into();
    assert!(is_data_variable(&expr));
    assert_eq!(expr.data_sort().name(), "@no_value@");
    assert_eq!(expr.data_arguments().count(), 0);

    let typed: DataExpression = DataVariable::with_sort("y", SortExpression::nat().copy()).into();
    assert_eq!(typed.data_sort(), SortExpression::nat());
}

#[test]
fn test_from_string_untyped_variable() {
    let vars = AHashSet::from_iter(["x".to_string()]);
    let expr = DataExpression::from_string_untyped("f(x, a)", &vars).unwrap();

    // 'x' is recognised as a variable, 'a' stays a function symbol.
    assert_eq!(expr.data_arg(0).protect(), DataExpression::from(DataVariable::new("x")));
    assert!(is_data_function_symbol(&expr.data_arg(1)));
    assert_eq!(expr.data_arg(1).data_function_symbol().name(), "a");
}

#[test]
fn test_from_string_untyped_applied_identifier_keeps_args() {
    // 'x' is in the variable set but appears applied; it must stay an application rather than
    // collapsing to a variable and silently dropping its argument.
    let vars = AHashSet::from_iter(["x".to_string()]);
    let expr = DataExpression::from_string_untyped("x(a)", &vars).unwrap();

    assert!(is_data_application(&expr));
    assert_eq!(expr.data_function_symbol().name(), "x");
    assert_eq!(expr.data_arguments().count(), 1);
    assert_eq!(expr.data_arg(0).data_function_symbol().name(), "a");
}

#[test]
fn test_data_equation_unconditional() {
    let sort = SortExpression::nat();
    let x = DataVariable::with_sort("x", sort.copy());
    let lhs: DataExpression =
        DataApplication::with_args(&DataFunctionSymbol::new("f"), std::slice::from_ref(&x)).into();
    let rhs: DataExpression = x.clone().into();

    let equation = DataEquation::new(std::slice::from_ref(&x), None, lhs.clone(), rhs.clone());

    assert!(equation.condition().is_none());
    assert_eq!(equation.lhs().protect(), lhs);
    assert_eq!(equation.rhs().protect(), rhs);
    assert_eq!(equation.variables().to_vec(), vec![x]);
    assert_eq!(format!("{equation}"), "f(x) = x");
}

#[test]
fn test_data_equation_conditional() {
    let sort = SortExpression::bool();
    let b = DataVariable::with_sort("b", sort.copy());
    let condition: DataExpression = b.clone().into();
    let lhs: DataExpression = DataFunctionSymbol::new("f").into();
    let rhs: DataExpression = DataFunctionSymbol::new("g").into();

    let equation = DataEquation::new(
        std::slice::from_ref(&b),
        Some(condition.clone()),
        lhs.clone(),
        rhs.clone(),
    );

    assert_eq!(
        equation.condition().map(|condition| condition.protect()),
        Some(condition)
    );
    assert_eq!(equation.lhs().protect(), lhs);
    assert_eq!(equation.rhs().protect(), rhs);
    assert_eq!(equation.variables().to_vec(), vec![b]);
    assert_eq!(format!("{equation}"), "b -> f = g");
}
