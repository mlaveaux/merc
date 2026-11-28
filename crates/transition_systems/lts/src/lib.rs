#![doc = include_str!("../README.md")]

mod incoming_transitions;
mod io;
mod io_aut;
mod io_aut_stream;
mod io_bcg;
mod io_lts;
mod io_lts_stream;
mod labelled_transition_system;
mod lts;
mod lts_builder;
mod multi_action;
mod per_state_dedup;
mod random_lts;
mod reachability;

// Explicit pub(crate) re-exports.
pub(crate) use io_aut::AUT_TAU_LABEL;
pub(crate) use io_aut::MCRL2_TAU_LABEL;
pub(crate) use io_bcg::read_bcg;

// Public API
pub use incoming_transitions::FromTransition;
pub use incoming_transitions::IncomingTransitions;
pub use io::GenericLts;
pub use io::LtsFormat;
pub use io::apply_slice;
pub use io::guess_lts_format_from_extension;
pub use io::guess_lts_output_format;
pub use io::read_explicit_lts;
pub use io_aut::read_aut;
pub use io_aut::read_mcrl2_aut;
pub use io_aut::write_aut;
pub use io_aut::write_mcrl2_aut;
pub use io_aut_stream::AutFormat;
pub use io_aut_stream::AutStream;
pub use io_bcg::write_bcg;
pub use io_lts::LtsStateLabel;
pub use io_lts::LtsStateLabels;
pub use io_lts::read_lts;
pub use io_lts::write_lts;
pub use io_lts_stream::LtsStream;
pub use labelled_transition_system::LabelledTransitionSystem;
pub use labelled_transition_system::LtsMetrics;
pub use lts::AsGraph;
pub use lts::LTS;
pub use lts::LabelIndex;
pub use lts::StateIndex;
pub use lts::StateLabels;
pub use lts::Transition;
pub use lts::TransitionLabel;
pub use lts_builder::BuilderIndex;
pub use lts_builder::ConcurrentLtsBuilder;
pub use lts_builder::LtsBuilder;
pub use lts_builder::LtsBuilderMem;
pub use lts_builder::MutexLtsBuilder;
pub use multi_action::LtsAction;
pub use multi_action::LtsMultiAction;
pub use multi_action::SimpleAction;
pub use per_state_dedup::PerStateDedup;
pub use random_lts::mutate_lts;
pub use random_lts::random_lts;
pub use reachability::reachability;
pub use reachability::reachable_lts;
