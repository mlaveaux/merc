use merc_process::ActionSetError;

/// An error linearising a `merc_process::ProcessSpecification`.
///
/// Processes and expressions are carried in their printed form, so that the error can be sent
/// between threads while terms cannot.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum LinearisationError {
    #[error("the specification has no 'init'; there is nothing to linearise")]
    MissingInit,

    #[error("the recursion through '{process}' is unguarded: it can call itself without performing an action first")]
    UnguardedRecursion { process: String },

    #[error(
        "the process is not in parallel pCRL format: '{construct}' occurs in '{expression}' inside a sequential or \
         alternative composition, but parallel composition and the operators on it are only allowed at the outer level"
    )]
    NotParallelPcrl {
        construct: &'static str,
        expression: String,
    },

    #[error(
        "linearisation generated more than {limit} processes for sequences of calls starting at '{process}'; the \
         process is probably not regular"
    )]
    NonRegular { process: String, limit: usize },

    #[error("the action name '{action}' is reserved for successful termination")]
    ReservedActionName { action: String },

    #[error(transparent)]
    InvalidActionSet(#[from] ActionSetError),

    #[error("'{construct}' is not supported by linearisation, found in '{expression}'")]
    UnsupportedConstruct {
        construct: &'static str,
        expression: String,
    },
}
