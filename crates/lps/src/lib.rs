//! Native support for mCRL2's Linear Process Specification (`.lps`) binary
//! format: a reader (see [`io::read_lps_file`]) and a `merc_explore`
//! `LPS`/`Summand` implementation backed by `merc_enumerate`.
//!
//! See `docs/enumeration-crate-plan.md` (Phase 3, section 7) in the workspace
//! root for the design this crate implements.

mod explore;
mod io;
mod lps;
mod prelude;
mod terms;

pub use explore::ExploreLinearProcessSpecification;
pub use explore::ExploreSummand;
pub use explore::explore_lps;
pub use io::read_lps;
pub use io::read_lps_file;
pub use lps::ActionSummand;
pub use lps::DeadlockSummand;
pub use lps::LinearProcess;
pub use lps::LinearProcessSpecification;
pub use terms::Action;
pub use terms::ActionLabel;
pub use terms::Distribution;
pub use terms::LinearProcessInit;
