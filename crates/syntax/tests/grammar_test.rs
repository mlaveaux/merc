use indoc::indoc;
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
fn test_parse_sort_spec() {
    let sort_spec = indoc! {"
        sort D = Bool -> Int -> Bool;
        

        % Test
        F     = struct d1 | d2;
        Error = struct e;
    "};

    match UntypedProcessSpecification::parse(sort_spec) {
        Ok(result) => {
            println!("{}", result);
        }
        Err(e) => {
            panic!("Failed to parse expression: {}", e);
        }
    }
}

#[test]
fn test_parse_regular_expression() {
    let spec = "[true++false]true";

    match UntypedStateFrmSpec::parse(spec) {
        Ok(result) => {
            println!("{}", result);
        }
        Err(e) => {
            panic!("Failed to parse expression: {}", e);
        }
    }
}

#[test]
fn test_parse_procexpr() {
    test_logger();

    use indoc::indoc;

    let spec: &str = indoc! {"init
        true -> delta <> delta;
    "};

    match UntypedProcessSpecification::parse(spec) {
        Ok(result) => {
            println!("{}", result);
        }
        Err(e) => {
            panic!("Failed to parse expression: {}", e);
        }
    }
}

#[test]
fn test_parse_statefrm() {
    test_logger();

    use indoc::indoc;

    let spec: &str = indoc! {"<b> <a> exists b: Bool . b && !b"};

    match UntypedStateFrmSpec::parse(spec) {
        Ok(result) => {
            println!("{}", result);
        }
        Err(e) => {
            panic!("Failed to parse expression: {}", e);
        }
    }
}

#[test]
fn test_sort_precedence() {
    let term = "Bool # Int -> Int -> Bool";

    match Mcrl2Parser::parse(Rule::SortExpr, term) {
        Ok(result) => {
            print!("{}", parse_sortexpr(result).unwrap());
        }
        Err(e) => {
            panic!("{}", e);
        }
    }
}

// #[test]
// fn test_bool_spec() {
//     match UntypedDataSpecification::parse(include_str!("../spec/bool.mcrl2")) {
//         Ok(result) => {
//             println!("{}", result);
//         }
//         Err(e) => {
//             panic!("Failed to parse expression: {}", e);
//         }
//     }
// }
