#![forbid(unsafe_code)]

mod args;
mod bdd;
mod dependency_graph;
mod io;
mod ldd;
mod mince_variable_order;
mod summand_grouping;
mod symbolic_lps;
mod symbolic_lps_explore;
mod util;
mod variable_order;

#[cfg(test)]
mod random_symbolic_lts;
#[cfg(test)]
mod random_vector_set;

pub(crate) use bdd::*;
pub(crate) use dependency_graph::*;
pub(crate) use ldd::*;
pub(crate) use util::*;

#[cfg(test)]
pub(crate) use random_symbolic_lts::*;
#[cfg(test)]
pub(crate) use random_vector_set::*;

pub use args::BDD_CACHE_CAPACITY;
pub use args::BDD_NODE_CAPACITY;
pub use args::LDD_CACHE_CAPACITY;
pub use args::LDD_NODE_CAPACITY;
#[cfg(feature = "clap")]
pub use args::ExplorationArgs;
#[cfg(feature = "clap")]
pub use args::MaxIterationsArgs;
#[cfg(feature = "clap")]
pub use args::OxiddArgs;
#[cfg(feature = "clap")]
pub use args::ReorderArgs;
pub use bdd::CubeIterAll;
pub use bdd::FormatConfig;
pub use bdd::FormatConfigSet;
pub use bdd::SymbolicLtsBdd;
pub use bdd::bits_to_bdd;
pub use bdd::convert_symbolic_lts_bdd;
pub use bdd::minus;
pub use bdd::minus_edge;
pub use bdd::quotient_symbolic;
pub use bdd::random_bdd;
pub use bdd::reachability_bdd;
pub use bdd::refine_bisimulation;
pub use bdd::sigref_symbolic;
pub use dependency_graph::parse_compacted_dependency_graph;
pub use io::SymFormat;
pub use io::guess_format_from_extension;
pub use ldd::ExplorationStrategy;
pub use ldd::LddDisplay;
pub use ldd::LddDot;
pub use ldd::ReachabilityOptions;
pub use ldd::ReachabilityResult;
pub use ldd::SylvanTransitionGroup;
pub use ldd::SymbolicLTS;
pub use ldd::SymbolicLts;
pub use ldd::convert_symbolic_lts;
pub use ldd::fix_element;
pub use ldd::from_iter;
pub use ldd::iter;
pub use ldd::merge;
pub use ldd::reachability;
pub use ldd::reachability_with_callback;
pub use ldd::reachability_with_options;
pub use ldd::read_sylvan;
pub use ldd::read_symbolic_lts;
pub use ldd::write_symbolic_lts;
pub use mince_variable_order::reorder;
pub use summand_grouping::ReadWritePattern;
pub use summand_grouping::SummandGrouping;
pub use summand_grouping::print_read_write_patterns;
pub use summand_grouping::print_transition_groups;
pub use summand_grouping::transition_group_pattern;
pub use symbolic_lps::SummandGroup;
pub use symbolic_lps::SymbolicLPS;
pub use symbolic_lps::TransitionGroup;
pub use symbolic_lps_explore::SymbolicLps;
pub use symbolic_lps_explore::SymbolicLpsOptions;
pub use util::LddLenCache;
pub use util::SatCount;
pub use util::SatCountCache;
pub use util::approx_satcount;
pub use util::element_of;
pub use util::height;
pub use util::ldd_len;
pub use variable_order::Order;
pub use variable_order::VariableOrder;
#[cfg(feature = "clap")]
pub use variable_order::parse_order;
