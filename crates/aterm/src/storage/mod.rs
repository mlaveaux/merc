//! Implementation of the [ATerm] related data structure.
//!
//! An aterm is a first-order term of the following form:
//!
//! t := c | f(t1, ..., tn) | u64
//!
//! where `f` is a function symbol with arity `n > 0` and a unique name, `c` is
//! a constant and `u64` is a numerical term.
//!
//! Terms are stored maximally shared in the global aterm pool, meaning that T1,
//! Tn are shared between all terms and the term is immutable. This global aterm
//! pool performs garbage collection to remove terms that are no longer
//! reachable. This is kept track of by the thread-local aterm pool.
//!
//! This crate does use `unsafe` for some of the more intricrate parts of the
//! ATerm library, but every module that only uses safe Rust is marked with
//! `#![forbid(unsafe_code)]`.

mod aterm_storage;
mod gc_mutex;
mod global_aterm_pool;
mod shared_term;
mod symbol_pool;
mod thread_aterm_pool;

pub(crate) use aterm_storage::*;
pub use gc_mutex::*;
pub use global_aterm_pool::*;
pub use shared_term::*;
pub use symbol_pool::*;
pub use thread_aterm_pool::*;
