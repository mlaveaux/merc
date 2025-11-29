//!
//! This crate provides the raw Rust bindings for the libraries of the
//! [mCRL2](https://mcrl2.org/) toolset.
//!
//! Every module mirrors the corresponding library of the mCRL2 toolset. Within
//! it a foreign function interface (FFI) is defined using the
//! [cxx](https://cxx.rs/) crate.

pub mod pbes;

// Reexport the cxx types that we use
pub mod cxx {
    pub use cxx::Exception;
    pub use cxx::UniquePtr;
}
