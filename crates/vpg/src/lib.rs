//!
//! This crate provides functionality for working with variability parity games.
//!
//! Authors: Maurice Laveaux, Sjef van Loo, Erik de Vink and Tim A.C. Willemse
//!

#![forbid(unsafe_code)]

mod cube_iter;
mod feature_transition_system;
mod fixpoint_equation_system;
mod io;
mod io_pg;
mod io_vpg;
mod parity_game;
mod player;
mod predecessors;
mod reachability;
mod translate;
mod variability_parity_game;
mod variability_predecessors;
mod variability_zielonka;
mod zielonka;

pub use cube_iter::*;
pub use feature_transition_system::*;
pub use fixpoint_equation_system::*;
pub use io::*;
pub use io_pg::*;
pub use io_vpg::*;
pub use parity_game::*;
pub use player::*;
pub use predecessors::*;
pub use reachability::*;
pub use translate::*;
pub use variability_parity_game::*;
pub use variability_predecessors::*;
pub use variability_zielonka::*;
pub use zielonka::*;
