use std::collections::HashSet;

use merc_utilities::SourceMap;
use merc_utilities::Span;
use merc_utilities::Spanned;
use merc_utilities::respan;

/// A single-file [SourceMap] wrapping `source`, for tests that only care about rendering
/// against one document (where offsets equal the local, base-0 offsets pest reports).
fn single(source: &str) -> SourceMap {
    let mut sources = SourceMap::new();
    sources.add_text("<test>", source);
    sources
}

#[test]
fn test_start_line_col_first_line() {
    let span = Span { start: 4, end: 5 };
    assert_eq!(span.start_line_col("eqn f = x;"), (1, 5));
}

#[test]
fn test_start_line_col_counts_newlines() {
    let source = "sort D;\nmap f: D;\neqn f = undeclared;";
    let start = source.rfind("undeclared").unwrap();
    let span = Span {
        start,
        end: start + "undeclared".len(),
    };
    assert_eq!(span.start_line_col(source), (3, 9));
}

#[test]
fn test_start_line_col_multibyte() {
    // A multi-byte character before the span must not throw off the
    // column, which is counted in `char`s, not bytes.
    let source = "eqn é = x;";
    let start = source.rfind('x').unwrap();
    let span = Span { start, end: start + 1 };
    assert_eq!(span.start_line_col(source), (1, 9));
}

#[test]
fn test_render_single_line() {
    let source = "eqn f = undeclared;";
    let start = source.find("undeclared").unwrap();
    let span = Span {
        start,
        end: start + "undeclared".len(),
    };
    assert_eq!(
        span.render(&single(source)),
        " --> 1:9\n  |\n1 | eqn f = undeclared;\n  |         ^^^^^^^^^^"
    );
}

#[test]
fn test_render_later_line() {
    let source = "sort D;\nmap f: D;\neqn f = undeclared;";
    let start = source.rfind("undeclared").unwrap();
    let span = Span {
        start,
        end: start + "undeclared".len(),
    };
    assert_eq!(
        span.render(&single(source)),
        " --> 3:9\n  |\n3 | eqn f = undeclared;\n  |         ^^^^^^^^^^"
    );
}

#[test]
fn test_render_clamps_to_line_when_span_crosses_newline() {
    let source = "eqn f = x\n+ y;";
    let start = source.find('x').unwrap();
    // A span spuriously extending past the end of the line is still
    // underlined only up to that line's end.
    let span = Span {
        start,
        end: source.len(),
    };
    assert_eq!(
        span.render(&single(source)),
        " --> 1:9\n  |\n1 | eqn f = x\n  |         ^"
    );
}

#[test]
fn test_render_default_span_points_at_source_start() {
    let source = "eqn f = 1;";
    let span = Span::default();
    assert_eq!(span.render(&single(source)), " --> 1:1\n  |\n1 | eqn f = 1;\n  | ^");
}

#[test]
fn test_render_names_the_file_once_multiple_are_loaded() {
    let mut sources = SourceMap::new();
    let _first = sources.add_text("a.mcrl2", "sort D;");
    let second = sources.add_text("b.mcrl2", "sort E;");

    let span_in_first = Span { start: 5, end: 6 };
    assert_eq!(
        span_in_first.render(&sources),
        " --> a.mcrl2:1:6\n  |\n1 | sort D;\n  |      ^"
    );

    let base = sources.base_offset(second);
    let span_in_second = Span {
        start: base + 5,
        end: base + 6,
    };
    assert_eq!(
        span_in_second.render(&sources),
        " --> b.mcrl2:1:6\n  |\n1 | sort E;\n  |      ^"
    );
}

#[test]
fn test_shift_moves_both_endpoints() {
    let mut span = Span::new(2, 5);
    span.shift(10);
    assert_eq!(span, Span::new(12, 15));
}

#[test]
fn test_render_in_later_file_after_shift() {
    let mut sources = SourceMap::new();
    sources.add_text("a.mcrl2", "sort D;");
    let second = sources.add_text("b.mcrl2", "map f: D;\neqn f = g;");

    // A span local to the second file, rebased into the global offset space.
    let mut span = Span::new(18, 19);
    span.shift(sources.base_offset(second));
    assert_eq!(
        span.render(&sources),
        " --> b.mcrl2:2:9\n  |\n2 | eqn f = g;\n  |         ^"
    );
}

/// Equality, ordering and hashing of [Spanned] consider only the node, and [Spanned::map]
/// keeps the span.
#[test]
fn test_spanned_ignores_span() {
    let a = Spanned::new(1, Span::new(0, 1));
    let b = respan(Span::new(5, 9), 1);
    let c = Spanned::new(2, Span::new(0, 1));

    assert_eq!(a, b);
    assert_ne!(a, c);
    assert!(b < c);
    assert_eq!(HashSet::from([a.clone(), b.clone()]).len(), 1);

    assert_eq!(*b, 1);
    let mapped = b.map(|node| node + 1);
    assert_eq!(mapped.node, 2);
    assert_eq!(mapped.span, Span::new(5, 9));
}
