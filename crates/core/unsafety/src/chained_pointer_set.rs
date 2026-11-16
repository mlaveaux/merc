use std::hash::BuildHasher;
use std::hash::Hash;
use std::ptr::NonNull;
use std::sync::atomic::AtomicPtr;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use rustc_hash::FxBuildHasher;

use crate::Erasable;
use crate::StablePointer;
use crate::BlockAllocatorSafe;
use crate::node_store::NodeStore;

/// The bucket array is grown when the nodes exceed the number of buckets
/// times this factor.
const MAX_LOAD: usize = 1;

/// A node of a chain; `value` comes first so that a pointer to the node is a
/// pointer to the value.
#[repr(C)]
pub struct ChainNode<T> {
    value: T,
    next: AtomicPtr<ChainNode<T>>,
}

// SAFETY: the first word of a node is the first word of `value` (`repr(C)`, `value` first).
unsafe impl<T: BlockAllocatorSafe> BlockAllocatorSafe for ChainNode<T> {}

/// Allocates `len` null buckets without touching the pages.
fn null_buckets<T>(len: usize) -> Box<[AtomicPtr<T>]> {
    use std::alloc::Layout;
    let layout = Layout::array::<AtomicPtr<T>>(len).expect("table too large");
    // SAFETY: `len` is at least one, so the layout is not empty.
    let ptr = unsafe { std::alloc::alloc_zeroed(layout) } as *mut AtomicPtr<T>;
    if ptr.is_null() {
        std::alloc::handle_alloc_error(layout);
    }
    // SAFETY: allocated with the layout of `[AtomicPtr<T>; len]`; all zero is
    // a null `AtomicPtr`.
    unsafe { Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len)) }
}

/// Every insertion whose hash has its top [`SAMPLE_BITS`] bits zero is counted, so
/// that the size can be estimated without a shared write on every insertion.
const SAMPLE_BITS: u32 = 6;

/// A lock-free hash set with chaining. Every bucket is the head of a singly
/// linked list of nodes. An insertion searches the chain of its bucket and,
/// when the value is absent, pushes a new node onto the head with a
/// compare-and-swap. When that fails, other threads pushed nodes in the
/// meantime: only those new nodes are searched for the value before retrying,
/// because the rest of the chain was already searched and never changes
/// concurrently. This is sound because nodes are only ever added concurrently;
/// removal happens through `&mut self`.
///
/// The nodes live in a [`NodeStore`], so they never move, and the bucket array
/// is rebuilt (and grown) only through `&mut self`
/// ([`ChainedPointerSet::retain`], [`ChainedPointerSet::grow`]). The set is
/// never full: an overdue grow only makes the chains longer.
pub struct ChainedPointerSet<T: Send + Sync + BlockAllocatorSafe, S = FxBuildHasher> {
    buckets: Box<[AtomicPtr<ChainNode<T>>]>,
    bucket_mask: usize,
    nodes: NodeStore<ChainNode<T>>,
    /// Number of nodes at the last rebuild.
    base_len: usize,
    /// Sampled insertions since the last rebuild, see [`SAMPLE_BITS`].
    sampled: AtomicUsize,
    hasher: S,
}

// SAFETY: as for `NodeStore`.
unsafe impl<T: Send + Sync + BlockAllocatorSafe, S: Sync> Sync for ChainedPointerSet<T, S> {}
unsafe impl<T: Send + Sync + BlockAllocatorSafe, S: Send> Send for ChainedPointerSet<T, S> {}

impl<T: Hash + Eq + Erasable + Send + Sync + BlockAllocatorSafe, S: BuildHasher>
    ChainedPointerSet<T, S>
{
    /// Creates a set with at least `buckets` buckets (a power of two).
    pub fn with_capacity_and_hasher(buckets: usize, hasher: S) -> Self {
        let buckets = buckets.max(1).next_power_of_two();
        Self {
            buckets: null_buckets(buckets),
            bucket_mask: buckets - 1,
            nodes: NodeStore::new(),
            base_len: 0,
            sampled: AtomicUsize::new(0),
            hasher,
        }
    }

    /// Number of buckets.
    pub fn bucket_capacity(&self) -> usize {
        self.buckets.len()
    }

    /// An estimate of the number of nodes, exact after `retain` and `grow`.
    pub fn len(&self) -> usize {
        self.base_len + (self.sampled.load(Ordering::Relaxed) << SAMPLE_BITS)
    }

    /// Returns true if the set has no nodes (as far as [`Self::len`] knows).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Number of nodes the set can take before it [needs to grow](Self::needs_grow).
    pub fn capacity(&self) -> usize {
        self.buckets.len() * MAX_LOAD
    }

    /// Whether the bucket array should be grown by the next `&mut self` operation.
    pub fn needs_grow(&self) -> bool {
        self.len() > self.capacity()
    }

    /// Inserts `value` if no equal node is present, and returns a pointer to
    /// the node equal to `value` and whether it was inserted.
    pub fn insert(&self, value: T) -> (StablePointer<T>, bool) {
        let (node, inserted) = self.insert_ptr(value);
        // SAFETY: nodes stay at their address until they are removed by
        // `retain`, and `value` is at offset 0 of the `repr(C)` node.
        (unsafe { StablePointer::from_ptr(node.cast::<T>()) }, inserted)
    }

    #[inline]
    fn insert_ptr(&self, value: T) -> (NonNull<ChainNode<T>>, bool) {
        let hash = self.hasher.hash_one(&value);
        let bucket = &self.buckets[hash as usize & self.bucket_mask];

        let mut head = bucket.load(Ordering::Acquire);
        if let Some(existing) = find(head, std::ptr::null_mut(), &value) {
            return (existing, false);
        }

        let node = self.nodes.alloc(ChainNode {
            value,
            next: AtomicPtr::new(head),
        });
        loop {
            // Release: a thread that reads the head with Acquire also sees the node.
            match bucket.compare_exchange(head, node.as_ptr(), Ordering::Release, Ordering::Acquire) {
                Ok(_) => {
                    if hash >> (64 - SAMPLE_BITS) == 0 {
                        self.sampled.fetch_add(1, Ordering::Relaxed);
                    }
                    return (node, true);
                }
                Err(current) => {
                    // Other threads pushed nodes in the meantime: check those.
                    // SAFETY: `node` is ours and not published.
                    let value = unsafe { &node.as_ref().value };
                    if let Some(existing) = find(current, head, value) {
                        // SAFETY: allocated above and never published.
                        unsafe { self.nodes.free_unpublished(node) };
                        return (existing, false);
                    }
                    head = current;
                    // SAFETY: not published, so only this thread accesses it.
                    unsafe { node.as_ref() }.next.store(head, Ordering::Relaxed);
                }
            }
        }
    }

    /// Removes every node for which `keep` returns false, and rebuilds the
    /// bucket array, growing it so that the remaining nodes fit at the
    /// maximum load. Returns the number of removed nodes.
    ///
    /// # Safety
    ///
    /// Pointers to removed nodes must not be used afterwards.
    pub unsafe fn retain(&mut self, mut keep: impl FnMut(&StablePointer<T>) -> bool) -> usize {
        let published = self.published();
        let before = published.len();
        // SAFETY: `published` holds every node once; by the caller's contract.
        let live = unsafe {
            self.nodes.retain(published, &mut |node| {
                // SAFETY: the node is alive during the call; see `insert`.
                keep(&StablePointer::from_ptr(node.cast::<T>()))
            })
        };
        let removed = before - live.len();
        self.rebuild(live);
        removed
    }

    /// Rebuilds the bucket array if [`Self::needs_grow`].
    pub fn grow(&mut self) {
        if self.needs_grow() {
            let published = self.published();
            self.rebuild(published);
        }
    }

    /// All nodes, by walking every chain.
    fn published(&mut self) -> Vec<NonNull<ChainNode<T>>> {
        let hint = self.len();
        collect_chains(&mut self.buckets, hint)
    }

    /// Relinks `nodes` into a bucket array with at least `nodes.len() / MAX_LOAD` buckets.
    fn rebuild(&mut self, nodes: Vec<NonNull<ChainNode<T>>>) {
        let wanted = (nodes.len() / MAX_LOAD).max(1).next_power_of_two();
        if wanted > self.buckets.len() {
            self.buckets = null_buckets(wanted);
            self.bucket_mask = wanted - 1;
        } else {
            for bucket in self.buckets.iter_mut() {
                *bucket.get_mut() = std::ptr::null_mut();
            }
        }
        self.base_len = nodes.len();
        *self.sampled.get_mut() = 0;

        for mut node in nodes {
            // SAFETY: `&mut self`: no other thread accesses the nodes.
            let node_ref = unsafe { node.as_mut() };
            let head = self.buckets[self.hasher.hash_one(&node_ref.value) as usize & self.bucket_mask].get_mut();
            *node_ref.next.get_mut() = *head;
            *head = node.as_ptr();
        }
    }
}

/// Collects the nodes of all chains.
fn collect_chains<T>(buckets: &mut [AtomicPtr<ChainNode<T>>], hint: usize) -> Vec<NonNull<ChainNode<T>>> {
    let mut nodes = Vec::with_capacity(hint);
    for bucket in buckets.iter_mut() {
        let mut node = *bucket.get_mut();
        while let Some(mut current) = NonNull::new(node) {
            nodes.push(current);
            // SAFETY: exclusive access; chains hold live nodes.
            node = *unsafe { current.as_mut() }.next.get_mut();
        }
    }
    nodes
}

impl<T: Send + Sync + BlockAllocatorSafe, S> Drop for ChainedPointerSet<T, S> {
    fn drop(&mut self) {
        let nodes = collect_chains(&mut self.buckets, 0);
        // SAFETY: the set is dropped, so no pointer to its nodes is used afterwards.
        unsafe { self.nodes.retain(nodes, &mut |_| false) };
    }
}

/// Searches the chain from `node` up to (excluding) `end` for `value`.
#[inline]
fn find<T: Eq>(mut node: *mut ChainNode<T>, end: *mut ChainNode<T>, value: &T) -> Option<NonNull<ChainNode<T>>> {
    while node != end {
        // SAFETY: published nodes stay initialised until the next `&mut self`
        // operation, and were read with Acquire (bucket heads and `next`
        // pointers are written before the node is published).
        let current = unsafe { &*node };
        if current.value == *value {
            // SAFETY: `node` is not null: chains end in null, and `end`, when
            // not null, is reachable from the start of the walk.
            return Some(unsafe { NonNull::new_unchecked(node) });
        }
        node = current.next.load(Ordering::Relaxed);
    }
    None
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    /// A test value; never `usize::MAX`, as `BlockAllocatorSafe` requires.
    #[derive(Hash, PartialEq, Eq, Debug)]
    struct V(u64);
    unsafe impl BlockAllocatorSafe for V {}

    /// Number of values in the tests; small under Miri, which is slow.
    const N: u64 = if cfg!(miri) { 600 } else { 10_000 };

    #[test]
    fn insert_retain_grow() {
        let mut set: ChainedPointerSet<V, FxBuildHasher> = ChainedPointerSet::with_capacity_and_hasher(8, FxBuildHasher);
        let mut pointers = Vec::new();
        for i in 0..N {
            let (p, inserted) = set.insert(V(i));
            assert!(inserted);
            pointers.push(unsafe { p.ptr() });
        }
        for i in 0..N {
            let (p, inserted) = set.insert(V(i));
            assert!(!inserted);
            assert_eq!(unsafe { p.ptr() }, pointers[i as usize]);
        }
        assert!(set.needs_grow());
        set.grow();
        assert_eq!(set.len(), N as usize);
        assert!(!set.needs_grow());
        let removed = unsafe { set.retain(|p| p.deref().0 % 2 == 0) };
        assert_eq!(removed, N as usize / 2);
        assert_eq!(set.len(), N as usize / 2);
        for i in (0..N).step_by(2) {
            let (p, inserted) = set.insert(V(i));
            assert!(!inserted, "{i}");
            assert_eq!(unsafe { p.ptr() }, pointers[i as usize], "nodes do not move");
        }
        for i in (1..N).step_by(2) {
            assert!(set.insert(V(i)).1);
        }
    }



    #[test]
    fn concurrent() {
        let set: Arc<ChainedPointerSet<V, FxBuildHasher>> = Arc::new(ChainedPointerSet::with_capacity_and_hasher(64, FxBuildHasher));
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let set = set.clone();
                std::thread::spawn(move || {
                    (0..2 * N)
                        .map(|i| {
                            let (p, inserted) = set.insert(V(i));
                            (unsafe { p.ptr() }.as_ptr() as usize, inserted)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        for i in 0..2 * N as usize {
            let inserted = results.iter().filter(|r| r[i].1).count();
            assert_eq!(inserted, 1, "exactly one thread inserts {i}");
            assert!(results.iter().all(|r| r[i].0 == results[0][i].0));
        }
    }


}
