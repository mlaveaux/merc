//! This crate offers parsing for REC files and test cases for the rewrite
//! engines.
//!
//! The Rewrite Engine Competition (REC) is a
//! [competition](https://doi.org/10.1007/978-3-030-17502-3_6) for benchmarking
//! (first-order) term rewriting systems. This crate is used to perform these
//! benchmarks with our [`merc_sabre`] rewrite engine.
//!
//! This crate does not use any unsafe code.

#![forbid(unsafe_code)]

mod parse_rec;
mod syntax;

pub use parse_rec::load_rec_from_file;
pub use parse_rec::load_rec_from_strings;
