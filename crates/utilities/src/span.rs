use std::cell::RefCell;
use std::cmp::Ordering;
use std::hash::Hash;
use std::hash::Hasher;
use std::ops::Deref;
use std::ops::DerefMut;

use crate::SourceMap;

/// Source location information, spanning from start to end in the source text.
#[derive(Clone, Default, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

thread_local! {
    /// Ambient byte-offset correction applied by [Span]'s `From<pest::Span>` conversion, set up by
    /// [with_offset_corrections]. `None` (the default) means "no correction, use positions as-is".
    static OFFSET_CORRECTION: RefCell<Option<(Vec<usize>, usize)>> = const { RefCell::new(None) };
}

/// Runs `f` while every [Span] built from a `pest::Span` (via `Span::from`/`.into()`) is corrected
/// as though `insertions.len() * insertion_len` fewer bytes existed before it: a parser given text
/// with `insertion_len` placeholder bytes spliced in at each of `insertions` (byte offsets into
/// *that* text, sorted ascending) reports positions in the spliced text, and this makes the spans
/// it builds come out as if it had parsed the text without those placeholders instead.
///
/// This exists for a parser that needs a little extra, otherwise-unrepresentable syntax spliced
/// into its input to disambiguate what it's about to parse (see `merc_syntax`'s condition-marker
/// preprocessing), without every one of that parser's many span-computing call sites having to
/// know about it. It is a thread-local rather than an explicit parameter for exactly that reason;
/// nested calls save and restore the previous correction (via an RAII guard, so a panicking `f`
/// still restores it), so it composes safely with itself even though nothing in this codebase
/// currently nests it.
pub fn with_offset_corrections<R>(insertions: &[usize], insertion_len: usize, f: impl FnOnce() -> R) -> R {
    struct RestoreOnDrop(Option<(Vec<usize>, usize)>);

    impl Drop for RestoreOnDrop {
        fn drop(&mut self) {
            OFFSET_CORRECTION.with(|cell| *cell.borrow_mut() = self.0.take());
        }
    }

    let previous = OFFSET_CORRECTION.with(|cell| cell.replace(Some((insertions.to_vec(), insertion_len))));
    let _restore = RestoreOnDrop(previous);
    f()
}

/// Maps a position through the ambient correction installed by [with_offset_corrections], if any.
fn correct_offset(position: usize) -> usize {
    OFFSET_CORRECTION.with(|cell| match cell.borrow().as_ref() {
        Some((insertions, insertion_len)) => {
            let count = insertions.partition_point(|&inserted| inserted < position);
            position - count * insertion_len
        }
        None => position,
    })
}

impl From<pest::Span<'_>> for Span {
    fn from(span: pest::Span) -> Self {
        Span {
            start: correct_offset(span.start()),
            end: correct_offset(span.end()),
        }
    }
}

impl Span {
    /// Creates a span covering the byte range `[start, end)`.
    pub fn new(start: usize, end: usize) -> Self {
        Span { start, end }
    }

    /// Moves both endpoints forward by `delta` — rebasing a span produced by parsing a file's text
    /// alone into the [SourceMap]-wide offset space, in place of padding that text with `delta`
    /// leading bytes before parsing it.
    pub fn shift(&mut self, delta: usize) {
        self.start += delta;
        self.end += delta;
    }

    /// The 1-based (line, column) of `self.start` within `source`, counted in
    /// `char`s rather than bytes so the column lines up under multi-byte
    /// UTF-8 text.
    pub fn start_line_col(&self, source: &str) -> (usize, usize) {
        let mut line = 1;
        let mut col = 1;
        for ch in source[..self.start.min(source.len())].chars() {
            if ch == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        (line, col)
    }

    /// Renders this span against `sources` as a caret-annotated snippet, in
    /// the `-->`/`|`/`^^^` style `pest` and `rustc` diagnostics use, so
    /// parser errors and later-pass errors (type errors, …) read
    /// consistently:
    ///
    /// ```text
    ///  --> 1:23
    ///   |
    /// 1 | eqn f = undeclared;
    ///   |         ^^^^^^^^^^
    /// ```
    ///
    /// `self.start` is looked up in `sources` (a global byte offset shared
    /// across every loaded file, see [SourceMap]) to find which file it
    /// falls into; the header names that file ahead of `line:col` only once
    /// more than one file is loaded, so a single-file `SourceMap` renders
    /// exactly as a bare source string did before spans became
    /// multi-document aware.
    ///
    /// A span crossing a newline is underlined only up to the end of its
    /// first line; an out-of-range span (e.g. [Span::default] on a synthetic
    /// node) renders against the start of its file.
    pub fn render(&self, sources: &SourceMap) -> String {
        let id = sources.lookup(self.start);
        let base = sources.base_offset(id);
        let source = sources.text(id);
        let local = Span::new(self.start.saturating_sub(base), self.end.saturating_sub(base));

        let (line, col) = local.start_line_col(source);
        let line_text = source.lines().nth(line - 1).unwrap_or("");

        let span_len = source
            .get(local.start..local.end.max(local.start))
            .map_or(1, |text| text.chars().count())
            .max(1);
        let underline_len = span_len.min(line_text.chars().count().saturating_sub(col - 1).max(1));

        let gutter = " ".repeat(line.to_string().len());
        let location = if sources.file_count() > 1 {
            format!("{}:{line}:{col}", sources.path(id))
        } else {
            format!("{line}:{col}")
        };
        format!(
            "{gutter}--> {location}\n{gutter} |\n{line} | {line_text}\n{gutter} | {}{}",
            " ".repeat(col - 1),
            "^".repeat(underline_len),
        )
    }
}

/// A value of type `T` paired with the source [Span] it originates from.
///
/// This mirrors rustc's `Spanned<T>` / node-struct pattern: the wrapper carries
/// the location while the inner `node` holds the actual syntax. It is used to
/// give every expression node a span without threading a `span` field into each
/// enum variant.
///
/// Equality, ordering and hashing deliberately ignore the [Span] and consider
/// only `node`, so two structurally identical values at different source
/// locations compare and hash equal. Many passes rely on this structural
/// equality (hash maps, deduplication, `assert_eq!` in tests).
#[derive(Clone, Debug)]
pub struct Spanned<T> {
    /// The wrapped value.
    pub node: T,
    /// The source location the value originates from.
    pub span: Span,
}

impl<T> Spanned<T> {
    /// Wraps `node` together with its source `span`.
    pub fn new(node: T, span: Span) -> Self {
        Spanned { node, span }
    }

    /// Transforms the wrapped value while preserving the span.
    pub fn map<U>(self, function: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            node: function(self.node),
            span: self.span,
        }
    }
}

/// Wraps `node` together with its source `span`; the free-function counterpart
/// of [Spanned::new], mirroring rustc's `respan`.
pub fn respan<T>(span: Span, node: T) -> Spanned<T> {
    Spanned { node, span }
}

impl<T> Deref for Spanned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.node
    }
}

impl<T> DerefMut for Spanned<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.node
    }
}

impl<T: PartialEq> PartialEq for Spanned<T> {
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node
    }
}

impl<T: Eq> Eq for Spanned<T> {}

impl<T: PartialOrd> PartialOrd for Spanned<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.node.partial_cmp(&other.node)
    }
}

impl<T: Ord> Ord for Spanned<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.node.cmp(&other.node)
    }
}

impl<T: Hash> Hash for Spanned<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.node.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::Span;
    use crate::SourceMap;

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
}
