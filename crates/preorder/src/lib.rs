//!
//! Implements various (antichain) based preorder checks for labelled transition systems.
//!

#![forbid(unsafe_code)]

mod antichain;
mod failures_refinement;
mod preorder;

pub use antichain::*;
pub use failures_refinement::*;
pub use preorder::*;
