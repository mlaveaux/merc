//! A thread-safe library to manipulate first-order terms (represented by the
//! main [ATerm] data type).
//!
//! An first-order term is defined by the following grammar:
//!
//!     t := c | f(t1, ..., tn) | u64
//!
//! where `f` is a function symbol with arity `n > 0` and a unique name, `c` is
//! a constant and `u64` is a numerical term.
//!
//! Terms are stored maximally shared in the global aterm pool, meaning that
//! arguments `t1, ..., tn` are shared between all terms. Terms are generally
//! immutable, but can be created concurrently in different threads, using
//! thread-local data structures for their protection. The global aterm pool
//! performs garbage collection to remove terms that are no longer reachable.
//!
//! This crate does use `unsafe` for some of the more intricrate parts of the
//! ATerm library, but every module that only uses safe Rust is marked with
//! `#![forbid(unsafe_code)]`. This crate is a full reimplementation of the
//! ATerm library used in the [mCRL2](https://mcrl2.org) toolset.
//!
//! # Citations
//!
//! Further details on the implementation are explained in the following paper:
//!
//! "Using the Parallel ATerm Library for Parallel Model Checking and State
//! Space Generation". Jan Friso Groote, Kevin H.J. Jilissen, Maurice Laveaux,
//! Flip van Spaendonck. [DOI](https://doi.org/10.1007/978-3-031-15629-8_16).
//!
//! The initial ATerm library was inspired by:
//!
//! "Efficient annotated terms". M. G. J. van den Brand, H. A. de Jong, P.
//! Klint, P. A. Olivier.
//! [DOI](https://doi.org/10.1002/(SICI)1097-024X(200003)30:3<259::AID-SPE298>3.0.CO;2-Y).
mod aterm;
mod aterm_binary_stream;
mod aterm_builder;
mod aterm_int;
mod aterm_list;
mod aterm_storage;
mod aterm_string;
mod gc_mutex;
mod global_aterm_pool;
mod markable;
mod parse_term;
mod protected;
mod random_term;
mod shared_term;
mod symbol;
mod symbol_pool;
mod thread_aterm_pool;
mod transmutable;

pub use aterm::*;
pub use aterm_binary_stream::*;
pub use aterm_builder::*;
pub use aterm_int::*;
pub use aterm_list::*;
pub(crate) use aterm_storage::*;
pub use aterm_string::*;
pub use global_aterm_pool::*;
pub use markable::*;
pub use parse_term::*;
pub use protected::*;
pub use random_term::*;
pub use shared_term::*;
pub use symbol::*;
pub use symbol_pool::*;
pub use thread_aterm_pool::*;
pub use transmutable::*;
