//! Various collections implemented for the Merc toolset.
//!
//! Forbid unsafe code in this crate. If unsafe code is needed it should be in the `merc_unsafety` crate.
#![forbid(unsafe_code)]

mod compressed_vec;
mod indexed_set;
mod macros;
mod protection_set;
mod vecset;

pub use compressed_vec::*;
pub use indexed_set::*;
pub use protection_set::*;
pub use vecset::*;
