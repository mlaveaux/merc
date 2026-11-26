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

impl From<pest::Span<'_>> for Span {
    fn from(span: pest::Span) -> Self {
        Span {
            start: span.start(),
            end: span.end(),
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
