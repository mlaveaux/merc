use std::array;
use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::mem::needs_drop;
use std::ptr::null_mut;
use std::sync::atomic::AtomicPtr;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use crossbeam_utils::CachePadded;
use thread_local::ThreadLocal;

/// Fixed number of buckets. A `usize`-indexed pool never needs more, because the
/// buckets grow geometrically and the largest reachable block index lands well
/// inside this range (checked in [`bucket_of_block`]).
const NUM_BUCKETS: usize = usize::BITS as usize;

/// Number of blocks held by the first bucket, expressed as a shift. Later
/// buckets double in size, so this also fixes how quickly the geometric growth
/// kicks in.
const FIRST_BITS: u32 = 4;
const FIRST_BLOCKS: usize = 1 << FIRST_BITS;

/// An append-only, thread-safe vector whose [`push`](Self::push) returns the
/// index the element landed at, trading dense indices for low write contention.
///
/// # Details
///
/// Where `boxcar::Vec` tags every slot with its own atomic flag and
/// `append-only-vec` keeps indices dense with a per-push compare-exchange spin,
/// this structure hands each thread a whole *block* of `BLOCK` consecutive
/// indices with a single `fetch_add` and lets it fill them with plain writes.
/// The shared cursor is therefore touched only once per `BLOCK` pushes, and a
/// thread's writes never contend with another's.
///
/// The price is that indices are not dense: a thread that stops pushing leaves
/// the unused tail of its current block as a permanent gap. [`len`](Self::len)
/// counts only the elements actually written, and [`get`](Self::get) returns
/// `None` for a gap.
///
/// Each block is published with one cache-line-padded counter (one per `BLOCK`
/// slots, not one per slot), which both makes a written slot visible to readers
/// and records exactly which slots to drop. Storage lives in geometrically
/// growing buckets, so the per-element overhead is the padded counter amortized
/// over a block plus the usual geometric slack — far below `boxcar`'s per-slot
/// flag.
pub struct ConcurrentAppendVec<T, const BLOCK: usize = 256> {
    /// Block reservation cursor. `fetch_add(1)` hands out the next block of
    /// `BLOCK` indices; the base index of block `b` is `b * BLOCK`.
    reserved_blocks: CachePadded<AtomicUsize>,
    /// Per-thread reservation cursor and element count. Padded so two threads'
    /// hot push state never share a cache line.
    local: ThreadLocal<CachePadded<Local>>,
    /// Geometrically growing, lazily allocated storage buckets.
    buckets: [AtomicPtr<Bucket<T>>; NUM_BUCKETS],
    /// Marks `T` as owned so the auto `Send`/`Sync` derivation depends on it;
    /// the real bounds are supplied by the manual `unsafe impl`s below.
    _marker: PhantomData<*mut T>,
}

/// SAFETY: the vector owns its `T` values; moving it to another thread is sound
/// exactly when `T` may move between threads.
unsafe impl<T: Send, const BLOCK: usize> Send for ConcurrentAppendVec<T, BLOCK> {}

/// SAFETY: sharing `&self` lets threads push values (dropped or read by other
/// threads, needing `T: Send`) and obtain `&T` (needing `T: Sync`). Per-slot
/// writes are published through the per-block release/acquire counter, so no two
/// accesses to the same slot race.
unsafe impl<T: Send + Sync, const BLOCK: usize> Sync for ConcurrentAppendVec<T, BLOCK> {}

/// Per-thread state: the half-open range `[next, end)` of indices still free in
/// the thread's current block, plus the number of elements it has pushed.
///
/// `next` and `end` are touched only by the owning thread, but they are atomic
/// so that the whole struct is `Sync` and [`ConcurrentAppendVec::len`] can
/// iterate every thread's `count`.
struct Local {
    /// Next free index in the current block.
    next: AtomicUsize,
    /// One past the last index of the current block; `next == end` means the
    /// block is exhausted and a new one must be reserved.
    end: AtomicUsize,
    /// Count of elements this thread has pushed.
    count: AtomicUsize,
}

impl Local {
    fn new() -> Local {
        Local {
            next: AtomicUsize::new(0),
            end: AtomicUsize::new(0),
            count: AtomicUsize::new(0),
        }
    }
}

/// One storage bucket: a run of slots plus a published length per block.
struct Bucket<T> {
    /// `block_count * BLOCK` slots, written through `&self` so guarded by
    /// `UnsafeCell`.
    slots: Box<[UnsafeCell<MaybeUninit<T>>]>,
    /// Number of initialized slots at the front of each block. Padded to a cache
    /// line so a thread publishing its block does not falsely share with the
    /// thread filling the neighbouring block.
    commits: Box<[CachePadded<AtomicUsize>]>,
}

impl<T> Bucket<T> {
    fn new(slot_count: usize, block_count: usize) -> Bucket<T> {
        let slots = (0..slot_count)
            .map(|_| UnsafeCell::new(MaybeUninit::uninit()))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let commits = (0..block_count)
            .map(|_| CachePadded::new(AtomicUsize::new(0)))
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Bucket { slots, commits }
    }
}

impl<T> Drop for Bucket<T> {
    fn drop(&mut self) {
        if needs_drop::<T>() {
            let block_size = self.slots.len() / self.commits.len();
            for (block, commit) in self.commits.iter().enumerate() {
                let committed = commit.load(Ordering::Relaxed);
                for offset in 0..committed {
                    let slot = block * block_size + offset;
                    // SAFETY: a committed slot was written and is uniquely owned
                    // here (`&mut self`), so dropping it once is sound.
                    unsafe {
                        (*self.slots[slot].get()).assume_init_drop();
                    }
                }
            }
        }
    }
}

impl<T, const BLOCK: usize> ConcurrentAppendVec<T, BLOCK> {
    /// Base-two logarithm of `BLOCK`, valid because `BLOCK` is a power of two.
    const BLOCK_BITS: u32 = BLOCK.trailing_zeros();

    /// Creates a new empty vector.
    pub fn new() -> ConcurrentAppendVec<T, BLOCK> {
        const {
            assert!(
                BLOCK.is_power_of_two() && BLOCK >= 2,
                "BLOCK must be a power of two >= 2"
            )
        };
        ConcurrentAppendVec {
            reserved_blocks: CachePadded::new(AtomicUsize::new(0)),
            local: ThreadLocal::new(),
            buckets: array::from_fn(|_| AtomicPtr::new(null_mut())),
            _marker: PhantomData,
        }
    }

    /// Appends `value` and returns the index it was stored at. The returned
    /// indices are unique but not necessarily contiguous.
    pub fn push(&self, value: T) -> usize {
        let local = self.local.get_or(|| CachePadded::new(Local::new()));

        // Reserve a fresh block only once the current one is exhausted, so the
        // shared cursor is hit at most once per `BLOCK` pushes. Only this thread
        // touches `next`/`end`, so relaxed ordering suffices.
        let mut next = local.next.load(Ordering::Relaxed);
        if next == local.end.load(Ordering::Relaxed) {
            let block = self.reserved_blocks.fetch_add(1, Ordering::Relaxed);
            next = block << Self::BLOCK_BITS;
            local.end.store(next + BLOCK, Ordering::Relaxed);
        }
        local.next.store(next + 1, Ordering::Relaxed);
        local.count.fetch_add(1, Ordering::Relaxed);

        let index = next;
        let (bucket_index, block, offset) = Self::locate(index);
        let bucket = self.bucket_or_alloc(bucket_index);
        let slot = block * BLOCK + offset;

        // SAFETY: `index` belongs to a block this thread reserved exclusively, so
        // no other thread writes or reads this slot until the commit store below
        // publishes it; the write therefore does not race.
        unsafe {
            (*bucket.slots[slot].get()).write(value);
        }

        // Single writer per block, so the published length only grows; the
        // release pairs with the acquire in `get`/`iter` to expose the write.
        bucket.commits[block].store(offset + 1, Ordering::Release);
        index
    }

    /// Returns the element at `index`, or `None` if `index` was never written
    /// (out of range or a gap left by block reservation).
    pub fn get(&self, index: usize) -> Option<&T> {
        let (bucket_index, block, offset) = Self::locate(index);
        // SAFETY: a bucket pointer is null or a `Box` we own and keep alive until
        // `&mut self` is taken; the acquire pairs with the allocating release.
        let bucket = unsafe { self.buckets[bucket_index].load(Ordering::Acquire).as_ref()? };
        let committed = bucket.commits[block].load(Ordering::Acquire);
        if offset < committed {
            let slot = block * BLOCK + offset;
            // SAFETY: `offset < committed` means the slot was written before the
            // release we just synchronized with, so it is initialized and, being
            // append-only, never mutated again.
            Some(unsafe { (*bucket.slots[slot].get()).assume_init_ref() })
        } else {
            None
        }
    }

    /// Returns the element at `index` without verifying that it was written.
    ///
    /// # Safety
    ///
    /// `index` must have been returned by a previous [`push`](Self::push) whose
    /// effect is visible to the caller through external synchronization (a lock,
    /// or joining the producing thread). Passing an out-of-range index or a
    /// reserved-but-unwritten gap is undefined behavior. Skipping the per-block
    /// commit load makes this cheaper than [`get`](Self::get) on hot read paths.
    pub unsafe fn get_unchecked(&self, index: usize) -> &T {
        let (bucket_index, block, offset) = Self::locate(index);
        let pointer = self.buckets[bucket_index].load(Ordering::Acquire);
        debug_assert!(!pointer.is_null(), "get_unchecked on an unallocated bucket");
        let slot = block * BLOCK + offset;
        // SAFETY: by the contract `index` came from a published `push`, so its
        // bucket is allocated and the slot initialized, and that push
        // happens-before this read; reading the slot is therefore sound.
        unsafe {
            let bucket = &*pointer;
            (*bucket.slots[slot].get()).assume_init_ref()
        }
    }

    /// Returns the number of elements written, summed across threads.
    ///
    /// This walks every thread that has pushed, so it is `O(threads)`; it is a
    /// snapshot only when no push runs concurrently.
    pub fn len(&self) -> usize {
        self.local.iter().map(|local| local.count.load(Ordering::Relaxed)).sum()
    }

    /// Returns true if no element has been written.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterates the written elements in index order, skipping gaps.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        let high_water = self.reserved_blocks.load(Ordering::Acquire) << Self::BLOCK_BITS;
        (0..high_water).filter_map(move |index| self.get(index))
    }

    /// Removes every element, invalidating all previously returned indices.
    pub fn clear(&mut self) {
        for bucket in &mut self.buckets {
            let pointer = *bucket.get_mut();
            if !pointer.is_null() {
                // SAFETY: `&mut self` is exclusive, so reclaiming the owned box
                // (which drops its initialized slots) cannot race.
                unsafe {
                    drop(Box::from_raw(pointer));
                }
                *bucket.get_mut() = null_mut();
            }
        }
        self.reserved_blocks.store(0, Ordering::Relaxed);
        self.local = ThreadLocal::new();
    }

    /// Returns the bucket for `bucket_index`, allocating it on first use.
    fn bucket_or_alloc(&self, bucket_index: usize) -> &Bucket<T> {
        let slot = &self.buckets[bucket_index];
        let pointer = slot.load(Ordering::Acquire);
        if !pointer.is_null() {
            // SAFETY: non-null means a `Box` we allocated and still own.
            return unsafe { &*pointer };
        }

        let block_count = bucket_blocks(bucket_index);
        let allocated = Box::into_raw(Box::new(Bucket::<T>::new(block_count * BLOCK, block_count)));
        match slot.compare_exchange(null_mut(), allocated, Ordering::Release, Ordering::Acquire) {
            // SAFETY: we published `allocated`; it stays valid until `&mut self`.
            Ok(_) => unsafe { &*allocated },
            Err(winner) => {
                // SAFETY: another thread won the race; reclaim our unused
                // allocation and use the published one instead.
                unsafe {
                    drop(Box::from_raw(allocated));
                    &*winner
                }
            }
        }
    }

    /// Splits `index` into its bucket, block offset within the bucket, and offset
    /// within the block.
    fn locate(index: usize) -> (usize, usize, usize) {
        let block = index >> Self::BLOCK_BITS;
        let bucket_index = bucket_of_block(block);
        let block_in_bucket = block - bucket_start_blocks(bucket_index);
        let offset = index & (BLOCK - 1);
        (bucket_index, block_in_bucket, offset)
    }
}

impl<T, const BLOCK: usize> Default for ConcurrentAppendVec<T, BLOCK> {
    fn default() -> ConcurrentAppendVec<T, BLOCK> {
        ConcurrentAppendVec::new()
    }
}

impl<T, const BLOCK: usize> Drop for ConcurrentAppendVec<T, BLOCK> {
    fn drop(&mut self) {
        for bucket in &mut self.buckets {
            let pointer = *bucket.get_mut();
            if !pointer.is_null() {
                // SAFETY: at drop we hold exclusive access; the box drops its
                // initialized slots and frees its storage exactly once.
                unsafe {
                    drop(Box::from_raw(pointer));
                }
            }
        }
    }
}

/// Returns the bucket holding block `block`.
fn bucket_of_block(block: usize) -> usize {
    if block < FIRST_BLOCKS {
        0
    } else {
        (usize::BITS - (block >> FIRST_BITS).leading_zeros()) as usize
    }
}

/// Returns the first block index stored in bucket `bucket`.
fn bucket_start_blocks(bucket: usize) -> usize {
    if bucket == 0 { 0 } else { FIRST_BLOCKS << (bucket - 1) }
}

/// Returns the number of blocks stored in bucket `bucket`.
fn bucket_blocks(bucket: usize) -> usize {
    if bucket == 0 {
        FIRST_BLOCKS
    } else {
        FIRST_BLOCKS << (bucket - 1)
    }
}
