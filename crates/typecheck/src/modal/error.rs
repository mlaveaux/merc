use merc_syntax::SourceMap;
use merc_syntax::Span;

use crate::InferenceError;
use crate::WellTypedError;

/// An error type checking a whole state formula with [`crate::ModalSpecification`]: an `act`
/// declaration, a fixpoint variable, or the formula itself that doesn't type check, on top of
/// everything [`WellTypedError`]/[`InferenceError`] already cover for the data-specification
/// subtree.
///
/// `#[non_exhaustive]`: more variants may be added, so a caller matching on this needs a catch-all
/// arm.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum ModalError {
    /// An error resolving or checking the data specification itself, or an `act`/fixpoint-variable
    /// parameter sort against it.
    #[error(transparent)]
    WellTyped(#[from] WellTypedError),
    /// A sort-inference error checking a real-valued `val(...)` state-formula expression, a
    /// boolean-valued `val(...)` action-formula expression, an action argument, a fixpoint
    /// variable's initial value or reference argument, or a `forall`/`exists`/`inf`/`sup`/`sum`
    /// binder.
    #[error(transparent)]
    Inference(#[from] InferenceError),

    #[error("an anonymous structured sort is not allowed in an action declaration")]
    AnonymousStructInDeclaration { span: Span },

    #[error("the parameter '{name}' of fixpoint variable '{variable}' is declared more than once")]
    DuplicateFixedPointParameter { variable: String, name: String, span: Span },

    #[error("no fixpoint variable named '{name}' is in scope here")]
    UndeclaredStateVariable { name: String, span: Span },
    #[error("'{name}' expects {expected} argument(s), found {found}")]
    ArityMismatch {
        name: String,
        expected: usize,
        found: usize,
        span: Span,
    },

    #[error("no action named '{name}' takes {arity} argument(s)")]
    UndeclaredAction {
        name: String,
        arity: usize,
        span: Span,
        /// Every declared action name, regardless of arity.
        candidates: Vec<String>,
    },
    #[error("no overload of '{name}' accepts these arguments")]
    NoMatchingOverload {
        name: String,
        span: Span,
        #[source]
        cause: Box<ModalError>,
    },
    #[error("the use of action '{name}' is ambiguous between {count} declarations")]
    AmbiguousAction { name: String, count: usize, span: Span },

    /// A state-level `val(...)` failed to type check against both `Real` and `Bool`.
    #[error("this val(...) expression fits neither sort Real ({real_cause}) nor Bool ({bool_cause})")]
    NoMatchingValSort {
        span: Span,
        #[source]
        real_cause: Box<ModalError>,
        bool_cause: Box<ModalError>,
    },

    /// A `*`-multiplied sub-formula occurs in a formula declared plain boolean: the multiplier is
    /// inherently quantitative, so it's only allowed when the formula's type is `Real`.
    #[error("a constant multiplier ('*') is not allowed in a plain boolean formula")]
    ConstantMultiplyInBooleanFormula { span: Span },
}

impl ModalError {
    /// The span of the offending construct. Every variant either carries a span directly or
    /// delegates to one, so this is always `Some` except through a `WellTypedError::Custom`.
    pub fn span(&self) -> Option<&Span> {
        match self {
            ModalError::WellTyped(error) => error.span(),
            ModalError::Inference(error) => Some(error.span()),
            ModalError::AnonymousStructInDeclaration { span }
            | ModalError::DuplicateFixedPointParameter { span, .. }
            | ModalError::UndeclaredStateVariable { span, .. }
            | ModalError::ArityMismatch { span, .. }
            | ModalError::UndeclaredAction { span, .. }
            | ModalError::NoMatchingOverload { span, .. }
            | ModalError::AmbiguousAction { span, .. }
            | ModalError::NoMatchingValSort { span, .. }
            | ModalError::ConstantMultiplyInBooleanFormula { span } => Some(span),
        }
    }

    /// Renders this error's message, followed by a caret-annotated source snippet. `sources` must
    /// contain the original specification text this error was raised against.
    pub fn render(&self, sources: &SourceMap) -> String {
        match self.span() {
            Some(span) => format!("{self}\n{}", span.render(sources)),
            None => self.to_string(),
        }
    }
}
