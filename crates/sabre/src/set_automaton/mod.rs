//! This module contains the code to construct a set automaton.
//!
//! The code is documented with the assumption that the reader knows how set automata work.
//! See <https://arxiv.org/abs/2202.08687> for a paper on the construction of set automata.
//! 
//! This module does not use unsafe code.
#![forbid(unsafe_code)]

mod automaton;
mod display;
mod match_goal;

pub use automaton::*;
pub(crate) use match_goal::*;

#[allow(unused)]
pub use display::*;
