//!
//! This crate provides functionality for working with variability parity games.
//!
//! Authors: Maurice Laveaux, Sjef van Loo, Erik de Vink and Tim A.C. Willemse
//!

#![forbid(unsafe_code)]

mod cube_iter;
mod feature_transition_system;
mod modal_equation_system;
mod parity_games;
mod project;
mod random_bdd;
mod reachability;
mod translate;
mod variability_zielonka;
mod zielonka;

pub use cube_iter::*;
pub use feature_transition_system::*;
pub use modal_equation_system::*;
pub use parity_games::*;
pub use project::*;
pub use random_bdd::*;
pub use reachability::*;
pub use translate::*;
pub use variability_zielonka::*;
pub use zielonka::*;
