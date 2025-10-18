//! This is the reduction module.
//!
//! It provides various functionalities for reducing labeled transition systems (LTS).
//! The module includes different partitioning strategies, quotienting mechanisms,
//! and other techniques to simplify and analyze LTS.
//!
//!

mod block_partition;
mod indexed_partition;
mod quotient;
mod scc_decomposition;
mod signature_refinement;
mod signatures;
mod sort_topological;

pub use block_partition::*;
pub use indexed_partition::*;
pub use quotient::*;
pub use scc_decomposition::*;
pub use signature_refinement::*;
pub use signatures::*;
pub use sort_topological::*;