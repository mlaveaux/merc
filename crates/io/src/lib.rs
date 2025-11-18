//! Utility types and functions related to IO for the mCRL3 toolset.
//!
//! Forbid unsafe code in this crate. If unsafe code is needed it should be in the `merc_unsafety` crate.
#![forbid(unsafe_code)]

mod bitstream;
mod line_iterator;
mod parse_numbers;
mod progress;
mod text_utility;

pub use bitstream::*;
pub use line_iterator::*;
pub use parse_numbers::*;
pub use progress::*;
pub use text_utility::*;
