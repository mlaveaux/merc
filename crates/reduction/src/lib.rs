//! This is the reduction module.
//!
//! It provides various functionalities for reducing labeled transition systems (LTS).
//! The module includes different partitioning strategies, quotienting mechanisms,
//! and other techniques to simplify and analyze LTS.
//!
//!

mod block_partition;
mod compare;
mod indexed_partition;
mod quotient;
mod reduce;
mod scc_decomposition;
mod signature_refinement;
mod signatures;
mod simple_block_partition;
mod sort_topological;
mod weak_bisimulation;

pub use block_partition::*;
pub use compare::*;
pub use indexed_partition::*;
pub use quotient::*;
pub use reduce::*;
pub use scc_decomposition::*;
pub use signature_refinement::*;
pub use signatures::*;
pub use simple_block_partition::*;
pub use sort_topological::*;
pub use weak_bisimulation::*;
