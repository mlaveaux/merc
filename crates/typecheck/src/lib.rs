mod builtins;
mod checking;
mod data_specification;
mod inference;
mod ir;
mod lowering;
mod modal;
mod number_encoding;
mod pbes;
mod pres;
mod process;
mod resolution;
mod signature;
mod typing_info;

// The internal passes are flattened to the crate root for convenience; their
// exact module is not part of the interface. Only the items below marked `pub`
// are exposed outside the crate.
pub(crate) use builtins::*;
pub(crate) use data_specification::*;
pub(crate) use inference::*;
#[allow(unused_imports)]
pub(crate) use ir::*;
#[allow(unused_imports)]
pub(crate) use lowering::*;
pub(crate) use resolution::*;
#[allow(unused_imports)]
pub(crate) use signature::*;

pub use data_specification::DataSpecification;
pub use data_specification::TypedDataSpecification;
pub use inference::InferenceError;
pub use modal::Equation;
pub use modal::FormulaType;
pub use modal::ModalEquationSystem;
pub use modal::ModalError;
pub use modal::ModalSpecification;
pub use number_encoding::NumberEncoding;
pub use pbes::PbesError;
pub use pbes::PbesSpecification;
pub use pres::PresError;
pub use pres::PresSpecification;
pub use process::ProcessError;
pub use process::ProcessSpecification;
pub use process::disambiguate_process_specification;
pub use signature::WellTypedError;
pub use typing_info::ResolvedName;
pub use typing_info::TypedNode;
pub use typing_info::TypingInfo;
pub(crate) use typing_info::declared_span;
