#![doc = include_str!("../README.md")]

mod alphabet;
mod compositional;
mod error;
mod print;
mod specification;
mod translate;

pub use compositional::CompositionalComponent;
pub use compositional::CompositionalSpecification;
pub use compositional::Manifest;
pub use compositional::ManifestComponent;
pub use compositional::linearise_compositional;
pub use error::LinearisationError;
pub use specification::ActionInstance;
pub use specification::ActionSummand;
pub use specification::DeadlockSummand;
pub use specification::LinearProcessSpecification;
pub use translate::linearise;
