#![doc = include_str!("../README.md")]

mod data;
mod error;
mod linearise;
mod passes;
mod process;

pub use error::LinearisationError;
pub use linearise::LinearisationOptions;
pub use linearise::linearise;
pub use linearise::rewriter;
