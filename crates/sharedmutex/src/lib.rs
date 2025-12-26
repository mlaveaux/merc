//!
//! Implements the busy-forbidden protocol, explained in the paper:
//!
//! > A Thread-Safe Term Library: (with a New Fast Mutual Exclusion Protocol). Jan Friso Groote, Maurice Laveaux, Flip van Spaendonck. [preprint](https://arxiv.org/pdf/2111.02706).
//!
//! Compared to the paper, the implementation is extended with (read) recursive locks.

mod bf_sharedmutex;
mod bf_vec;
mod recursive_lock;

pub use bf_sharedmutex::*;
pub use bf_vec::*;
pub use recursive_lock::*;
