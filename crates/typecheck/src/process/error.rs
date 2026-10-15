use merc_syntax::SourceMap;
use merc_syntax::Span;

use crate::InferenceError;
use crate::WellTypedError;

/// An error type checking a whole [`crate::ProcessSpecification`]: an action, process, or `init`
/// declaration that doesn't type check, on top of everything [`WellTypedError`]/[`InferenceError`]
/// already cover for the data-specification subtree.
///
/// `#[non_exhaustive]`: more variants may be added, so a caller matching on this needs a catch-all
/// arm.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    /// An error resolving or checking the data specification itself, or an `act`/`proc`/`glob`
    /// sort against it.
    #[error(transparent)]
    WellTyped(#[from] WellTypedError),
    /// A sort-inference error checking an action argument, a process-instantiation argument,
    /// or a `sum`/`dist` condition or time bound.
    #[error(transparent)]
    Inference(#[from] InferenceError),

    #[error("an anonymous structured sort is not allowed in an action, process, or global variable declaration")]
    AnonymousStructInDeclaration { span: Span },

    #[error("'{name}' is declared as both an action and a process")]
    ActionAndProcessConflict { name: String, span: Span },
    #[error("the parameter '{name}' of process '{process}' is declared more than once")]
    DuplicateProcessParameter { process: String, name: String, span: Span },
    #[error("the parameter '{name}' of process '{process}' is assigned more than once")]
    DuplicateAssignment { process: String, name: String, span: Span },
    #[error("the global variable '{name}' is declared more than once")]
    DuplicateGlobalVariable { name: String, span: Span },

    #[error("no action or process named '{name}' takes {arity} argument(s)")]
    UndeclaredActionOrProcess {
        name: String,
        arity: usize,
        span: Span,
        /// Every declared action and process name, regardless of arity.
        candidates: Vec<String>,
    },
    #[error("no overload of '{name}' accepts these arguments")]
    NoMatchingOverload {
        name: String,
        span: Span,
        #[source]
        cause: Box<ProcessError>,
    },
    #[error("the use of '{name}' is ambiguous between {count} declarations")]
    AmbiguousActionOrProcess { name: String, count: usize, span: Span },

    #[error("'{name}' is not a parameter of process '{process}'")]
    UnknownProcessParameter { process: String, name: String, span: Span },

    #[error("the action '{name}' is not declared")]
    UndeclaredAction {
        name: String,
        span: Span,
        /// Every declared action name, the same "did you mean" candidate list
        /// [`ProcessError::UndeclaredActionOrProcess`] carries.
        candidates: Vec<String>,
    },

    /// No way to pick one declared overload per action in a `comm` rule's
    /// left-hand side and one for its right-hand side makes every left-hand
    /// action's sort combine into something assignable to the right-hand
    /// action's declared sort.
    #[error("the communication '{lhs} -> {result}' cannot be type checked: {reason}")]
    IncompatibleCommunication {
        /// The left-hand actions, already joined with `|` (e.g. `"a|b"`).
        lhs: String,
        result: String,
        reason: String,
        span: Span,
    },
    /// The `rename` counterpart: no declared overload of `from` has a sort
    /// assignable to any declared overload of `to`.
    #[error("the rename '{from} -> {to}' cannot be type checked: {reason}")]
    IncompatibleRename {
        from: String,
        to: String,
        reason: String,
        span: Span,
    },
}

impl ProcessError {
    /// The span of the offending construct.
    pub fn span(&self) -> Option<&Span> {
        match self {
            ProcessError::WellTyped(error) => error.span(),
            ProcessError::Inference(error) => Some(error.span()),
            ProcessError::AnonymousStructInDeclaration { span }
            | ProcessError::ActionAndProcessConflict { span, .. }
            | ProcessError::DuplicateProcessParameter { span, .. }
            | ProcessError::DuplicateAssignment { span, .. }
            | ProcessError::DuplicateGlobalVariable { span, .. }
            | ProcessError::UndeclaredActionOrProcess { span, .. }
            | ProcessError::NoMatchingOverload { span, .. }
            | ProcessError::AmbiguousActionOrProcess { span, .. }
            | ProcessError::UnknownProcessParameter { span, .. }
            | ProcessError::UndeclaredAction { span, .. }
            | ProcessError::IncompatibleCommunication { span, .. }
            | ProcessError::IncompatibleRename { span, .. } => Some(span),
        }
    }

    /// Renders this error's message, followed by a caret-annotated source snippet, the same way
    /// [`WellTypedError::render`]/[`InferenceError::render`] do. `sources` must contain the
    /// original specification text this error was raised against.
    pub fn render(&self, sources: &SourceMap) -> String {
        match self.span() {
            Some(span) => format!("{self}\n{}", span.render(sources)),
            None => self.to_string(),
        }
    }
}
