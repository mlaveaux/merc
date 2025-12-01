//!
//! This crate provides functionality for working with variability parity games.
//!
//! Authors: Maurice Laveaux, Sjef van Loo, Erik de Vink and Tim A.C. Willemse
//!

#![forbid(unsafe_code)]

mod feature_transition_system;
mod io_pg;
mod io_vpg;
mod io;
mod parity_game;
mod predecessors;
mod reachability;
mod variability_parity_game;
mod variability_predecessors;
mod variability_zielonka;
mod zielonka;

pub use feature_transition_system::*;
pub use io_pg::*;
pub use io_vpg::*;
pub use io::*;
pub use parity_game::*;
pub use predecessors::*;
pub use reachability::*;
pub use variability_parity_game::*;
pub use variability_predecessors::*;
pub use variability_zielonka::*;
pub use zielonka::*;
