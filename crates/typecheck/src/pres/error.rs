//! Errors from whole-PRES-specification type checking ([`crate::PresSpecification`]).

use merc_syntax::SourceMap;
use merc_syntax::Span;

use crate::InferenceError;
use crate::WellTypedError;

/// An error type checking a whole PRES specification: a global variable, equation, or `init`
/// declaration that doesn't type check, on top of everything [`WellTypedError`]/[`InferenceError`]
/// already cover for the data-specification subtree.
///
/// `#[non_exhaustive]`, as [`crate::PbesError`]: more variants may be added, so a caller
/// matching on this needs a catch-all arm.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum PresError {
    /// An error resolving or checking the data specification itself, or a `glob`/equation-parameter
    /// sort against it.
    #[error(transparent)]
    WellTyped(#[from] WellTypedError),
    /// A sort-inference error checking a real-valued data expression embedded via `val(...)`, a
    /// `PropVarInst` argument, or an `inf`/`sup`/`sum` binder.
    #[error(transparent)]
    Inference(#[from] InferenceError),

    #[error("an anonymous structured sort is not allowed in a PRES declaration")]
    AnonymousStructInDeclaration { span: Span },

    #[error("the parameter '{name}' of equation '{equation}' is declared more than once")]
    DuplicateEquationParameter { equation: String, name: String, span: Span },
    #[error("the global variable '{name}' is declared more than once")]
    DuplicateGlobalVariable { name: String, span: Span },
    #[error("the propositional variable '{name}' is declared more than once")]
    DuplicatePropositionalVariable { name: String, span: Span },

    #[error("no propositional variable named '{name}' is declared")]
    UndeclaredPropositionalVariable {
        name: String,
        span: Span,
        /// Every declared propositional-variable name.
        candidates: Vec<String>,
    },
    #[error("'{name}' expects {expected} argument(s), found {found}")]
    ArityMismatch {
        name: String,
        expected: usize,
        found: usize,
        span: Span,
    },
}

impl PresError {
    /// The span of the offending construct. Mirrors [`crate::PbesError::span`] — every variant
    /// here ultimately delegates to or carries a span directly, always `Some` except through a
    /// `WellTypedError::Custom`.
    pub fn span(&self) -> Option<&Span> {
        match self {
            PresError::WellTyped(error) => error.span(),
            PresError::Inference(error) => Some(error.span()),
            PresError::AnonymousStructInDeclaration { span }
            | PresError::DuplicateEquationParameter { span, .. }
            | PresError::DuplicateGlobalVariable { span, .. }
            | PresError::DuplicatePropositionalVariable { span, .. }
            | PresError::UndeclaredPropositionalVariable { span, .. }
            | PresError::ArityMismatch { span, .. } => Some(span),
        }
    }

    /// Renders this error's message, followed by a caret-annotated source snippet, the same way
    /// [`crate::PbesError::render`] does. `sources` must contain the original specification text
    /// this error was raised against.
    pub fn render(&self, sources: &SourceMap) -> String {
        match self.span() {
            Some(span) => format!("{self}\n{}", span.render(sources)),
            None => self.to_string(),
        }
    }
}
