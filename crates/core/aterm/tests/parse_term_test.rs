//! Parsing of terms from text, including the error spans.

use merc_aterm::ATerm;
use merc_aterm::ATermInt;
use merc_aterm::ParseTermError;
use merc_aterm::Symb;
use merc_aterm::Term;
use merc_aterm::is_int_term;

#[test]
fn test_parse_term() {
    let term = ATerm::from_string("f(a, g(b), c)").unwrap();

    assert_eq!(term.get_head_symbol().name(), "f");
    assert_eq!(term.get_head_symbol().arity(), 3);
    assert_eq!(term.arg(1).get_head_symbol().name(), "g");
    assert_eq!(term.to_string(), "f(a, g(b), c)");
}

#[test]
fn test_parse_term_with_comments_and_white_space() {
    let term = ATerm::from_string("  f( % the first argument\n\ta ,b ) % trailing").unwrap();

    assert_eq!(term, ATerm::from_string("f(a,b)").unwrap());
}

#[test]
fn test_parse_integer() {
    let term = ATerm::from_string("f(42, x1)").unwrap();

    assert!(is_int_term(&term.arg(0)));
    assert_eq!(ATermInt::from(term.arg(0).protect()).value(), 42);
    assert!(!is_int_term(&term.arg(1)));
}

#[test]
#[cfg_attr(miri, ignore)] // Parsing 100k nested terms is too slow under miri.
fn test_parse_deeply_nested_term() {
    let depth = 100_000;
    let text = format!("{}a{}", "s(".repeat(depth), ")".repeat(depth));

    let mut term = ATerm::from_string(&text).unwrap();
    for _ in 0..depth {
        assert_eq!(term.get_head_symbol().name(), "s");
        term = term.arg(0).protect();
    }
    assert_eq!(term.get_head_symbol().name(), "a");
}

#[test]
fn test_parse_errors_point_at_the_input() {
    let span = |text: &str| {
        let error = ATerm::from_string(text).unwrap_err();
        let error = error.downcast_ref::<ParseTermError>().unwrap();
        (error.span().start, error.span().end)
    };

    assert_eq!(span(""), (0, 0));
    assert_eq!(span("f()"), (2, 3));
    assert_eq!(span("f(a b)"), (4, 5));
    assert_eq!(span("f(a"), (3, 3));
    assert_eq!(span("f(a) b"), (5, 6));
    assert_eq!(span("1(a)"), (0, 2));
    assert_eq!(span("99999999999999999999999"), (0, 23));
}

#[test]
fn test_parse_error_renders_the_source_line() {
    let error = ATerm::from_string("f(a,\n  g(b c))").unwrap_err();
    // Render the parse error itself, since MercError appends a backtrace when RUST_BACKTRACE is set.
    let error = error.downcast_ref::<ParseTermError>().unwrap();

    assert_eq!(
        error.to_string(),
        "expected ',' or ')', found 'c'\n --> 2:7\n  |\n2 |   g(b c))\n  |       ^"
    );
}
