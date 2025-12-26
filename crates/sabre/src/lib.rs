//!
//! The Set Automaton Based Rewrite Engine (abbreviated Sabre) implements a
//! rewriter for conditional first-order non-linear rewrite rules, based on the
//! set automaton construction defined in this paper:
//! 
//! > "Term Rewriting Based On Set Automaton Matching". Mark Bouwman, Rick Erkens. [DOI](https://arxiv.org/abs/2202.08687).
//! 
//! This crate does not use unsafe code.

mod innermost_rewriter;
mod matching;
mod naive_rewriter;
mod rewrite_specification;
mod sabre_rewriter;
mod set_automaton;
pub mod utilities;

#[cfg(test)]
pub mod test_utility;

pub use innermost_rewriter::*;
pub use naive_rewriter::*;
pub use rewrite_specification::*;
pub use sabre_rewriter::*;
pub use set_automaton::*;
