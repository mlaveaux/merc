//! Differential tests: [SabreRewriter], [InnermostRewriter] and [NaiveRewriter] must agree on
//! the normal form of random terms over a specification with duplicating, non-linear and
//! conditional rules.

use ahash::AHashSet;
use rand::Rng;
use rand::RngExt;

use merc_data::DataExpression;
use merc_sabre::Condition;
use merc_sabre::InnermostRewriter;
use merc_sabre::NaiveRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;
use merc_sabre::Rule;
use merc_sabre::SabreRewriter;
use merc_utilities::random_test;

/// Parses `input` treating `x` and `y` as variables.
fn term(input: &str) -> DataExpression {
    let variables: AHashSet<String> = ["x", "y"].iter().map(|v| v.to_string()).collect();
    DataExpression::from_string_untyped(input, &variables).unwrap()
}

/// Natural numbers with addition and multiplication, an if-then-else, a non-linear equality
/// and a maximum defined by two conditional rules.
fn spec() -> RewriteSpecification {
    let rules = vec![
        Rule::new(term("plus(d0, y)"), term("y")),
        Rule::new(term("plus(s(x), y)"), term("s(plus(x, y))")),
        Rule::new(term("times(d0, y)"), term("d0")),
        Rule::new(term("times(s(x), y)"), term("plus(y, times(x, y))")),
        Rule::new(term("gt(d0, y)"), term("false")),
        Rule::new(term("gt(s(x), d0)"), term("true")),
        Rule::new(term("gt(s(x), s(y))"), term("gt(x, y)")),
        Rule::new(term("eq(x, x)"), term("true")),
        Rule::new(term("ite(true, x, y)"), term("x")),
        Rule::new(term("ite(false, x, y)"), term("y")),
        Rule::with_condition(
            vec![Condition::new(term("gt(x, y)"), term("true"), true)],
            term("max(x, y)"),
            term("x"),
        ),
        Rule::with_condition(
            vec![Condition::new(term("gt(x, y)"), term("true"), false)],
            term("max(x, y)"),
            term("y"),
        ),
    ];
    RewriteSpecification::new(rules)
}

/// Returns a random closed term of at most `depth` levels over the symbols of [spec].
fn random_term(rng: &mut impl Rng, depth: usize) -> String {
    const LEAVES: [&str; 5] = ["d0", "s(d0)", "s(s(d0))", "true", "false"];
    const BINARY: [&str; 5] = ["plus", "times", "gt", "eq", "max"];

    if depth == 0 || rng.random_bool(0.2) {
        return LEAVES[rng.random_range(0..LEAVES.len())].to_string();
    }

    match rng.random_range(0..3) {
        0 => format!("s({})", random_term(rng, depth - 1)),
        1 => format!(
            "ite({}, {}, {})",
            random_term(rng, depth - 1),
            random_term(rng, depth - 1),
            random_term(rng, depth - 1)
        ),
        _ => format!(
            "{}({}, {})",
            BINARY[rng.random_range(0..BINARY.len())],
            random_term(rng, depth - 1),
            random_term(rng, depth - 1)
        ),
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_engines_agree_on_fixed_terms() {
    let spec = spec();
    let cases = [
        ("times(s(s(d0)), s(s(s(d0))))", "s(s(s(s(s(s(d0))))))"),
        ("max(s(d0), s(s(d0)))", "s(s(d0))"),
        ("max(s(s(d0)), plus(d0, s(d0)))", "s(s(d0))"),
        ("eq(plus(s(d0), d0), times(s(d0), s(d0)))", "true"),
        ("eq(s(d0), d0)", "eq(s(d0), d0)"),
        ("ite(gt(s(d0), d0), s(d0), d0)", "s(d0)"),
    ];

    for (input, expected) in cases {
        let input = term(input);
        let expected = term(expected);
        assert_eq!(
            InnermostRewriter::new(&spec).rewrite(&input),
            expected,
            "innermost on {input}"
        );
        assert_eq!(SabreRewriter::new(&spec).rewrite(&input), expected, "sabre on {input}");
        assert_eq!(NaiveRewriter::new(&spec).rewrite(&input), expected, "naive on {input}");
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_engines_agree_with_innermost() {
    let spec = spec();
    let mut inner = InnermostRewriter::new(&spec);
    let mut sabre = SabreRewriter::new(&spec);
    let mut naive = NaiveRewriter::new(&spec);

    random_test(300, |rng| {
        let input = term(&random_term(rng, 3));
        let expected = inner.rewrite(&input);

        assert_eq!(sabre.rewrite(&input), expected, "sabre on {input}");
        assert_eq!(naive.rewrite(&input), expected, "naive on {input}");
    });
}
