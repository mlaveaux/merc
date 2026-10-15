//! Errors from whole-PBES-specification type checking ([`crate::PbesSpecification`]).

use merc_syntax::SourceMap;
use merc_syntax::Span;

use crate::InferenceError;
use crate::WellTypedError;

/// An error type checking a whole PBES specification: a global variable, equation, or `init`
/// declaration that doesn't type check, on top of everything [`WellTypedError`]/[`InferenceError`]
/// already cover for the data-specification subtree.
///
/// `#[non_exhaustive]`, as [`crate::ProcessError`]: more variants may be added, so a caller
/// matching on this needs a catch-all arm.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum PbesError {
    /// An error resolving or checking the data specification itself, or a `glob`/equation-parameter
    /// sort against it.
    #[error(transparent)]
    WellTyped(#[from] WellTypedError),
    /// A sort-inference error checking a `val(...)` expression, a `PropVarInst` argument, or a
    /// quantifier binder.
    #[error(transparent)]
    Inference(#[from] InferenceError),

    #[error("an anonymous structured sort is not allowed in a PBES declaration")]
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

impl PbesError {
    /// The span of the offending construct. Mirrors [`crate::ProcessError::span`] — every variant
    /// here ultimately delegates to or carries a span directly, always `Some` except through a
    /// `WellTypedError::Custom`.
    pub fn span(&self) -> Option<&Span> {
        match self {
            PbesError::WellTyped(error) => error.span(),
            PbesError::Inference(error) => Some(error.span()),
            PbesError::AnonymousStructInDeclaration { span }
            | PbesError::DuplicateEquationParameter { span, .. }
            | PbesError::DuplicateGlobalVariable { span, .. }
            | PbesError::DuplicatePropositionalVariable { span, .. }
            | PbesError::UndeclaredPropositionalVariable { span, .. }
            | PbesError::ArityMismatch { span, .. } => Some(span),
        }
    }

    /// Renders this error's message, followed by a caret-annotated source snippet, the same way
    /// [`crate::ProcessError::render`] does. `sources` must contain the original specification
    /// text this error was raised against.
    pub fn render(&self, sources: &SourceMap) -> String {
        match self.span() {
            Some(span) => format!("{self}\n{}", span.render(sources)),
            None => self.to_string(),
        }
    }
}
