//! Errors from [`crate::translate::linearise`] and
//! [`crate::compositional::linearise_compositional`].

use merc_syntax::Span;

/// An error linearising a `merc_typecheck::ProcessSpecification`.
///
/// `#[non_exhaustive]`: more variants will be added as later milestones (see
/// the crate README) lift the restrictions the current translator has.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum LinearisationError {
    #[error("the specification has no 'init'; there is nothing to linearise")]
    MissingInit,

    #[error(
        "'{name}' is called both directly and through another process; linearising more than one \
         mutually recursive process is not yet supported"
    )]
    MutualRecursionUnsupported { name: String, span: Span },

    #[error(
        "'{name}' is not called in tail position (it is followed by further behaviour via '.'); \
         only regular (tail-recursive) processes are supported"
    )]
    NotInTailPosition { name: String, span: Span },

    #[error("'{construct}' is not supported by this milestone of the linearisation crate yet")]
    UnsupportedConstruct { construct: &'static str, span: Span },

    #[error(
        "'|' is only supported directly between actions (to write a multi-action); combining \
         arbitrary behaviour with '|' needs full parallel-composition support, which is not \
         implemented yet"
    )]
    UnsupportedSynchronisation { span: Span },

    #[error(
        "every operand of the top-level '||' in a compositional specification must be a call to a \
         declared process (e.g. 'P1(x) || P2(y)'); this operand is something else"
    )]
    CompositionalOperandNotAProcessCall { span: Span },
}
