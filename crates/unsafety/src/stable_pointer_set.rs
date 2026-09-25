use std::alloc::handle_alloc_error;
use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::BuildHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::ops::Deref;
use std::ptr::NonNull;
use std::ptr::addr_eq;
use std::ptr::slice_from_raw_parts_mut;
#[cfg(debug_assertions)]
use std::sync::Arc;

use allocator_api2::alloc::Allocator;
use allocator_api2::alloc::Global;
use allocator_api2::alloc::Layout;
use dashmap::DashSet;
use equivalent::Equivalent;

use crate::AllocatorDst;
use crate::Erasable;
use crate::SliceDst;
use crate::Thin;

/// A handle to an element stored in a [`StablePointerSet`].
///
/// The handle itself is inert: holding, comparing, hashing and copying it is
/// always sound. Reading the pointee, however, is only valid while the element
/// is still in its owning set, which the borrow checker does not track — so
/// dereferencing goes through the unsafe [`StablePointer::deref`].
///
/// Comparisons are based on the pointer's address, not the value it points to.
///
/// Deliberately not `Clone`: duplicating a handle extends the set of pointers
/// that must be kept valid, so duplication goes through the unsafe
/// [`StablePointer::copy`].
///
/// The pointer is stored type-erased so a handle is always a single machine
/// word, even when `T` is a slice DST whose native pointer would be wide. The
/// slice length is reconstructed from the pointee on deref via [`Erasable`].
#[repr(C)]
pub struct StablePointer<T: ?Sized + Erasable> {
    /// The thin, type-erased pointer to the element. Guaranteed non-null.
    ptr: Thin<T>,

    /// Keep track of reference counts in debug mode.
    #[cfg(debug_assertions)]
    reference_counter: Arc<()>,
}

/// Check that the Option<StablePointer> is the same size as a usize for release builds.
#[cfg(not(debug_assertions))]
const _: () = assert!(std::mem::size_of::<Option<StablePointer<usize>>>() == std::mem::size_of::<usize>());

impl<T: ?Sized + Erasable> StablePointer<T> {
    /// Returns true if this is the last reference to the pointer.
    fn is_last_reference(&self) -> bool {
        #[cfg(debug_assertions)]
        {
            // There is a reference in the table, and the one of `self.ptr`.
            Arc::strong_count(&self.reference_counter) == 2
        }
        #[cfg(not(debug_assertions))]
        {
            true
        }
    }

    /// Creates a new StablePointer from a raw pointer.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the pointer is valid and points to a valid T that outlives the StablePointer.
    pub unsafe fn from_ptr(ptr: NonNull<T>) -> Self {
        Self {
            ptr: Thin::new(ptr),
            #[cfg(debug_assertions)]
            reference_counter: Arc::new(()),
        }
    }

    /// Creates a new StablePointer from a raw pointer while preserving the
    /// debug reference counter from another StablePointer.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` points to the same allocation as
    /// `source` (potentially with a different pointee type/metadata) and
    /// remains valid for at least as long as any derived StablePointer.
    pub unsafe fn from_related_ptr<U: ?Sized + Erasable>(
        ptr: NonNull<U>,
        #[allow(unused)] source: &Self,
    ) -> StablePointer<U> {
        StablePointer {
            ptr: Thin::new(ptr),
            #[cfg(debug_assertions)]
            reference_counter: source.reference_counter.clone(),
        }
    }

    /// Returns public access to the underlying pointer.
    ///
    /// For a slice DST this reconstructs the wide pointer from the pointee, so
    /// it reads the header; prefer identity operations (eq/hash) that do not.
    pub fn ptr(&self) -> NonNull<T> {
        self.ptr.as_nonnull()
    }
}

impl<T: ?Sized + Erasable> PartialEq for StablePointer<T> {
    fn eq(&self, other: &Self) -> bool {
        // Identity is the erased address; the pointee is never read here.
        addr_eq(self.ptr.as_erased().as_ptr(), other.ptr.as_erased().as_ptr())
    }
}

impl<T: ?Sized + Erasable> Eq for StablePointer<T> {}

impl<T: ?Sized + Erasable> Ord for StablePointer<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.ptr.as_erased().cmp(&other.ptr.as_erased())
    }
}

impl<T: ?Sized + Erasable> PartialOrd for StablePointer<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: ?Sized + Erasable> Hash for StablePointer<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.ptr.as_erased().hash(state);
    }
}

// SAFETY: `ptr: Thin<T>` wraps an `ErasedPtr` (a `NonNull`), which is what blocks
// auto-`Send`/auto-`Sync` regardless of `T`; `reference_counter: Arc<()>` (debug builds only)
// is always `Send + Sync` on its own.
//
// Contract discharged by these impls: `StablePointer<T>` is an inert handle — holding, copying,
// comparing and hashing it never touches the pointee (see the type's own doc comment) — so
// `Send` needs only that the *handle* is safe to move to another thread, which holds
// unconditionally since moving it performs no access. `unsafe fn deref(&self) -> &T` is the one
// operation that reads the pointee, and its own `# Safety` contract already requires the caller
// to prove the element is still live in its owning set; given that, sharing `&StablePointer<T>`
// across threads to call `deref` concurrently is sound exactly when `T: Sync` licenses sharing
// the resulting `&T`. `T: Erasable` is required structurally (by `Thin<T>`), not for either bound.
unsafe impl<T: ?Sized + Erasable + Send> Send for StablePointer<T> {}
unsafe impl<T: ?Sized + Erasable + Sync> Sync for StablePointer<T> {}

impl<T: ?Sized + Erasable> StablePointer<T> {
    /// Returns a copy of the StablePointer.
    ///
    /// # Safety
    /// The caller must ensure the pointer points to a valid T that outlives the returned StablePointer.
    /// In particular the element must not be removed from the owning [`StablePointerSet`] (nor the set
    /// dropped) while the copy is alive, since [`Deref`] dereferences the pointer without any check.
    pub unsafe fn copy(&self) -> Self {
        Self {
            ptr: self.ptr,
            #[cfg(debug_assertions)]
            reference_counter: self.reference_counter.clone(),
        }
    }

    /// Creates a new StablePointer from a boxed element.
    fn from_entry(entry: &Entry<T>) -> Self {
        Self {
            ptr: Thin::new(entry.ptr),
            #[cfg(debug_assertions)]
            reference_counter: entry.reference_counter.clone(),
        }
    }

    /// Borrows the pointee.
    ///
    /// # Safety
    ///
    /// The element must still be present in its owning [`StablePointerSet`] (the
    /// set must not have been dropped, nor the element removed) for the duration
    /// of the returned borrow. This is not tracked by the borrow checker.
    pub unsafe fn deref(&self) -> &T {
        // SAFETY: the caller guarantees the pointee outlives the returned borrow.
        // `as_ref` reconstructs the wide pointer from the pointee's header.
        unsafe { self.ptr.as_ref() }
    }
}

impl<T: ?Sized + Erasable + SliceDst> StablePointer<T> {
    /// Reconstructs the wide pointer from a caller-supplied slice length instead
    /// of reading the pointee's header, as [`StablePointer::ptr`] does.
    ///
    /// This is the fast path for callers that already know the length (e.g. a
    /// compile-time-constant arity), avoiding the dependent header load.
    ///
    /// # Safety
    ///
    /// `len` must equal the pointee's [`SliceDst::length`]. Passing a larger
    /// value yields a pointer that reads out of bounds when dereferenced.
    pub unsafe fn ptr_with_len(&self, len: usize) -> NonNull<T> {
        let slice = slice_from_raw_parts_mut(self.ptr.as_erased().as_ptr().cast::<()>(), len);
        // SAFETY: the address originates from a `NonNull`, so it is non-null; the
        // caller guarantees `len` matches the pointee's slice length.
        T::retype(unsafe { NonNull::new_unchecked(slice) })
    }
}

impl<T: ?Sized + Erasable> fmt::Debug for StablePointer<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("StablePointer").field(&self.ptr.as_erased()).finish()
    }
}

/// A set that provides stable pointers to its elements.
///
/// Similar to `IndexedSet` but uses pointers instead of indices for direct access to elements.
/// Elements are stored in stable memory locations using a custom allocator, with the hash set maintaining references.
///
/// The set can use a custom hasher type for potentially better performance based on workload characteristics.
/// Uses an allocator for memory management, defaulting to the global allocator.
pub struct StablePointerSet<T: ?Sized, S = RandomState, A = Global>
where
    T: Hash + Eq + SliceDst + Erasable,
    S: BuildHasher + Clone,
    A: Allocator + AllocatorDst,
{
    index: DashSet<Entry<T>, S>,

    allocator: A,
}

impl<T: ?Sized> Default for StablePointerSet<T, RandomState, Global>
where
    T: Hash + Eq + SliceDst + Erasable,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<T: ?Sized> StablePointerSet<T, RandomState, Global>
where
    T: Hash + Eq + SliceDst + Erasable,
{
    /// Creates an empty StablePointerSet with the default hasher and global allocator.
    pub fn new() -> Self {
        Self {
            index: DashSet::default(),
            allocator: Global,
        }
    }

    /// Creates an empty StablePointerSet with the specified capacity, default hasher, and global allocator.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            index: DashSet::with_capacity_and_hasher(capacity, RandomState::new()),
            allocator: Global,
        }
    }
}

impl<T: ?Sized, S> StablePointerSet<T, S, Global>
where
    T: Hash + Eq + SliceDst + Erasable,
    S: BuildHasher + Clone,
{
    /// Creates an empty StablePointerSet with the specified hasher and global allocator.
    pub fn with_hasher(hasher: S) -> Self {
        Self {
            index: DashSet::with_hasher(hasher),
            allocator: Global,
        }
    }

    /// Creates an empty StablePointerSet with the specified capacity, hasher, and global allocator.
    pub fn with_capacity_and_hasher(capacity: usize, hasher: S) -> Self {
        Self {
            index: DashSet::with_capacity_and_hasher(capacity, hasher),
            allocator: Global,
        }
    }
}

impl<T: ?Sized, S, A> StablePointerSet<T, S, A>
where
    T: Hash + Eq + SliceDst + Erasable,
    S: BuildHasher + Clone,
    A: Allocator + AllocatorDst,
{
    /// Creates an empty StablePointerSet with the specified allocator and default hasher.
    pub fn new_in(allocator: A) -> Self
    where
        S: Default,
    {
        Self {
            index: DashSet::with_hasher(S::default()),
            allocator,
        }
    }

    /// Creates an empty StablePointerSet with the specified capacity, allocator, and default hasher.
    pub fn with_capacity_in(capacity: usize, allocator: A) -> Self
    where
        S: Default,
    {
        Self {
            index: DashSet::with_capacity_and_hasher(capacity, S::default()),
            allocator,
        }
    }

    /// Creates an empty StablePointerSet with the specified hasher and allocator.
    pub fn with_hasher_in(hasher: S, allocator: A) -> Self {
        Self {
            index: DashSet::with_hasher(hasher),
            allocator,
        }
    }

    /// Creates an empty StablePointerSet with the specified capacity, hasher, and allocator.
    pub fn with_capacity_and_hasher_in(capacity: usize, hasher: S, allocator: A) -> Self {
        Self {
            index: DashSet::with_capacity_and_hasher(capacity, hasher),
            allocator,
        }
    }

    /// Returns the number of elements in the set.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Returns true if the set is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns the capacity of the set.
    pub fn capacity(&self) -> usize {
        self.index.capacity()
    }

    /// Inserts an element into the set using an equivalent value.
    ///
    /// This version takes a reference to an equivalent value and creates the value to insert
    /// only if it doesn't already exist in the set. Returns a stable pointer to the element
    /// and a boolean indicating whether the element was inserted.
    pub fn insert_equiv<'a, Q>(&self, value: &'a Q) -> (StablePointer<T>, bool)
    where
        Q: Hash + Equivalent<T>,
        T: From<&'a Q>,
    {
        debug_assert!(std::mem::size_of::<T>() > 0, "Zero-sized types not supported");

        // Check if we already have this value
        let raw_ptr = self.get(value);

        if let Some(ptr) = raw_ptr {
            // We already have this value, return pointer to existing element
            return (ptr, false);
        }

        // Allocate memory for the value
        let layout = Layout::new::<T>();
        let ptr = self.allocator.allocate(layout).expect("Allocation failed").cast::<T>();

        // Write the value to the allocated memory
        unsafe {
            ptr.as_ptr().write(value.into());
        }

        // Insert new value using allocator
        let entry = Entry::new(ptr);
        let result = StablePointer::from_entry(&entry);

        // First add to storage, then to index
        let inserted = self.index.insert(entry);
        if !inserted {
            let entry = Entry::new(ptr);
            let element = self
                .index
                .get(&entry)
                .expect("Insertion failed, so entry must be in the set");

            // Call the drop function
            unsafe { std::ptr::drop_in_place(ptr.as_ptr()) };

            // Remove the entry we just created since it was not inserted
            unsafe {
                self.allocator.deallocate(ptr.cast(), layout);
            }

            return (StablePointer::from_entry(&element), false);
        }

        // Insertion succeeded.
        (result, true)
    }

    /// Returns `true` if the set contains a value.
    pub fn contains<Q>(&self, value: &Q) -> bool
    where
        T: Eq + Hash,
        Q: ?Sized + Hash + Equivalent<T>,
    {
        self.get(value).is_some()
    }

    /// Returns a stable pointer to a value in the set, if present.
    ///
    /// Searches for a value equal to the provided reference and returns a pointer to the stored element.
    /// The returned pointer remains valid until the element is removed from the set.
    pub fn get<Q>(&self, value: &Q) -> Option<StablePointer<T>>
    where
        T: Eq + Hash,
        Q: ?Sized + Hash + Equivalent<T>,
    {
        // Find the boxed element that contains an equivalent value
        let boxed = self.index.get(&LookUp(value))?;

        // SAFETY: The pointer is valid as long as the set is valid.
        let ptr = StablePointer::from_entry(boxed.key());
        Some(ptr)
    }

    /// Returns an iterator over the elements of the set.
    ///
    /// # Safety
    ///
    /// The yielded references borrow into the stable allocations. The caller
    /// must ensure no element is removed while a yielded reference is live.
    pub unsafe fn iter(&self) -> impl Iterator<Item = &T> {
        // SAFETY: each pointer is valid while its element remains in the set,
        // which the caller upholds per the contract above.
        self.index.iter().map(|boxed| unsafe { boxed.ptr.as_ref() })
    }

    /// Removes an element from the set using its stable pointer.
    ///
    /// Returns true if the element was found and removed.
    ///
    /// # Safety
    ///
    /// `pointer` must be the last [`StablePointer`] to the element; removal invalidates every
    /// remaining pointer to it, and dereferencing such a pointer afterwards is undefined
    /// behaviour. This is only checked in debug builds.
    pub unsafe fn remove(&self, pointer: StablePointer<T>) -> bool {
        debug_assert!(
            pointer.is_last_reference(),
            "Pointer must be the last reference to the element"
        );

        // SAFETY: This is the last reference to the element, so it is still
        // present in the set and safe to dereference.
        let t = unsafe { pointer.deref() };
        let result = self.index.remove(&LookUp(t));

        if let Some(ptr) = result {
            // SAFETY: We have exclusive access during drop and the pointer is valid
            unsafe {
                self.drop_and_deallocate_entry(ptr.ptr);
            }
            true
        } else {
            false
        }
    }

    /// Retains only the elements specified by the predicate, modifying the set in-place.
    ///
    /// The predicate closure is called with a mutable reference to each element and must
    /// return true if the element should remain in the set.
    ///
    /// # Safety
    ///
    /// It invalidates any StablePointers to removed elements; the caller must guarantee that no
    /// such pointer is dereferenced afterwards.
    pub unsafe fn retain<F>(&self, mut predicate: F)
    where
        F: FnMut(&StablePointer<T>) -> bool,
    {
        // First pass: determine what to keep/remove without modifying the collection
        self.index.retain(|element| {
            let ptr = StablePointer::from_entry(element);

            if !predicate(&ptr) {
                // Note that retain can remove disconnect graphs of elements in
                // one go, so it is not necessarily the case that there is only
                // one reference to the element.

                // SAFETY: We have exclusive access during drop and the pointer
                // is valid. `element.ptr` is the stored wide pointer, so no
                // header read is needed to recover the slice length.
                unsafe {
                    self.drop_and_deallocate_entry(element.ptr);
                }
                return false;
            }

            true
        });
    }

    /// Returns mutable access to the underlying allocator.
    pub fn allocator_mut(&mut self) -> &mut A {
        &mut self.allocator
    }

    /// Drops the element at the given pointer and deallocates its memory.
    ///
    /// # Safety
    ///
    /// This requires that ptr can be dereferenced, so it must point to a valid element.
    unsafe fn drop_and_deallocate_entry(&self, ptr: NonNull<T>) {
        // SAFETY: We have exclusive access during drop and the pointer is valid
        let length = unsafe { T::length(ptr.as_ref()) };
        unsafe {
            // Drop the value in place before deallocating
            std::ptr::drop_in_place(ptr.as_ptr());
        }
        self.allocator.deallocate_slice_dst(ptr, length);
    }
}

impl<T: ?Sized + SliceDst + Erasable, S, A> StablePointerSet<T, S, A>
where
    T: Hash + Eq,
    S: BuildHasher + Clone,
    A: Allocator + AllocatorDst + Sync,
{
    /// Clears the set, removing all values and invalidating all pointers.
    ///
    /// # Safety
    ///
    /// This is unsafe because it invalidates all pointers to the elements in the set; the caller
    /// must guarantee that none of them is dereferenced afterwards.
    pub unsafe fn clear(&self) {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.index.iter().all(|x| Arc::strong_count(&x.reference_counter) == 1),
            "All pointers must be the last reference to the element"
        );

        // Manually deallocate all entries before clearing
        for entry in self.index.iter() {
            // SAFETY: We have exclusive access during drop and the pointer is valid
            unsafe {
                self.drop_and_deallocate_entry(entry.ptr);
            }
        }

        self.index.clear();
        debug_assert!(self.index.is_empty(), "Index should be empty after clearing");
    }

    /// Inserts an element into the set using an equivalent value.
    ///
    /// This version takes a reference to an equivalent value and creates the
    /// value to insert only if it doesn't already exist in the set. Returns a
    /// stable pointer to the element and a boolean indicating whether the
    /// element was inserted.
    ///
    /// # Safety
    ///
    /// construct must fully initialize the value at the given pointer,
    /// otherwise it may lead to undefined behavior.
    pub unsafe fn insert_equiv_dst<'a, Q, C>(
        &self,
        value: &'a Q,
        length: usize,
        construct: C,
    ) -> (StablePointer<T>, bool)
    where
        Q: Hash + Equivalent<T>,
        C: Fn(*mut T, &'a Q),
    {
        // Check if we already have this value
        let raw_ptr = self.get(value);

        if let Some(ptr) = raw_ptr {
            // We already have this value, return pointer to existing element
            return (ptr, false);
        }

        // Allocate space for the entry and construct it
        let mut ptr = self
            .allocator
            .allocate_slice_dst::<T>(length)
            .unwrap_or_else(|_| handle_alloc_error(Layout::new::<()>()));

        unsafe {
            construct(ptr.as_mut(), value);
        }

        loop {
            let entry = Entry::new(ptr);
            let ptr = StablePointer::from_entry(&entry);

            let inserted = self.index.insert(entry);
            if !inserted {
                // Add the result to the storage, it could be at this point that the entry was inserted by another thread. So
                // this insertion might actually fail, in which case we should clean up the created entry and return the old pointer.

                // TODO: I suppose this can go wrong with begin_insert(x); insert(x); remove(x); end_insert(x) chain.
                if let Some(existing_ptr) = self.get(value) {
                    // SAFETY: We have exclusive access during drop and the pointer is valid.
                    unsafe {
                        self.drop_and_deallocate_entry(ptr.ptr.as_nonnull());
                    }

                    return (existing_ptr, false);
                }
            } else {
                // Value was successfully inserted
                return (ptr, true);
            }
        }
    }
}

impl<T, S, A> StablePointerSet<T, S, A>
where
    T: Hash + Eq + SliceDst + Erasable,
    S: BuildHasher + Clone,
    A: Allocator + AllocatorDst,
{
    /// Inserts an element into the set.
    ///
    /// If the set did not have this value present, `true` is returned along
    /// with a stable pointer to the inserted element.
    ///
    /// If the set already had this value present, `false` is returned along
    /// with a stable pointer to the existing element.
    pub fn insert(&self, value: T) -> (StablePointer<T>, bool) {
        debug_assert!(std::mem::size_of::<T>() > 0, "Zero-sized types not supported");

        if let Some(ptr) = self.get(&value) {
            // We already have this value, return pointer to existing element
            return (ptr, false);
        }

        let ptr = self
            .allocator
            .allocate(Layout::new::<T>())
            .unwrap_or_else(|_| handle_alloc_error(Layout::new::<T>()))
            .cast::<T>();

        unsafe {
            ptr.write(value);
        }

        // Insert new value using allocator
        let entry = Entry::new(ptr);
        let ptr = StablePointer::from_entry(&entry);

        // First add to storage, then to index
        let inserted = self.index.insert(entry);
        if !inserted {
            let entry = Entry::new(ptr.ptr());
            let element = self
                .index
                .get(&entry)
                .expect("Insertion failed, so entry must be in the set");

            // Drop and deallocate the allocation we created since it was not inserted.
            unsafe { std::ptr::drop_in_place(ptr.ptr().as_ptr()) };
            unsafe { self.allocator.deallocate(ptr.ptr().cast(), Layout::new::<T>()) };

            return (StablePointer::from_entry(&element), false);
        }

        (ptr, true)
    }
}

impl<T: ?Sized, S, A> Drop for StablePointerSet<T, S, A>
where
    T: Hash + Eq + SliceDst + Erasable,
    S: BuildHasher + Clone,
    A: Allocator + AllocatorDst,
{
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.index.iter().all(|x| Arc::strong_count(&x.reference_counter) == 1),
            "All pointers must be the last reference to the element"
        );

        // Manually drop and deallocate all entries
        for entry in self.index.iter() {
            unsafe {
                self.drop_and_deallocate_entry(entry.ptr);
            }
        }
    }
}

/// A helper struct to store the allocated element in the set.
///
/// Uses manual allocation instead of Box for custom allocator support.
/// Optionally stores a reference counter for debugging purposes in debug builds.
struct Entry<T: ?Sized> {
    /// Pointer to the allocated value
    ptr: NonNull<T>,

    #[cfg(debug_assertions)]
    reference_counter: Arc<()>,
}

// SAFETY: `ptr: NonNull<T>` is the field that blocks auto-`Send`/auto-`Sync`; the debug-only
// `reference_counter: Arc<()>` is always `Send + Sync` on its own. `Entry<T>` is the value the
// `DashSet<Entry<T>, S>` index stores, so a thread other than the one that inserted it may end
// up dropping it (`T: Send`) or reading it via `Deref`/hashing/equality when another thread's
// lookup walks the map's shards (`T: Sync`).
unsafe impl<T: ?Sized + Send> Send for Entry<T> {}
unsafe impl<T: ?Sized + Sync> Sync for Entry<T> {}

impl<T: ?Sized> Entry<T> {
    /// Creates a new entry by allocating memory for the value using the provided allocator.
    fn new(ptr: NonNull<T>) -> Self {
        Self {
            ptr,
            #[cfg(debug_assertions)]
            reference_counter: Arc::new(()),
        }
    }
}

impl<T: ?Sized> Deref for Entry<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The pointer is valid as long as the Entry exists
        unsafe { self.ptr.as_ref() }
    }
}

impl<T: PartialEq + ?Sized> PartialEq for Entry<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T: Hash + ?Sized> Hash for Entry<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

impl<T: Eq + ?Sized> Eq for Entry<T> {}

/// A helper struct to look up elements in the set using a reference.
#[derive(Hash, PartialEq, Eq)]
struct LookUp<'a, T: ?Sized>(&'a T);

impl<T: ?Sized, Q: ?Sized> Equivalent<Entry<T>> for LookUp<'_, Q>
where
    Q: Equivalent<T>,
{
    fn equivalent(&self, other: &Entry<T>) -> bool {
        self.0.equivalent(&**other)
    }
}

#[cfg(test)]
mod tests {
    use std::hash::BuildHasherDefault;
    use std::hash::Hash;
    use std::hash::Hasher;
    use std::hash::RandomState;

    use allocator_api2::alloc::System;
    use dashmap::Equivalent;
    use rustc_hash::FxHasher;

    use crate::StablePointerSet;

    #[test]
    fn test_insert_and_get() {
        let set = StablePointerSet::new();

        // Insert a value and ensure we get it back
        let (ptr1, inserted) = set.insert(42);
        assert!(inserted);
        // SAFETY: the elements live in `set` for the duration of the test.
        assert_eq!(unsafe { *ptr1.deref() }, 42);

        // Insert the same value and ensure we get the same pointer
        let (ptr2, inserted) = set.insert(42);
        assert!(!inserted);
        assert_eq!(unsafe { *ptr2.deref() }, 42);

        // Pointers to the same value should be identical
        assert_eq!(ptr1, ptr2);

        // Verify that we have only one element
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn test_contains() {
        let set = StablePointerSet::new();
        set.insert(42);
        set.insert(100);

        assert!(set.contains(&42));
        assert!(set.contains(&100));
        assert!(!set.contains(&200));
    }

    #[test]
    fn test_get() {
        let set = StablePointerSet::new();
        set.insert(42);
        set.insert(100);

        let ptr = set.get(&42).expect("Value should exist");
        // SAFETY: the elements live in `set` for the duration of the test.
        assert_eq!(unsafe { *ptr.deref() }, 42);

        let ptr = set.get(&100).expect("Value should exist");
        assert_eq!(unsafe { *ptr.deref() }, 100);

        assert!(set.get(&200).is_none(), "Value should not exist");
    }

    #[test]
    fn test_iteration() {
        let set = StablePointerSet::new();
        set.insert(1);
        set.insert(2);
        set.insert(3);

        // SAFETY: no element is removed while the iterator references are live.
        let mut values: Vec<i32> = unsafe { set.iter() }.copied().collect();
        values.sort();

        assert_eq!(values, vec![1, 2, 3]);
    }

    #[test]
    fn test_stable_pointer_set_insert_equiv_ref() {
        #[derive(PartialEq, Eq, Debug)]
        struct TestValue {
            id: i32,
            name: String,
        }

        impl From<&i32> for TestValue {
            fn from(id: &i32) -> Self {
                TestValue {
                    id: *id,
                    name: format!("Value-{}", id),
                }
            }
        }

        impl Hash for TestValue {
            fn hash<H: Hasher>(&self, state: &mut H) {
                self.id.hash(state);
            }
        }

        impl Equivalent<TestValue> for i32 {
            fn equivalent(&self, key: &TestValue) -> bool {
                *self == key.id
            }
        }

        let set: StablePointerSet<TestValue> = StablePointerSet::new();

        // Insert using equivalent reference (i32 -> TestValue)
        let (ptr1, inserted) = set.insert_equiv(&42);
        assert!(inserted, "Value should be inserted");
        // SAFETY: the elements live in `set` for the duration of the test.
        assert_eq!(unsafe { ptr1.deref() }.id, 42);
        assert_eq!(unsafe { ptr1.deref() }.name, "Value-42");

        // Try inserting the same value again via equivalent
        let (ptr2, inserted) = set.insert_equiv(&42);
        assert!(!inserted, "Value should not be inserted again");
        assert_eq!(ptr1, ptr2, "Should return the same pointer");

        // Insert a different value
        let (ptr3, inserted) = set.insert_equiv(&100);
        assert!(inserted, "New value should be inserted");
        assert_eq!(unsafe { ptr3.deref() }.id, 100);
        assert_eq!(unsafe { ptr3.deref() }.name, "Value-100");

        // Ensure we have exactly two elements
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_stable_pointer_deref() {
        let set = StablePointerSet::new();
        let (ptr, _) = set.insert(42);

        // Test dereferencing
        // SAFETY: the element lives in `set` for the duration of the test.
        let value: &i32 = unsafe { ptr.deref() };
        assert_eq!(*value, 42);

        // Test methods on the dereferenced value
        assert_eq!(unsafe { ptr.deref() }.checked_add(10), Some(52));
    }

    #[test]
    fn test_stable_pointer_set_remove() {
        let set = StablePointerSet::new();

        // Insert values
        let (ptr1, _) = set.insert(42);
        let (ptr2, _) = set.insert(100);
        assert_eq!(set.len(), 2);

        // SAFETY: `ptr1` and `ptr2` are the last pointers to their elements.
        unsafe {
            // Remove one value
            assert!(set.remove(ptr1));
            assert_eq!(set.len(), 1);

            // Remove other value
            assert!(set.remove(ptr2));
            assert_eq!(set.len(), 0);
        }
    }

    #[test]
    fn test_stable_pointer_set_retain() {
        let set = StablePointerSet::new();

        // Insert values
        set.insert(1);
        let (ptr2, _) = set.insert(2);
        set.insert(3);
        let (ptr4, _) = set.insert(4);
        assert_eq!(set.len(), 4);

        // SAFETY: No pointers to the removed (odd) elements are used afterwards,
        // and each predicate borrow is only read while the element is present.
        unsafe {
            // Retain only even numbers
            set.retain(|x| *x.deref() % 2 == 0);
        }

        // Verify results
        assert_eq!(set.len(), 2);
        assert!(!set.contains(&1));
        assert!(set.contains(&2));
        assert!(!set.contains(&3));
        assert!(set.contains(&4));

        // SAFETY: `ptr2` and `ptr4` are the last pointers to their elements.
        unsafe {
            // Verify that removed pointers are invalid and remaining are valid
            assert!(set.remove(ptr2));
            assert!(set.remove(ptr4));
        }
    }

    #[test]
    fn test_stable_pointer_set_custom_allocator() {
        // Test with System allocator
        let set: StablePointerSet<i32, RandomState, System> = StablePointerSet::new_in(System);

        // Insert some values
        let (ptr1, inserted) = set.insert(42);
        assert!(inserted);
        let (ptr2, inserted) = set.insert(100);
        assert!(inserted);

        // Check that everything works as expected
        assert_eq!(set.len(), 2);
        // SAFETY: the elements live in `set` for the duration of the test.
        assert_eq!(unsafe { *ptr1.deref() }, 42);
        assert_eq!(unsafe { *ptr2.deref() }, 100);

        // Test contains
        assert!(set.contains(&42));
        assert!(set.contains(&100));
        assert!(!set.contains(&200));
    }

    #[test]
    fn test_stable_pointer_set_custom_hasher_and_allocator() {
        // Use both custom hasher and allocator
        let set: StablePointerSet<i32, BuildHasherDefault<FxHasher>, System> =
            StablePointerSet::with_hasher_in(BuildHasherDefault::<FxHasher>::default(), System);

        // Insert some values
        let (ptr1, inserted) = set.insert(42);
        assert!(inserted);
        let (ptr2, inserted) = set.insert(100);
        assert!(inserted);

        // Check that everything works as expected
        assert_eq!(set.len(), 2);
        // SAFETY: the elements live in `set` for the duration of the test.
        assert_eq!(unsafe { *ptr1.deref() }, 42);
        assert_eq!(unsafe { *ptr2.deref() }, 100);

        // Test contains
        assert!(set.contains(&42));
        assert!(set.contains(&100));
        assert!(!set.contains(&200));
    }
}
