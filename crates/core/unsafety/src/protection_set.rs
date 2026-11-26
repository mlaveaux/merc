use std::fmt;
use std::hash::Hash;
use std::mem::ManuallyDrop;
use std::ops::Deref;
use std::ops::Index;

use merc_utilities::GenerationCounter;
use merc_utilities::GenerationalIndex;

/// A type-safe index for the ProtectionSet to prevent accidental use of wrong indices
#[repr(transparent)]
#[derive(Copy, Clone, Default, PartialEq, Eq, Hash)]
pub struct ProtectionIndex(GenerationalIndex<usize>);

impl Deref for ProtectionIndex {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl fmt::Debug for ProtectionIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ProtectionIndex({:?})", self.0)
    }
}

impl fmt::Display for ProtectionIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A collection that assigns a unique index to every object added to it, and
/// allows removing objects while reusing their indices later. This is useful
/// for managing objects that must not be garbage collected, and as such it is
/// called a protection set.
#[derive(Default)]
pub struct ProtectionSet<T> {
    roots: Vec<Entry<T>>, // The set of root active nodes.
    free: Option<usize>,
    number_of_insertions: u64,
    size: usize,

    /// The number of generations
    generation_counter: GenerationCounter,
}

impl<T> ProtectionSet<T> {
    /// Creates a new empty protection set.
    pub fn new() -> Self {
        ProtectionSet {
            roots: Vec::new(),
            free: None,
            number_of_insertions: 0,
            size: 0,
            generation_counter: GenerationCounter::new(),
        }
    }

    /// Returns the number of insertions into the protection set.
    pub fn number_of_insertions(&self) -> u64 {
        self.number_of_insertions
    }

    /// Returns maximum number of active instances.
    pub fn maximum_size(&self) -> usize {
        self.roots.capacity()
    }

    /// Returns the number of roots in the protection set
    pub fn len(&self) -> usize {
        self.size
    }

    /// Returns whether the protection set is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Adds the given object to the protection set and returns its index.
    pub fn protect(&mut self, object: T) -> ProtectionIndex {
        self.number_of_insertions += 1;
        self.size += 1;

        let index = match self.free {
            Some(first) => {
                let next = unsafe { self.roots[first].next };
                if first == next {
                    // The list is empty as its first element points to itself.
                    self.free = None;
                } else {
                    // Update free to be the next element in the list.
                    self.free = Some(next);
                }

                self.roots[first] = Entry {
                    object: ManuallyDrop::new(object),
                };
                first
            }
            None => {
                // If free list is empty insert new entry into roots.
                self.roots.push(Entry {
                    object: ManuallyDrop::new(object),
                });

                self.roots.len() - 1
            }
        };

        ProtectionIndex(self.generation_counter.create_index(index))
    }

    /// Remove protection from the given object. Note that index must be the
    /// index returned by the [ProtectionSet::protect] call.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `index` refers to a currently protected
    /// entry and that it is not unprotected more than once.
    #[cfg_attr(kani, kani::requires(self.contains_root(index)))]
    #[cfg_attr(
        kani,
        kani::modifies(
            &self.size,
            &self.free,
            core::ptr::slice_from_raw_parts_mut(self.roots.as_mut_ptr(), self.roots.len())
        )
    )]
    pub unsafe fn unprotect(&mut self, index: ProtectionIndex) {
        let index = self.generation_counter.get_index(index.0);
        self.size -= 1;

        // SAFETY: `index` refers to an occupied entry. We must drop the stored
        // object before overwriting the union slot with freelist metadata.
        unsafe {
            ManuallyDrop::drop(&mut self.roots[index].object);
        }

        match self.free {
            Some(next) => {
                self.roots[index] = Entry { next };
            }
            None => {
                self.roots[index] = Entry { next: index };
            }
        };

        self.free = Some(index);

        // Postcondition: verify the object was correctly removed from protection
        debug_assert!(
            self.freelist_iter().any(|free_idx| free_idx == index),
            "Failed to unprotect object"
        );
    }

    /// Replaces the object at the given index with the new object.
    pub fn replace(&mut self, index: ProtectionIndex, object: T) {
        let index = self.generation_counter.get_index(index.0);

        debug_assert!(
            !self.freelist_iter().any(|free_idx| free_idx == index),
            "Index {index} does not point to a filled entry"
        );

        self.roots[index] = Entry {
            object: ManuallyDrop::new(object),
        };
    }

    /// Returns an iterator over all root indices in the protection set.
    ///
    /// This is an O(n) operation in the size of the freelist, so it should be
    /// used with care. It is intended for debugging and testing purposes.
    pub fn iter(&self) -> ProtSetIter<'_, T> {
        let mut free_indices: Vec<usize> = self.freelist_iter().collect();
        free_indices.sort_unstable();
        ProtSetIter {
            current: 0,
            protection_set: self,
            generation_counter: &self.generation_counter,
            free_indices,
        }
    }

    /// Returns an iterator over all free indices in the freelist.
    ///
    /// This is intended for use in `debug_assert!`s to verify that an entry is
    /// filled (not in the freelist) or free (in the freelist).
    fn freelist_iter(&self) -> FreeListIter<'_, T> {
        FreeListIter {
            current: self.free,
            protection_set: self,
        }
    }

    /// Returns whether the protection set contains the given index.
    ///
    /// This operation is O(n) in the size of the freelist.
    pub fn contains_root(&self, index: ProtectionIndex) -> bool {
        // A stale index (whose slot was reused) is not a live root; report it as
        // absent rather than panicking in `get_index`.
        if !self.generation_counter.is_valid(index.0) {
            return false;
        }

        let idx = self.generation_counter.get_index(index.0);
        !self.freelist_iter().any(|free_idx| free_idx == idx)
    }
}

impl<T> Drop for ProtectionSet<T> {
    fn drop(&mut self) {
        // Mark the free slots in a bitmap by walking the free list once, then
        // drop every slot that is not free. Testing membership against the free
        // list per slot instead would make this O(n^2), which for a protection
        // set grown to millions of slots effectively never finishes.
        //
        // A plain `Vec<bool>` is used rather than a bit-packed vector: at one byte
        // per slot it is negligible next to `roots` (which holds the `T` objects),
        // and it keeps this drop path free of the pointer tricks that trip Miri.
        let mut is_free = vec![false; self.roots.len()];
        for free_idx in self.freelist_iter() {
            is_free[free_idx] = true;
        }

        for (root, free) in self.roots.iter_mut().zip(is_free) {
            if free {
                continue;
            }

            // SAFETY: slots not in the freelist still contain a live protected
            // object that must be dropped.
            unsafe {
                ManuallyDrop::drop(&mut root.object);
            }
        }
    }
}

impl<T> Index<ProtectionIndex> for ProtectionSet<T> {
    type Output = T;

    fn index(&self, index: ProtectionIndex) -> &Self::Output {
        // Panics in debug builds when the generation is stale, i.e., the slot was freed
        // (and possibly reused) after the index was issued.
        let idx = self.generation_counter.get_index(index.0);
        debug_assert!(
            !self.freelist_iter().any(|free_idx| free_idx == idx),
            "Attempting to index free spot {}",
            index,
        );
        // SAFETY: The generational index ensures this slot was not freed after the index was issued.
        unsafe { &self.roots[idx].object }
    }
}

impl<T: fmt::Debug> fmt::Debug for ProtectionSet<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map().entries(self.iter()).finish()
    }
}

/// An entry in the protection set, which can either be filled with an object or
/// free and point to the next free entry.
union Entry<T> {
    object: ManuallyDrop<T>,

    // The next free entry in the free list. If this points to itself, the free list is empty.
    next: usize,
}

/// Check that the Entry is the same size as a usize; the union of
/// `ManuallyDrop<usize>` and `usize` is pointer-sized in every build.
const _: () = assert!(std::mem::size_of::<Entry<usize>>() == std::mem::size_of::<usize>());

/// An iterator over the filled entries in a protection set.
pub struct ProtSetIter<'a, T> {
    current: usize,
    protection_set: &'a ProtectionSet<T>,
    generation_counter: &'a GenerationCounter,
    /// Sorted free indices collected at iterator construction to skip free slots.
    free_indices: Vec<usize>,
}

impl<'a, T> Iterator for ProtSetIter<'a, T> {
    type Item = (ProtectionIndex, &'a T);

    fn next(&mut self) -> Option<Self::Item> {
        // Find the next valid entry, return it when found or None when end of roots is reached.
        while self.current < self.protection_set.roots.len() {
            let idx = self.current;
            self.current += 1;

            if self.free_indices.binary_search(&idx).is_err() {
                // SAFETY: idx is not in the free list, so it holds a valid filled object.
                let object = unsafe { &*self.protection_set.roots[idx].object };
                return Some((ProtectionIndex(self.generation_counter.recall_index(idx)), object));
            }
        }

        None
    }
}

/// An iterator over the free indices in the protection set freelist.
struct FreeListIter<'a, T> {
    current: Option<usize>,
    protection_set: &'a ProtectionSet<T>,
}

impl<'a, T> Iterator for FreeListIter<'a, T> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        let curr = self.current?;
        // SAFETY: curr is a valid index in the free list, so it stores a `next` pointer.
        let next = unsafe { self.protection_set.roots[curr].next };
        if next == curr {
            // Sentinel: last free entry points to itself.
            self.current = None;
        } else {
            self.current = Some(next);
        }
        Some(curr)
    }
}

impl<'a, T> IntoIterator for &'a ProtectionSet<T> {
    type Item = (ProtectionIndex, &'a T);
    type IntoIter = ProtSetIter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[cfg(kani)]
mod verification {
    use super::*;

    /// Checks the `unprotect` contract — `self.contains_root(index)` must
    /// hold at the call — against a symbolic-length set of live protections
    /// (1 to 3 slots) and a symbolic choice of which one to unprotect,
    /// instead of the single fixed slot the scenario proofs below use.
    #[kani::proof_for_contract(ProtectionSet::unprotect)]
    #[kani::unwind(4)]
    fn unprotect_contract_holds() {
        let mut ps: ProtectionSet<u32> = ProtectionSet::new();
        let mut indices: Vec<ProtectionIndex> = Vec::new();

        let count: usize = kani::any();
        kani::assume(count >= 1 && count <= 3);
        for _ in 0..count {
            indices.push(ps.protect(kani::any()));
        }

        let pick: usize = kani::any();
        kani::assume(pick < indices.len());
        let index = indices[pick];

        // SAFETY: every entry in `indices` is still live, satisfying the
        // `unprotect` contract checked below.
        unsafe {
            ps.unprotect(index);
        }
    }

    /// Covers the protect/index/replace/unprotect round trip and the empty-set
    /// pre/post conditions in a single harness.
    #[kani::proof]
    #[kani::unwind(3)]
    fn protection_set_roundtrip() {
        let mut ps: ProtectionSet<u32> = ProtectionSet::new();
        assert!(ps.is_empty());

        let v1: u32 = kani::any();
        let v2: u32 = kani::any();

        let idx = ps.protect(v1);
        assert_eq!(ps.len(), 1);
        assert!(ps.contains_root(idx));
        assert_eq!(ps[idx], v1);

        ps.replace(idx, v2);
        assert_eq!(ps[idx], v2);

        // SAFETY: `idx` is currently protected and is unprotected only once.
        unsafe {
            ps.unprotect(idx);
        }
        assert!(ps.is_empty());
        assert!(!ps.contains_root(idx));
        // The insertion counter is monotonic and is not affected by unprotect.
        assert_eq!(ps.number_of_insertions(), 1);
    }

    /// The generation counter must invalidate stale indices even when their
    /// underlying slot is reused — this is the load-bearing safety invariant.
    #[kani::proof]
    #[kani::unwind(3)]
    fn protection_set_stale_index_after_slot_reuse() {
        let mut ps: ProtectionSet<u32> = ProtectionSet::new();
        let v1: u32 = kani::any();
        let v2: u32 = kani::any();

        let i1 = ps.protect(v1);
        // SAFETY: `i1` is currently protected and is unprotected only once.
        unsafe {
            ps.unprotect(i1);
        }
        let i2 = ps.protect(v2);

        assert!(ps.contains_root(i2));
        assert!(!ps.contains_root(i1));
        assert_eq!(ps[i2], v2);
    }
}
