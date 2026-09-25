use std::alloc::Layout;
use std::array;
use std::cell::Cell;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::ptr::NonNull;
use std::sync::Mutex;
use std::sync::MutexGuard;

use allocator_api2::alloc::AllocError;
use allocator_api2::alloc::Allocator;
use thread_local::ThreadLocal;

use crate::FreeList;
use crate::FreeListEntry;

/// The number of entries in every free list chunk.
const FREE_LIST_CHUNK_SIZE: usize = 1000;

/// This is a memory pool or also called fixed-size block allocator for a
/// concrete type `T`. It stores blocks of `N` to minimize the overhead of
/// individual memory allocations, which are typically in the range of one or
/// two words.
///
/// Behaves like `Allocator`, except that it only allocates for layouts of `T`.
/// Requires periodic calls to `remove_free_blocks` to prevent memory usage from
/// growing indefinitely.
///
/// This allocator minimizes contention by maintaining per-thread state for
/// the common allocation/deallocation paths and only takes a lock when a new
/// block needs to be allocated. This does mean that external synchronisation
/// is required to prevent concurrent allocations overlapping with `remove_free_blocks`.
///
/// A single thread-local freelist is maintained per thread:
/// - `free`: popped from during allocation and pushed to during deallocation.
pub struct BlockAllocator<T: Send, const N: usize> {
    /// Owns the block list; only locked when a new block must be allocated.
    /// This is the only shared state accessed during allocation.
    blocks: Mutex<BlockList<T, N>>,

    /// Per-thread state for allocation: current block and bump offset. This eliminates
    /// contention in the common path.
    alloc_state: ThreadLocal<ThreadLocalAllocState<T, N>>,
}

/// The block list and bump pointer, protected by the blocks mutex.
struct BlockList<T, const N: usize> {
    /// The block that is currently being bump-allocated from. We avoid Box here
    /// to allow multiple blocks to point to the same next block without
    /// violating Box's noalias.
    head_block: Option<NonNull<Block<T, N>>>,

    /// Shared chunks of free entries represented by list heads. Each head
    /// points to a null-terminated list of length `FREE_LIST_CHUNK_SIZE`
    /// (except possibly the final chunk).
    free_chunks: Vec<NonNull<Entry<T>>>,
}

impl<T, const N: usize> Drop for BlockList<T, N> {
    fn drop(&mut self) {
        // Drop the head block; Block's Drop impl recursively drops the list.
        if let Some(block_ptr) = self.head_block.take() {
            // SAFETY: we own all blocks in the list, and each was created via
            // `Box::into_raw` (in `allocate_new_block`), so `Box::from_raw`
            // reconstructs a matching box.
            unsafe { drop(Box::from_raw(block_ptr.as_ptr())) };
        }
    }
}

impl<T: Send, const N: usize> Default for BlockAllocator<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send, const N: usize> BlockAllocator<T, N> {
    pub fn new() -> Self {
        Self {
            blocks: Mutex::new(BlockList {
                head_block: None,
                free_chunks: Vec::new(),
            }),
            alloc_state: ThreadLocal::new(),
        }
    }

    /// Allocates a slot for one object of type `T`.
    pub fn allocate_object(&self) -> Result<NonNull<T>, AllocError> {
        let state = self.alloc_state.get_or(ThreadLocalAllocState::new);

        // Fast path 1: try thread-local free list.
        if let Some(entry) = state.free.try_pop() {
            return Ok(entry.cast());
        }

        // Fast path 2: lock-free bump allocation from current thread's block.
        let block_ptr = state.current_block.get();
        let offset = state.bump_offset.get();
        if !block_ptr.is_null() && offset < N {
            state.bump_offset.set(offset + 1);
            // SAFETY: `block_ptr` is non-null and owned by this thread's
            // state; it stays valid until `remove_free_blocks`, which the
            // caller must not run concurrently with allocation. `offset < N`
            // was just checked, so `data_ptr.add(offset)` stays within the
            // block's `N`-element array. We use `addr_of_mut!` instead of
            // forming a reference so this does not race with other in-bounds
            // accesses to the same block via `UnsafeCell`, and the result is
            // never null.
            return unsafe {
                let data_ptr = (*block_ptr).data.get() as *mut Entry<T>;
                let entry_ptr = data_ptr.add(offset);
                Ok(NonNull::new_unchecked(
                    std::ptr::addr_of_mut!((*entry_ptr).data) as *mut T
                ))
            };
        }

        // Slow path: acquire the lock once and reuse it across refill /
        // new-block allocation.
        let mut guard = self.blocks.lock().expect("Lock poisoned");

        if self.refill_local_free_from_chunks(state, &mut guard)
            && let Some(entry) = state.free.try_pop()
        {
            return Ok(entry.cast());
        }

        self.allocate_new_block(state, guard)
    }

    /// Refills the calling thread's local freelist from one shared chunk.
    fn refill_local_free_from_chunks(
        &self,
        state: &ThreadLocalAllocState<T, N>,
        guard: &mut MutexGuard<'_, BlockList<T, N>>,
    ) -> bool {
        let Some(current) = guard.free_chunks.pop() else {
            return false;
        };

        debug_assert!(
            state.free.is_empty(),
            "local freelist must be empty before chunk refill"
        );
        // SAFETY: `current` was popped from `guard.free_chunks`, a
        // null-terminated freelist of live, allocator-owned entries built by
        // `remove_free_blocks`/`deallocate_object`, and `state.free` is
        // empty per the assertion above, so installing it as the new head
        // does not leak or double-link entries.
        unsafe {
            state.free.set_head(current.as_ptr());
        }
        true
    }

    /// Slow path: allocate a new block and update thread-local state.
    #[cold]
    fn allocate_new_block(
        &self,
        state: &ThreadLocalAllocState<T, N>,
        mut guard: MutexGuard<'_, BlockList<T, N>>,
    ) -> Result<NonNull<T>, AllocError> {
        // Allocate a new block and link it to the existing list.
        let mut new_block = Block::new();
        new_block.next = guard.head_block;
        // SAFETY: `Box::into_raw` never returns a null pointer.
        let new_block_ptr = unsafe { NonNull::new_unchecked(Box::into_raw(Box::new(new_block))) };
        guard.head_block = Some(new_block_ptr);

        drop(guard);

        // Update thread-local state with new block.
        state.current_block.set(new_block_ptr.as_ptr());
        state.bump_offset.set(1);

        // Return slot 0 of the new block.
        // SAFETY: `new_block_ptr` was just allocated above and is
        // exclusively owned by this call until installed into thread-local
        // state; `N >= 1` since slot 0 is being handed out, so indexing
        // element 0 of the `N`-element array is in-bounds, and
        // `addr_of_mut!` yields a pointer that is never null.
        unsafe {
            let data_ptr = (*new_block_ptr.as_ptr()).data.get() as *mut Entry<T>;
            Ok(NonNull::new_unchecked(
                std::ptr::addr_of_mut!((*data_ptr).data) as *mut T
            ))
        }
    }

    /// Deallocates a previously-allocated pointer.
    pub fn deallocate_object(&self, ptr: NonNull<T>) {
        let state = self.alloc_state.get_or(ThreadLocalAllocState::new);
        // SAFETY: `ptr` was returned by `allocate_object`, so it points to a block entry that
        // stays valid until `remove_free_blocks` reclaims it.
        unsafe {
            state.free.push(ptr.cast());
        }
    }

    /// Removes empty blocks from the block list.
    ///
    /// Should be called periodically to prevent memory usage from growing
    /// indefinitely.
    ///
    /// **Important**: This method must not be called concurrently with any
    /// allocations or deallocations. It should be called at a synchronization
    /// point where no other threads are active on this allocator.
    ///
    /// This method scans every thread-local `free` list, marks those entries
    /// with a sentinel value, then removes blocks where all entries are marked
    /// as free.
    ///
    /// Returns `(removed_blocks, free_size)`: the number of blocks removed, and
    /// the size of the merged `free` list before cleanup.
    pub fn remove_free_blocks(&mut self) -> (usize, usize)
    where
        T: BlockAllocatorSafe,
    {
        // Mark all elements in all thread-local freelists with a special
        // value that none of the live entries can have.
        let nonexisting_value = NONEXISTING_VALUE as *mut Entry<T>;

        // In debug mode, collect all freelist entry pointers.
        #[cfg(debug_assertions)]
        let mut freelist_ptrs: std::collections::HashSet<*mut Entry<T>> = std::collections::HashSet::new();

        // Helper: walk a freelist and mark every entry with the sentinel.
        // We only update previous entries to ensure that iter keeps working.
        // Returns the number of entries in the freelist.
        // SAFETY (closure body below): `remove_free_blocks` must not run
        // concurrently with allocation or deallocation (see the doc comment
        // above), so every pointer yielded by `list.iter()` is a live
        // freelist node exclusively owned by this call, safe to dereference
        // and overwrite through `next`.
        let mark_freelist =
            |list: &FreeList<Entry<T>>,
             #[cfg(debug_assertions)] ptrs: &mut std::collections::HashSet<*mut Entry<T>>| unsafe {
                let mut previous: Option<NonNull<Entry<T>>> = None;
                let mut count: usize = 0;
                for current in list.iter() {
                    #[cfg(debug_assertions)]
                    ptrs.insert(current.as_ptr());

                    if let Some(previous) = previous {
                        *(*previous.as_ptr()).next = nonexisting_value;
                    }
                    previous = Some(current);
                    count += 1;
                }

                if let Some(previous) = previous {
                    *(*previous.as_ptr()).next = nonexisting_value;
                }
                count
            };

        let mut free_size = 0;
        for state in self.alloc_state.iter_mut() {
            free_size += mark_freelist(
                &state.free,
                #[cfg(debug_assertions)]
                &mut freelist_ptrs,
            );
            state.free.clear();
            // Keep the thread-local bump pointer state; those entries will be
            // reclaimed when we walk the blocks next.
        }

        let mut guard = self.blocks.lock().expect("Lock poisoned");

        // Mark entries currently staged in shared free chunks.
        for &chunk_head in &guard.free_chunks {
            let mut current = Some(chunk_head);
            while let Some(entry) = current {
                #[cfg(debug_assertions)]
                freelist_ptrs.insert(entry.as_ptr());

                // SAFETY: `entry` is a live node reached by walking
                // `guard.free_chunks`, exclusively accessed here under the
                // held mutex while `remove_free_blocks`'s no-concurrent-
                // allocation contract holds.
                let next = unsafe { Entry::get_next(entry.as_ptr()) };
                unsafe {
                    *(*entry.as_ptr()).next = nonexisting_value;
                }
                free_size += 1;
                current = NonNull::new(next);
            }
        }
        guard.free_chunks.clear();

        // Debug check: verify that no live entry has the sentinel value.
        #[cfg(debug_assertions)]
        {
            for block_ptr in Self::iter_blocks(&guard) {
                // SAFETY: `block_ptr` is reachable from `guard.head_block`,
                // so it is allocator-owned and kept alive while the guard is
                // held; no concurrent allocation/deallocation can be
                // touching it per this function's synchronization contract.
                let data = unsafe { &*(*block_ptr.as_ptr()).data.get() };
                for entry in data {
                    let entry_ptr = entry as *const Entry<T> as *mut Entry<T>;
                    if !freelist_ptrs.contains(&entry_ptr) {
                        // This entry is live — it must not look like the sentinel.
                        // SAFETY: reading `entry.next` reinterprets the
                        // union's `data` variant as a pointer; this is sound
                        // per `BlockAllocatorSafe`'s contract that a live
                        // `T`'s first pointer-sized word is always fully
                        // initialized and never equals the sentinel.
                        unsafe {
                            debug_assert!(
                                !std::ptr::eq(*entry.next, nonexisting_value),
                                "Live entry at {entry_ptr:?} has the sentinel value (null in first word). \
                                 This violates the BlockAllocatorSafe contract."
                            );
                        }
                    }
                }
            }
        }

        let removed = if guard.head_block.is_some() {
            // Remove blocks that are now empty, i.e., all their entries have nonexisting_value.
            let mut prev_next_field: *mut Option<NonNull<Block<T, N>>> = &mut guard.head_block;
            let mut removed_blocks = 0;

            // SAFETY (whole loop): `prev_next_field` always points to a live
            // `Option<NonNull<Block<T, N>>>` link — initially `guard.head_block`,
            // and afterwards a `next` field of a block reachable from it — that
            // this call exclusively owns while the mutex guard is held, per
            // `remove_free_blocks`'s no-concurrent-access contract.
            while let Some(current_ptr) = unsafe { *prev_next_field } {
                // SAFETY: `current_ptr` was just read from a valid list link
                // above; reading every entry's `next` field, even for
                // entries whose live variant is `data`, is sound per
                // `BlockAllocatorSafe`'s contract (see above).
                let all_free = unsafe {
                    let data = &*(*current_ptr.as_ptr()).data.get();
                    data.iter().all(|entry| std::ptr::eq(*entry.next, nonexisting_value))
                };

                if all_free {
                    // Unlink and drop the current block.
                    // SAFETY: `current_ptr` is valid per above.
                    let next = unsafe { (*current_ptr.as_ptr()).next.take() };
                    // SAFETY: `prev_next_field` is a valid link per above.
                    unsafe { *prev_next_field = next };
                    // SAFETY: `current_ptr` was just unlinked from the list,
                    // so this call is its sole owner; like all blocks it was
                    // originally created via `Box::into_raw`
                    // (`allocate_new_block`), so `Box::from_raw` reconstructs
                    // a matching box.
                    unsafe { drop(Box::from_raw(current_ptr.as_ptr())) };
                    removed_blocks += 1;
                    // prev_next_field stays the same — it now points to the next block.
                } else {
                    // Keep this block; advance to next.
                    // SAFETY: `current_ptr` is valid per above.
                    prev_next_field = unsafe { &mut (*current_ptr.as_ptr()).next };
                }
            }

            removed_blocks
        } else {
            // No blocks, nothing to remove.
            0
        };

        // Recreate shared free chunks from remaining blocks.
        let mut rebuilt_chunk_heads: Vec<NonNull<Entry<T>>> = Vec::new();
        let mut chunk_head: Option<NonNull<Entry<T>>> = None;
        let mut chunk_tail: Option<NonNull<Entry<T>>> = None;
        let mut chunk_len = 0usize;

        for block_ptr in Self::iter_blocks(&guard) {
            // Walk entries via raw pointers; deriving `*mut` from `&Entry<T>`
            // would violate Stacked Borrows when we write through that pointer.
            // SAFETY: `block_ptr` is reachable from `guard.head_block`, so it
            // is allocator-owned and exclusively accessed here while the
            // guard is held.
            let data_ptr = unsafe { (*block_ptr.as_ptr()).data.get() as *mut Entry<T> };
            for i in 0..N {
                // SAFETY: `i < N`, so `data_ptr.add(i)` stays within the
                // block's `N`-element array, making `entry_ptr` non-null;
                // reading/writing `.next` is valid under the same union
                // contract as above (`BlockAllocatorSafe`), done exclusively
                // under the guard.
                unsafe {
                    let entry_ptr = data_ptr.add(i);
                    if std::ptr::eq(*(*entry_ptr).next, nonexisting_value) {
                        *(*entry_ptr).next = std::ptr::null_mut();
                        let entry_ptr = NonNull::new_unchecked(entry_ptr);

                        if let Some(tail) = chunk_tail {
                            *(*tail.as_ptr()).next = entry_ptr.as_ptr();
                            chunk_tail = Some(entry_ptr);
                            chunk_len += 1;
                        } else {
                            chunk_head = Some(entry_ptr);
                            chunk_tail = Some(entry_ptr);
                            chunk_len = 1;
                        }

                        if chunk_len == FREE_LIST_CHUNK_SIZE {
                            rebuilt_chunk_heads.push(chunk_head.expect("chunk head set"));
                            chunk_head = None;
                            chunk_tail = None;
                            chunk_len = 0;
                        }
                    }
                }
            }
        }

        if let Some(head) = chunk_head {
            rebuilt_chunk_heads.push(head);
        }

        guard.free_chunks.extend(rebuilt_chunk_heads);

        drop(guard);
        (removed, free_size)
    }

    /// Returns an iterator over the blocks.
    ///
    /// The caller must pass the already-acquired guard to avoid a deadlock.
    fn iter_blocks<'a>(guard: &'a MutexGuard<'_, BlockList<T, N>>) -> BlockIter<'a, T, N> {
        BlockIter {
            current: guard.head_block,
            _marker: PhantomData,
        }
    }
}

/// Per-thread state for allocation: current block and bump offset.
/// This eliminates contention in the hot path.
struct ThreadLocalAllocState<T, const N: usize> {
    /// The block currently being bump-allocated from.
    current_block: Cell<*mut Block<T, N>>,

    /// Bump offset within `current_block`. N or greater means the block is full.
    bump_offset: Cell<usize>,

    /// Thread-local free list (only popped from during allocation).
    free: FreeList<Entry<T>>,
}

impl<T, const N: usize> ThreadLocalAllocState<T, N> {
    fn new() -> Self {
        Self {
            current_block: Cell::new(std::ptr::null_mut()),
            bump_offset: Cell::new(N),
            free: FreeList::new(),
        }
    }
}

// SAFETY: `current_block: Cell<*mut Block<T, N>>` (a raw pointer) is what blocks
// auto-`Send`/auto-`Sync`; `bump_offset: Cell<usize>` is `Send` but not `Sync` regardless of
// `T` (any `Cell` is `!Sync`), and `free: FreeList<Entry<T>>` is `Send` for `T: Send` (see
// `FreeList`'s own `unsafe impl Send`) and likewise never `Sync`.
//
// Contract discharged by this impl (`Send` only — no `Sync` impl exists or is needed, since
// `ThreadLocal<X>` only requires `X: Send` to itself be `Send + Sync`, exposing each thread's
// slot only to that same thread): `ThreadLocal::get_or` guarantees a given
// `ThreadLocalAllocState` instance is created by, and every subsequent `current_block`/
// `bump_offset`/`free` access (`allocate_object`, `deallocate_object`) is performed by, only the
// thread that owns that `ThreadLocal` slot — never concurrently by two threads. `Send` is
// needed only because the whole `ThreadLocal<..>` collection (and thus every thread's
// `ThreadLocalAllocState`) may be dropped from a different thread than created it (e.g.
// alongside `BlockAllocator` itself); that drop path performs no dereference of `current_block`
// (`FreeList::drop`/`Cell::drop` are pure memory reclamation of the `Cell`/`FreeList` structure,
// not of the pointee), so no thread-affinity requirement is violated by the value's final drop
// running elsewhere. `T: Send` is required transitively by `free: FreeList<Entry<T>>`.
unsafe impl<T: Send, const N: usize> Send for ThreadLocalAllocState<T, N> {}

/// Implementing this trait for a type `T` asserts that the special sentinel
/// never occurs as a valid entry.
///
/// # Safety
///
/// Marker trait asserting two properties of `T`, both required because
/// [`BlockAllocator::remove_free_blocks`] reads the first
/// `size_of::<*mut _>()` bytes of every *live* entry through the freelist
/// union:
///
/// - The sentinel value—a pointer value where all bytes are set to `0xFF`
///   (i.e., `usize::MAX` cast to `*mut Entry<T>`)—can never appear as the
///   first `size_of::<*mut _>()` bytes of any valid value of `T`.
/// - The first `size_of::<*mut _>()` bytes of any valid value of `T` are
///   always fully initialized (no padding or uninitialized bytes in the first
///   pointer-sized word), since reading uninitialized bytes is undefined
///   behaviour.
pub unsafe trait BlockAllocatorSafe {}

/// Sentinel value used to identify entries that are not on the freelist.
const NONEXISTING_VALUE: usize = usize::MAX;

/// The [BlockAllocator] is thread-safe.
// SAFETY: `blocks: Mutex<BlockList<T, N>>` holds the only `NonNull` pointers
// (`head_block`, `free_chunks`), and every access to them happens while the
// mutex is held, giving exclusive access from whichever thread holds the
// lock; `Mutex<X>` is itself `Send`/`Sync` given `X: Send`, which holds here
// since `T: Send`. `alloc_state: ThreadLocal<ThreadLocalAllocState<T, N>>` is
// `Send`/`Sync` under the same `T: Send` bound (see the impl above). So both
// fields are safe to share/move across threads, making `BlockAllocator`
// itself sound to mark `Send`/`Sync`.
unsafe impl<T: Send, const N: usize> Send for BlockAllocator<T, N> {}
unsafe impl<T: Send, const N: usize> Sync for BlockAllocator<T, N> {}

/// `AllocBlock` implements the [`Allocator`] trait using the underlying [`BlockAllocator`].
pub struct AllocBlock<T: Send, const N: usize> {
    block_allocator: BlockAllocator<T, N>,
}

impl<T: Send, const N: usize> Default for AllocBlock<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Send, const N: usize> AllocBlock<T, N> {
    /// Creates a new `AllocBlock`.
    pub fn new() -> Self {
        Self {
            block_allocator: BlockAllocator::new(),
        }
    }

    /// Removes free blocks from the underlying block allocator, see [`BlockAllocator::remove_free_blocks`].
    pub fn remove_free_blocks(&mut self) -> usize
    where
        T: BlockAllocatorSafe,
    {
        self.block_allocator.remove_free_blocks().0
    }
}

// SAFETY: `allocate` only ever hands out pointers obtained from
// `block_allocator.allocate_object()`, which are valid `NonNull<T>` slots
// owned by this same `block_allocator`; `deallocate` requires (via its own
// safety contract) that `ptr`/`layout` came from a matching `allocate` call
// on this allocator, which is exactly what `deallocate_object` expects. Since
// `allocate` rejects any layout other than `Layout::new::<T>()`, every
// pointer that reaches `deallocate` was produced for that same layout.
unsafe impl<T: Send, const N: usize> Allocator for AllocBlock<T, N> {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        // The blocks only fit objects with exactly T's layout; returning one for a larger
        // request would hand the caller a too-small allocation.
        if layout != Layout::new::<T>() {
            return Err(AllocError);
        }

        let ptr = self.block_allocator.allocate_object()?;

        // Convert NonNull<T> to NonNull<[u8]> with the correct size
        let byte_ptr = ptr.cast::<u8>();
        let slice_ptr = NonNull::slice_from_raw_parts(byte_ptr, std::mem::size_of::<T>());

        Ok(slice_ptr)
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        debug_assert_eq!(
            layout,
            Layout::new::<T>(),
            "The requested layout should match the type T"
        );
        self.block_allocator.deallocate_object(ptr.cast::<T>());
    }
}

union Entry<T> {
    /// Stores the actual element.
    data: ManuallyDrop<T>,

    /// If the element is free, this points to the next entry in the freelist, or null if this is the last entry.
    next: ManuallyDrop<*mut Entry<T>>,
}

// SAFETY: `Entry<T>` stores a single intrusive next-pointer in `next` used only
// while the slot is on the freelist.
unsafe impl<T> FreeListEntry for Entry<T> {
    unsafe fn get_next(ptr: *mut Self) -> *mut Self {
        // SAFETY: caller ensures `ptr` is a valid freelist node.
        unsafe { *(*ptr).next }
    }

    unsafe fn set_next(ptr: *mut Self, next: *mut Self) {
        // SAFETY: caller ensures `ptr` is a valid freelist node.
        unsafe {
            *(*ptr).next = next;
        }
    }
}

/// An iterator over the blocks in the block allocator.
struct BlockIter<'a, T, const N: usize> {
    current: Option<NonNull<Block<T, N>>>,
    _marker: PhantomData<&'a BlockList<T, N>>,
}

impl<'a, T, const N: usize> Iterator for BlockIter<'a, T, N> {
    type Item = NonNull<Block<T, N>>;

    fn next(&mut self) -> Option<Self::Item> {
        let current_ptr = self.current?;

        // Move to the next block for the next iteration.
        // SAFETY: `current_ptr` is reachable from the `BlockList` the caller
        // passed in via `iter_blocks`, whose doc requires the already-
        // acquired guard, so the block stays valid and exclusively
        // accessible for the lifetime of this borrow.
        self.current = unsafe { (*current_ptr.as_ptr()).next };

        Some(current_ptr)
    }
}

/// We maintain a list of blocks that store N elements each.
struct Block<T, const N: usize> {
    /// Wrapped in UnsafeCell to avoid Stacked Borrows invalidation of
    /// outstanding pointers during subsequent allocations from the same block.
    data: UnsafeCell<[Entry<T>; N]>,

    /// Pointer to the next block (raw to avoid Box noalias).
    next: Option<NonNull<Block<T, N>>>,
}

impl<T, const N: usize> Block<T, N> {
    fn new() -> Self {
        Self {
            data: UnsafeCell::new(array::from_fn(|_i| Entry {
                next: ManuallyDrop::new(std::ptr::null_mut()),
            })),
            next: None,
        }
    }
}

impl<T, const N: usize> Drop for Block<T, N> {
    fn drop(&mut self) {
        // Iteratively drop the list to avoid stack overflow on long lists.
        let mut current = self.next.take();
        while let Some(block_ptr) = current {
            // SAFETY: every block in the `next` chain was created via
            // `Box::into_raw` (in `allocate_new_block`) and is unlinked from
            // the list here before being dropped, so this is its sole owner.
            let mut block = unsafe { Box::from_raw(block_ptr.as_ptr()) };
            current = block.next.take();
        }
    }
}

#[cfg(kani)]
mod verification {
    use super::*;

    /// `Entry<T>`'s `FreeListEntry` impl reinterprets the union's `next` variant; check that
    /// writing a next-pointer through `set_next` and reading it back through `get_next` is the
    /// identity, for an arbitrary (possibly null, possibly dangling — never dereferenced here)
    /// pointer value.
    #[kani::proof]
    fn entry_get_next_set_next_roundtrip() {
        let mut entry: Entry<u64> = Entry {
            next: ManuallyDrop::new(std::ptr::null_mut()),
        };
        let entry_ptr: *mut Entry<u64> = &mut entry;
        let next_value: *mut Entry<u64> = kani::any::<usize>() as *mut Entry<u64>;

        // SAFETY: `entry_ptr` is a valid pointer to `entry`, a local variable; `set_next`/
        // `get_next` only read/write the `next` field, never dereferencing `next_value` itself.
        unsafe {
            Entry::set_next(entry_ptr, next_value);
            assert_eq!(Entry::get_next(entry_ptr), next_value);
        }
    }

    /// Proves the safety comment on `allocate_object`'s fast (bump-allocation) path: for any
    /// `offset < N`, `data_ptr.add(offset)` stays within the block's `N`-element array, and the
    /// `T`-typed pointer built via `addr_of_mut!` from it is never null.
    #[kani::proof]
    #[kani::unwind(5)]
    fn bump_allocation_offset_stays_in_bounds() {
        const N: usize = 4;
        let block: Block<u64, N> = Block::new();
        let offset: usize = kani::any();
        kani::assume(offset < N);

        let data_ptr = block.data.get() as *mut Entry<u64>;
        // SAFETY: mirrors the indexing done in `allocate_object`'s fast path; `offset < N` is
        // the exact precondition for `data_ptr.add(offset)` to stay in the block's array.
        unsafe {
            let entry_ptr = data_ptr.add(offset);
            assert!(entry_ptr >= data_ptr);
            assert!(entry_ptr < data_ptr.add(N));

            let t_ptr = NonNull::new_unchecked(std::ptr::addr_of_mut!((*entry_ptr).data) as *mut u64);
            assert!(!t_ptr.as_ptr().is_null());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;
    use std::sync::Arc;

    use rand::RngExt;

    use merc_utilities::random_test;

    use super::BlockAllocator;
    use super::BlockAllocatorSafe;

    // In practice u64 is used only in tests; real clients must audit their types.
    // SAFETY: `usize::MAX` (the sentinel) never occurs as an ordinary `usize`
    // test value here, and `usize`'s bytes are always fully initialized.
    unsafe impl BlockAllocatorSafe for usize {}

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_block_allocator() {
        random_test(100, |rng| {
            let mut allocator: BlockAllocator<usize, 32> = BlockAllocator::new();

            // Allocate 1000 elements and keep track of ptr to value mapping.
            let mut allocated: Vec<(NonNull<usize>, usize)> = Vec::new();
            for _ in 0..1000 {
                let ptr = allocator.allocate_object().unwrap();
                let value: usize = rng.random_range(0..=usize::MAX - 1);
                // SAFETY: `ptr` was just returned by `allocate_object` and
                // points to a freshly-allocated, uninitialized slot exclusively
                // owned by this test until deallocated.
                unsafe {
                    ptr.as_ptr().write(value);
                }
                allocated.push((ptr, value));
            }

            // Deallocate a random subset and keep track of which entries remain live.
            let mut remaining = Vec::new();
            for (ptr, value) in allocated {
                if rng.random_bool(0.5) {
                    allocator.deallocate_object(ptr);
                } else {
                    remaining.push((ptr, value));
                }
            }

            // All remaining elements must still hold their original values.
            for (ptr, expected) in &remaining {
                // SAFETY: `ptr` was not deallocated, so it still points to a
                // live, initialized slot owned by this test.
                unsafe {
                    assert_eq!(*ptr.as_ref(), *expected);
                }
            }

            let (removed, free_size) = allocator.remove_free_blocks();
            println!("{removed} removed, {free_size} free");

            for _ in 0..500 {
                let ptr = allocator.allocate_object().unwrap();
                let value: usize = rng.random_range(0..=usize::MAX - 1);
                // SAFETY: `ptr` was just returned by `allocate_object` and
                // points to a freshly-allocated, uninitialized slot exclusively
                // owned by this test until deallocated.
                unsafe {
                    ptr.as_ptr().write(value);
                }
                remaining.push((ptr, value));
            }

            // All remaining elements must have the correct values.
            for (ptr, expected) in &remaining {
                // SAFETY: `ptr` was not deallocated, so it still points to a
                // live, initialized slot owned by this test.
                unsafe {
                    assert_eq!(*ptr.as_ref(), *expected);
                }
            }
        })
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_block_allocator_parallel_freelist() {
        let block_allocator = Arc::new(BlockAllocator::<u32, 32>::new());

        let threads: Vec<_> = (0..=2)
            .map(|_| {
                let block_allocator = block_allocator.clone();

                std::thread::spawn(move || {
                    // Not sure if this could actually trigger the ABA problem,
                    // but this is only used to detect data races.
                    let mut ptrs = Vec::new();
                    for _ in 0..100 {
                        let ptr = block_allocator.allocate_object().unwrap();
                        // SAFETY: `ptr` was just returned by `allocate_object`
                        // and points to a freshly-allocated, uninitialized
                        // slot exclusively owned by this thread until
                        // deallocated.
                        unsafe {
                            ptr.as_ptr().write(42);
                        }
                        ptrs.push(ptr);
                    }

                    for ptr in ptrs {
                        // SAFETY: `ptr` was not yet deallocated, so it still
                        // points to a live, initialized slot owned by this
                        // thread.
                        unsafe {
                            assert_eq!(*ptr.as_ref(), 42);
                        }
                        block_allocator.deallocate_object(ptr);
                    }
                })
            })
            .collect();

        for thread in threads {
            thread.join().unwrap();
        }
    }
}
