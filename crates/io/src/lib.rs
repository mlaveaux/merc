//! Utility types and functions related to IO for the Merc toolset.
//!
//! Forbid unsafe code in this crate. If unsafe code is needed it should be in the `merc_unsafety` crate.
#![forbid(unsafe_code)]

mod bitstream;
mod line_iterator;
mod progress;
mod text_utility;

pub use bitstream::*;
pub use line_iterator::*;
pub use progress::*;
pub use text_utility::*;
