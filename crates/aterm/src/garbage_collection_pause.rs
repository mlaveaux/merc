#![forbid(unsafe_code)]

use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

/// The number of [`GarbageCollectionPause`] guards that currently exist, in any thread.
static PAUSES: AtomicUsize = AtomicUsize::new(0);

/// Postpones automatic garbage collection in all threads for as long as it exists.
///
/// Intended for phases in which nearly every created term stays in use, such as reading a term
/// stream, where a collection would free almost nothing. Pauses nest, an explicit collection still
/// runs, and terms created during a pause do not count towards the next automatic collection.
#[must_use = "garbage collection resumes as soon as the pause is dropped"]
pub struct GarbageCollectionPause {
    _private: (),
}

impl GarbageCollectionPause {
    /// Pauses automatic garbage collection until the returned guard is dropped.
    pub fn new() -> Self {
        PAUSES.fetch_add(1, Ordering::Relaxed);
        Self { _private: () }
    }
}

impl Default for GarbageCollectionPause {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for GarbageCollectionPause {
    fn drop(&mut self) {
        PAUSES.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Returns true iff a [`GarbageCollectionPause`] exists. This only postpones collections, so a
/// stale value is harmless and no ordering with other memory is needed.
pub(crate) fn is_garbage_collection_paused() -> bool {
    PAUSES.load(Ordering::Relaxed) != 0
}

#[cfg(test)]
mod tests {
    use super::GarbageCollectionPause;
    use super::is_garbage_collection_paused;

    #[test]
    fn test_pauses_nest() {
        // Other tests in this process may hold a pause, so only check what this test controls.
        let outer = GarbageCollectionPause::new();
        let inner = GarbageCollectionPause::new();
        drop(inner);
        assert!(is_garbage_collection_paused());
        drop(outer);
    }
}
