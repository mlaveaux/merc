//!
//! This crate provides the raw Rust bindings for the libraries of the
//! [mCRL2](https://mcrl2.org/) toolset.
//!
//! Every module mirrors the corresponding library of the mCRL2 toolset. Within
//! it a foreign function interface (FFI) is defined using the
//! [cxx](https://cxx.rs/) crate.
//!
//! # Details
//!
//! Every type in a `ffi` module identifies a C++ type of the mCRL2 toolset.
//! Functions defined in the `ffi` module are wrappers around C++ functions.

pub mod atermpp;
pub mod data;
pub mod log;
pub mod pbes;

// Reexport the cxx types that we use
pub mod cxx {
    pub use cxx::CxxString;
    pub use cxx::CxxVector;
    pub use cxx::Exception;
    pub use cxx::UniquePtr;
}
