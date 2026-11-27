//! `%import "relative/path.mcrl2"` — splicing one data specification's declarations into
//! another before type checking, using mCRL2's own comment character so a file that uses it
//! stays valid, ordinary mCRL2.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use merc_pest_consume::Error as PestError;
use merc_utilities::MercError;
use merc_utilities::SourceId;
use merc_utilities::SourceMap;
use merc_utilities::Span;
use merc_utilities::Spanned;

use crate::OffsetSpans;
use crate::Rule;
use crate::UntypedDataSpecification;
use crate::UntypedProcessSpecification;
use crate::UntypedStateFrmSpec;

/// One `%import "relative/path"` directive, as found by [scan_imports]: the raw path text
/// between the quotes, not yet resolved against the importing file's directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportDirective {
    pub path: String,
    /// Span of just the quoted path text (excluding the quotes themselves).
    pub path_span: Span,
}

/// A parse failure's location and message, extracted from the parser's own error at the point
/// [ImportError::Parse] is built — see [ImportError::syntax_error].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxError {
    /// Where the failure was reported, at the same shared, global ([SourceMap]-wide) offset as
    /// every other span produced while resolving this import tree.
    pub span: Span,
    /// The parser's own rendered message, avoids depending on `pest`.
    pub message: String,
}

/// The `%import` graph resolved alongside one `parse_with_imports` call: which file every
/// directive, transitively, ends up resolving to.
#[derive(Clone, Debug, Default)]
pub struct ImportGraph {
    /// The [SourceId] of the file `parse_with_imports` was called on.
    pub root: SourceId,
    /// One entry per `%import` directive actually resolved, in load order: the importing file,
    /// the directive's own span (at the importer's shared, global offset), and the file it
    /// resolved to.
    pub edges: Vec<(SourceId, Span, SourceId)>,
}

impl From<&PestError<Rule>> for SyntaxError {
    fn from(error: &PestError<Rule>) -> Self {
        let (start, end) = match error.location {
            pest::error::InputLocation::Pos(pos) => (pos, pos),
            pest::error::InputLocation::Span((start, end)) => (start, end),
        };
        SyntaxError {
            span: Span::new(start, end),
            message: error.variant.message().into_owned(),
        }
    }
}

/// A failure resolving the import graph rooted at one `parse_with_imports` call: an `%import`
/// directive whose target couldn't be resolved, an import cycle, or a file (reached directly or
/// transitively) that failed to parse.
///
/// Every variant keeps the failure's own underlying error structured — as the original
/// [MercError] rather than a message rendered into a `String` — so a caller with access to the
/// [SourceMap] (an LSP) can recover it via [MercError::downcast_ref] and build a precise
/// diagnostic, instead of re-parsing formatted text. [ImportError::syntax_error] does exactly
/// that for a parse failure, however deeply nested behind [ImportError::Unresolved] layers it is.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// A `%import` directive's target couldn't be loaded: the file is missing or unreadable,
    /// fails to parse ([ImportError::Parse]), or has an unresolved import of its own (another
    /// [ImportError], one level further down).
    #[error("cannot resolve %import \"{path}\": {cause}")]
    Unresolved {
        /// The relative path exactly as written in the directive (`directive.node.path`), not
        /// the path it was resolved against the importing file's directory to.
        path: String,
        /// Span of the failing directive's own quoted path, at the importing file's global
        /// (shared [SourceMap]) offset.
        span: Span,
        /// The underlying failure, kept as the original [MercError].
        cause: MercError,
    },

    /// A cycle of `%import` directives, each importing the next, with no acyclic root.
    #[error("import cycle detected:\n  {}", cycle.join("\n  imports "))]
    Cycle {
        /// The cyclic chain of canonicalized paths, outermost first, already rendered for
        /// display (a caller wanting the raw paths back would need to re-canonicalize).
        cycle: Vec<String>,
    },

    /// A file reached via `%import` (or the root file of the resolution itself) failed to parse.
    #[error("in {}:\n{cause}", path.display())]
    Parse {
        path: PathBuf,
        /// Structured location/message for the failure, already shifted into the shared, global
        /// offset space — see [ImportError::syntax_error].
        syntax_error: SyntaxError,
        /// The parser's own [MercError], kept for its `Display` (which additionally renders the
        /// offending source line) and for a caller that specifically wants the raw pest error.
        cause: MercError,
    },
}

impl ImportError {
    /// The span of the failing `%import` directive itself. `None` for [ImportError::Cycle] and
    /// [ImportError::Parse], neither of which is anchored to one particular directive.
    pub fn span(&self) -> Option<&Span> {
        match self {
            ImportError::Unresolved { span, .. } => Some(span),
            ImportError::Cycle { .. } | ImportError::Parse { .. } => None,
        }
    }

    /// If this failure was ultimately a parse failure — reached directly or through any number
    /// of nested [ImportError::Unresolved] layers — its location and message, already shifted
    /// into the shared, global offset space. `None` for a missing file, an I/O error, or an
    /// import cycle.
    pub fn syntax_error(&self) -> Option<&SyntaxError> {
        match self {
            ImportError::Parse { syntax_error, .. } => Some(syntax_error),
            ImportError::Unresolved { cause, .. } => {
                cause.downcast_ref::<ImportError>().and_then(ImportError::syntax_error)
            }
            ImportError::Cycle { .. } => None,
        }
    }
}

/// Wraps a parser failure at `path` (whose own text starts at `base` in the shared [SourceMap]
/// offset space) into an [ImportError::Parse], shifting the parser's [SyntaxError] out of
/// file-local coordinates the same way every other span produced while parsing that file is
/// shifted (see [crate::OffsetSpans]).
fn parse_error(path: &Path, base: usize, cause: MercError) -> MercError {
    let mut syntax_error = cause
        .downcast_ref::<PestError<Rule>>()
        .map(SyntaxError::from)
        .unwrap_or_else(|| SyntaxError {
            span: Span::default(),
            message: cause.to_string(),
        });
    syntax_error.span.shift(base);

    MercError::from(ImportError::Parse {
        path: path.to_path_buf(),
        syntax_error,
        cause,
    })
}

/// Scans `text` line by line for `%import "relative/path"` directives: a line,
/// once its leading and trailing whitespace is trimmed, of the exact shape
/// `%import "PATH"`.
///
/// Spans are local to `text`; a caller splicing multiple files together is
/// responsible for shifting them into the shared, [SourceMap]-wide offset
/// space, the same way every other span from that file's parse is.
pub fn scan_imports(text: &str) -> Vec<Spanned<ImportDirective>> {
    let mut directives = Vec::new();
    let mut offset = 0;

    for line in text.split_inclusive('\n') {
        let leading_whitespace = line.len() - line.trim_start().len();
        let trimmed = line.trim();
        if let Some(directive) = parse_import_line(trimmed, offset + leading_whitespace) {
            // The span covers the whole line, trailing newline excluded, so a rendered error
            // underlines the entire directive rather than just the path.
            let end = offset + line.trim_end_matches('\n').len();
            directives.push(Spanned::new(directive, Span::new(offset, end)));
        }
        offset += line.len();
    }

    directives
}

/// Recognizes one already-trimmed line as `%import "PATH"`, with no trailing content after the
/// closing quote. `base` is `trimmed`'s own offset within the file being scanned, used to give
/// [ImportDirective::path_span] an absolute (file-local) span rather than one relative to
/// `trimmed`.
fn parse_import_line(trimmed: &str, base: usize) -> Option<ImportDirective> {
    let rest = trimmed.strip_prefix("%import")?;
    // Require at least one whitespace character between the keyword and the opening quote, so
    // `%importance` is not misparsed as a directive.
    let rest = rest.strip_prefix(char::is_whitespace)?.trim_start();
    let path = rest.strip_prefix('"')?.strip_suffix('"')?;

    if path.is_empty() || path.contains('"') {
        return None;
    }

    // `rest` still ends exactly where `trimmed` does, so its start is the byte offset of the
    // opening quote within `trimmed`; the path text itself starts one byte past that.
    let quote_offset = trimmed.len() - rest.len();
    let path_start = base + quote_offset + 1;
    let path_end = path_start + path.len();

    Some(ImportDirective {
        path: path.to_string(),
        path_span: Span::new(path_start, path_end),
    })
}

/// Implemented by every untyped AST that `%import` can compose.
trait ImportMergeable: Sized + OffsetSpans {
    /// Parses one file's complete text, at its own zero-based offsets — [`OffsetSpans::offset_spans`]
    /// rebases the result into the shared space afterwards, so this never sees padded text.
    fn parse_own_text(text: &str) -> Result<Self, MercError>;

    /// Merges an *imported* file's declarations into `self`, ahead of anything `self` already
    /// holds. Used for every file reached via a `%import` directive, however deeply nested.
    fn merge_imported(&mut self, other: &Self);

    /// As [Self::merge_imported], but for the root file's own parse.
    ///
    /// Can be used to merge the root file's own declarations in the same way as
    /// imported files.
    fn merge_own(&mut self, other: &Self) {
        self.merge_imported(other);
    }
}

impl ImportMergeable for UntypedDataSpecification {
    fn parse_own_text(text: &str) -> Result<Self, MercError> {
        UntypedDataSpecification::parse(text)
    }

    fn merge_imported(&mut self, other: &Self) {
        self.merge(other);
    }
}

impl ImportMergeable for UntypedProcessSpecification {
    fn parse_own_text(text: &str) -> Result<Self, MercError> {
        UntypedProcessSpecification::parse(text)
    }

    fn merge_imported(&mut self, other: &Self) {
        self.merge(other);
    }

    fn merge_own(&mut self, other: &Self) {
        self.merge(other);
        self.init = other.init.clone();
    }
}

/// Depth-first import resolution state, threaded through one call to
/// [UntypedDataSpecification::parse_with_imports] (or another type's own `parse_with_imports`).
struct Resolver<'a, T> {
    sources: &'a mut SourceMap,
    /// Every file whose declarations have already been merged, by canonicalized path — so a
    /// diamond import (the same file reached from two different places in the tree) is merged
    /// once, not once per import site.
    merged: HashMap<PathBuf, SourceId>,
    /// The canonicalized paths currently being loaded, innermost last — a file reappearing in
    /// here (rather than just in `merged`) is a cycle, not a diamond.
    stack: Vec<PathBuf>,
    /// Every `%import` directive resolved so far, in load order — becomes [`ImportGraph::edges`].
    graph: Vec<(SourceId, Span, SourceId)>,
    _marker: std::marker::PhantomData<T>,
}

impl<'a, T: ImportMergeable> Resolver<'a, T> {
    /// Starts a fresh resolution against `sources`, with nothing loaded yet.
    fn new(sources: &'a mut SourceMap) -> Self {
        Resolver {
            sources,
            merged: HashMap::new(),
            stack: Vec::new(),
            graph: Vec::new(),
            _marker: std::marker::PhantomData,
        }
    }

    /// As [Self::load_with_text], with no explicit text override — `path` is read from disk.
    fn load(&mut self, path: &Path, output: &mut T) -> Result<SourceId, MercError> {
        self.load_with_text(path, None, output)
    }

    /// Loads `path`, merging its declarations into `output` ahead of anything
    /// `output` already holds, and returns the [SourceId] it was registered
    /// under. A file already merged earlier in this resolution is skipped
    /// rather than merged a second time.
    ///
    /// `text_override`, when given, is used as `path`'s own text instead of reading it from disk.
    fn load_with_text(
        &mut self,
        path: &Path,
        text_override: Option<&str>,
        output: &mut T,
    ) -> Result<SourceId, MercError> {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        if let Some(&id) = self.merged.get(&canonical) {
            return Ok(id);
        }

        if let Some(position) = self.stack.iter().position(|p| p == &canonical) {
            let cycle = self.stack[position..]
                .iter()
                .chain(std::iter::once(&canonical))
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>();
            return Err(MercError::from(ImportError::Cycle { cycle }));
        }

        // A file is the *root* of this resolution exactly when nothing is on the stack yet.
        let is_root = self.stack.is_empty();

        let source_id = match text_override {
            Some(text) => self.sources.add_text(path.display().to_string(), text.to_string()),
            None => self.sources.load_file(path)?,
        };
        // The file's text is registered — and so its base offset into the shared, global byte
        // space fixed — *before* it (or anything it imports) is parsed, which is what lets the
        // offset pass below rebase this file's own, zero-based spans into that space.
        let base = self.sources.base_offset(source_id);
        let text = self.sources.text(source_id).to_string();

        self.stack.push(canonical.clone());

        let directory = path.parent().unwrap_or_else(|| Path::new("."));
        for directive in scan_imports(&text) {
            let import_path = directory.join(&directive.node.path);
            let span = Span::new(base + directive.span.start, base + directive.span.end);
            let child_id = self.load(&import_path, output).map_err(|error| {
                MercError::from(ImportError::Unresolved {
                    path: directive.node.path.clone(),
                    span: span.clone(),
                    cause: error,
                })
            })?;
            self.graph.push((source_id, span, child_id));
        }

        let mut file_spec = T::parse_own_text(&text).map_err(|error| parse_error(path, base, error))?;
        file_spec.offset_spans(base);
        if is_root {
            output.merge_own(&file_spec);
        } else {
            output.merge_imported(&file_spec);
        }

        self.stack.pop();
        self.merged.insert(canonical, source_id);

        Ok(source_id)
    }
}

impl UntypedDataSpecification {
    /// Parses `root_path` and every data specification it (transitively)
    /// `%import`s, merging them all into one [UntypedDataSpecification].
    ///
    /// Every declaration keeps a [merc_utilities::Span] that renders correctly
    /// (see [merc_utilities::Span::render]) against the returned `sources`,
    /// whether it came from `root_path` or from something it imported.
    ///
    /// `sources` accumulates every file loaded this way; pass a fresh, empty
    /// [SourceMap] for a single call, or reuse one across several
    /// `parse_with_imports` calls so a file imported by more than one of them
    /// is still only loaded once.
    pub fn parse_with_imports(
        root_path: &Path,
        sources: &mut SourceMap,
    ) -> Result<(UntypedDataSpecification, ImportGraph), MercError> {
        let mut resolver: Resolver<UntypedDataSpecification> = Resolver::new(sources);
        let mut output = UntypedDataSpecification::default();
        let root = resolver.load(root_path, &mut output)?;
        Ok((
            output,
            ImportGraph {
                root,
                edges: resolver.graph,
            },
        ))
    }
}

impl UntypedProcessSpecification {
    /// As [UntypedDataSpecification::parse_with_imports], for a process specification: `root_path`
    /// and every process (or data) specification it (transitively) `%import`s are parsed and
    /// merged into one [UntypedProcessSpecification], with the same span/`sources`/cycle/diamond
    /// guarantees.
    ///
    /// `text` is `root_path`'s own text, used as-is rather than re-read from disk.
    pub fn parse_with_imports(
        root_path: &Path,
        text: &str,
        sources: &mut SourceMap,
    ) -> Result<(UntypedProcessSpecification, ImportGraph), MercError> {
        let mut resolver: Resolver<UntypedProcessSpecification> = Resolver::new(sources);
        let mut output = UntypedProcessSpecification::default();
        let root = resolver.load_with_text(root_path, Some(text), &mut output)?;
        Ok((
            output,
            ImportGraph {
                root,
                edges: resolver.graph,
            },
        ))
    }
}

impl UntypedStateFrmSpec {
    /// Parses `root_path` as a modal (mu-calculus) state-formula specification,
    /// resolving every `%import` directive found in its own text against
    /// process specifications rather than other formula files.
    ///
    /// `text` is `root_path`'s own text, used as-is rather than re-read from
    /// disk.
    pub fn parse_with_imports(
        root_path: &Path,
        text: &str,
        sources: &mut SourceMap,
    ) -> Result<(UntypedStateFrmSpec, ImportGraph), MercError> {
        let root_id = sources.add_text(root_path.display().to_string(), text.to_string());
        // Registered (and so base-offset-fixed) before anything it imports is parsed, same
        // offset-rebasing precondition `Resolver::load` relies on for every other file kind.
        let base = sources.base_offset(root_id);
        let text = sources.text(root_id).to_string();

        let directory = root_path.parent().unwrap_or_else(|| Path::new("."));
        let mut resolver: Resolver<UntypedProcessSpecification> = Resolver::new(sources);
        let mut imported = UntypedProcessSpecification::default();
        for directive in scan_imports(&text) {
            let import_path = directory.join(&directive.node.path);
            let span = Span::new(base + directive.span.start, base + directive.span.end);
            let child_id = resolver.load(&import_path, &mut imported).map_err(|error| {
                MercError::from(ImportError::Unresolved {
                    path: directive.node.path.clone(),
                    span: span.clone(),
                    cause: error,
                })
            })?;
            resolver.graph.push((root_id, span, child_id));
        }
        let edges = resolver.graph;

        let mut spec = UntypedStateFrmSpec::parse(&text).map_err(|error| parse_error(root_path, base, error))?;
        spec.offset_spans(base);

        // `imported` was built the same way `Resolver::load` builds up a file's own accumulator.
        imported.data_specification.merge(&spec.data_specification);
        imported
            .action_declarations
            .extend_from_slice(&spec.action_declarations);
        spec.data_specification = imported.data_specification;
        spec.action_declarations = imported.action_declarations;

        Ok((spec, ImportGraph { root: root_id, edges }))
    }
}
