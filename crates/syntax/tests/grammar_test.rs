use indoc::indoc;
use merc_syntax::UntypedDataSpecification;
use pest::Parser;

use merc_syntax::Mcrl2Parser;
use merc_syntax::Rule;
use merc_syntax::UntypedProcessSpecification;
use merc_syntax::UntypedStateFrmSpec;
use merc_syntax::parse_sortexpr;
use merc_utilities::test_logger;

#[test]
fn test_parse_ifthen() {
    let expr = "init a -> b <> b;";

    match UntypedProcessSpecification::parse(expr) {
        Ok(result) => {
            println!("{}", result);
        }
        Err(e) => {
            panic!("Failed to parse expression: {}", e);
        }
    }
}

#[test]
fn test_parse_keywords() {
    let expr = "map or : Boolean # Boolean -> Boolean ;";

    let result = UntypedProcessSpecification::parse(expr).unwrap();
    print!("{}", result);
}

#[test]
fn test_parse_sort_spec() {
    let sort_spec = indoc! {"
        sort D = Bool -> Int -> Bool;
        

        % Test
        F     = struct d1 | d2;
        Error = struct e;
    "};

    let result = UntypedProcessSpecification::parse(sort_spec).unwrap();
    print!("{}", result);
}

#[test]
fn test_parse_regular_expression() {
    let spec = "[true++false]true";

    if let Err(y) = UntypedStateFrmSpec::parse(spec) {
        panic!("{}", y);
    }
}

#[test]
fn test_parse_procexpr() {
    let _ = test_logger();

    use indoc::indoc;

    let spec: &str = indoc! {"init
        true -> delta <> delta;
    "};

    println!("{}", UntypedProcessSpecification::parse(spec).unwrap());
}

#[test]
fn test_parse_statefrm() {
    let _ = test_logger();

    use indoc::indoc;

    let spec: &str = indoc! {"<b> <a> exists b: Bool . b && !b"};

    println!("{}", UntypedStateFrmSpec::parse(spec).unwrap());
}

#[test]
fn test_sort_precedence() {
    let term = "Bool # Int -> Int -> Bool";

    let result = Mcrl2Parser::parse(Rule::SortExpr, term).unwrap();
    print!("{}", parse_sortexpr(result).unwrap());
}

#[test]
fn test_bool_spec() {    
    let _result = UntypedDataSpecification::parse(include_str!("../spec/bool.mcrl2")).unwrap();
}