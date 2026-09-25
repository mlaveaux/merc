// Authors: Maurice Laveaux, Flip van Spaendonck and Jan Friso Groote

use std::alloc;
use std::alloc::Layout;
use std::cmp::max;
use std::hint::spin_loop;
use std::ptr;
use std::ptr::NonNull;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use crate::BfSharedMutex;

/// A vector that can be safely sent between and appended to concurrently by threads holding a
/// clone made with `share`.
///
/// Zero-sized element types are not supported.
pub struct BfVec<T> {
    shared: BfSharedMutex<BfVecShared<T>>,
}

/// Shared state behind a `BfVec`, protected by a `BfSharedMutex`.
pub struct BfVecShared<T> {
    buffer: Option<NonNull<T>>,
    capacity: usize,

    /// The number of slots reserved by writers; can exceed `len` while pushes
    /// are in flight.
    reserved: AtomicUsize,

    /// The number of initialised elements; reads may only access slots below
    /// this bound.
    len: AtomicUsize,
}

impl<T> BfVec<T> {
    /// Create a new vector with zero capacity.
    pub fn new() -> BfVec<T> {
        const {
            assert!(std::mem::size_of::<T>() > 0, "Zero-sized types are not supported");
        }

        BfVec {
            shared: BfSharedMutex::new(BfVecShared::<T> {
                buffer: None,
                capacity: 0,
                reserved: AtomicUsize::new(0),
                len: AtomicUsize::new(0),
            }),
        }
    }

    /// Append a new element to the vector.
    pub fn push(&self, value: T) {
        let mut read = self.shared.read().unwrap();

        // Reserve an index for the new element with a compare-and-swap so that a reservation
        // never has to be rolled back; a rollback could hand the same slot to two threads.
        let last_index = loop {
            let current = read.reserved.load(Ordering::Relaxed);

            if current >= read.capacity {
                // Vector needs to be resized.
                let new_capacity = max(read.capacity * 2, 8);

                drop(read);
                self.reserve(new_capacity);

                // Acquire read access and try a new position.
                read = self.shared.read().unwrap();
                continue;
            }

            if read
                .reserved
                .compare_exchange_weak(current, current + 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                break current;
            }
        };

        // Write the element on the specified index.

        // SAFETY: The CAS above reserved `last_index < capacity` while holding
        // the read lock, so the buffer is allocated and the slot is exclusively
        // ours and uninitialised.
        unsafe {
            let end = read.buffer.unwrap().as_ptr().add(last_index);
            ptr::write(end, value);
        }

        // Publish the element. `len` may only be raised past our slot once all
        // earlier slots have been initialised, so wait for the slots below us.
        while read
            .len
            .compare_exchange_weak(last_index, last_index + 1, Ordering::Release, Ordering::Relaxed)
            .is_err()
        {
            spin_loop();
        }
    }

    /// Obtain another view on the vector to share among threads.
    pub fn share(&self) -> BfVec<T> {
        BfVec {
            shared: self.shared.clone(),
        }
    }

    /// Obtain the number of elements in the vector.
    pub fn len(&self) -> usize {
        self.shared.read().unwrap().len.load(Ordering::Relaxed)
    }

    /// Returns true iff the vector is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns a clone of the element at the given index.
    ///
    /// The value is cloned while holding the read lock; handing out a
    /// reference instead would let it dangle when another thread's `push`
    /// reallocates the buffer.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds.
    pub fn at(&self, index: usize) -> T
    where
        T: Clone,
    {
        let read = self.shared.read().unwrap();

        // Acquire pairs with the publishing store in `push`, so the slots below `len` are initialised.
        assert!(
            index < read.len.load(Ordering::Acquire),
            "Index {index} is out of bounds"
        );

        // SAFETY: `index < len` (Acquire) guarantees the slot is initialised
        // and published, and the read lock keeps the buffer alive for the
        // clone.
        unsafe { (*read.buffer.unwrap().as_ptr().add(index)).clone() }
    }

    /// Drops the elements in the Vec, but keeps the capacity.
    pub fn clear(&self) {
        let mut write = self.shared.write().unwrap();
        write.clear();
    }

    /// Reserve the given capacity.
    fn reserve(&self, capacity: usize) {
        let mut write = self.shared.write().unwrap();

        // A reserve could have happened in the meantime which makes this call obsolete
        if capacity <= write.capacity {
            return;
        }

        // Reservation, write and publish all happen under the read lock, so once the write lock
        // is held no push is in flight and every reserved slot has been initialised.
        debug_assert_eq!(
            write.reserved.load(Ordering::Relaxed),
            write.len.load(Ordering::Relaxed),
            "All reserved slots must be published before a resize"
        );

        let old_layout = Layout::array::<T>(write.capacity).unwrap();
        let layout = Layout::array::<T>(capacity).unwrap();

        // SAFETY: Held under the write lock, so no push is in flight and every reserved slot is
        // initialised. We allocate a larger buffer, move the `len` live elements into it, and
        // free the old allocation with the layout it was created with.
        unsafe {
            let new_buffer = alloc::alloc(layout) as *mut T;
            if new_buffer.is_null() {
                alloc::handle_alloc_error(layout);
            }

            if let Some(old_buffer) = write.buffer {
                debug_assert!(
                    write.len.load(Ordering::Relaxed) <= write.capacity,
                    "Length {} should be less than capacity {}",
                    write.len.load(Ordering::Relaxed),
                    write.capacity
                );

                ptr::copy_nonoverlapping(old_buffer.as_ptr(), new_buffer, write.len.load(Ordering::Relaxed));

                // Clean up the old buffer.
                alloc::dealloc(old_buffer.as_ptr() as *mut u8, old_layout);
            }

            write.capacity = capacity;
            write.buffer = NonNull::new(new_buffer);
        }
    }
}

impl<T> Default for BfVec<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> BfVecShared<T> {
    /// Clears the vector by dropping all elements.
    pub fn clear(&mut self) {
        // Only drop items within the 0..len range since the other values are not initialised.
        for i in 0..self.len.load(Ordering::Relaxed) {
            // SAFETY: `&mut self` gives exclusive access, and slots `0..len` are initialised, so
            // each element can be dropped exactly once in place.
            unsafe {
                let ptr = self.buffer.unwrap().as_ptr().add(i);

                ptr::drop_in_place(ptr);
            }
        }

        // Reset the length so the dropped elements cannot be observed or dropped a second time.
        self.len.store(0, Ordering::Relaxed);
        self.reserved.store(0, Ordering::Relaxed);
    }
}

impl<T> Drop for BfVecShared<T> {
    fn drop(&mut self) {
        self.clear();

        // SAFETY: `clear` dropped every live element, so the buffer holds no initialised values;
        // we free it with the same layout it was allocated with.
        unsafe {
            let layout = Layout::array::<T>(self.capacity).unwrap();
            if let Some(buffer) = self.buffer {
                alloc::dealloc(buffer.as_ptr() as *mut u8, layout);
            }
        }
    }
}
// SAFETY: `BfVecShared<T>` is not auto-`Send` because `buffer: Option<NonNull<T>>` is a raw
// pointer, never auto-`Send`/`Sync` regardless of `T`.
//
// Contract discharged by this impl, given `T: Send + Sync`:
//   - `buffer` points to a `capacity`-element allocation of `T` that every thread holding a
//     `BfSharedMutex<BfVecShared<T>>` clone can reach: `push` writes a fresh, exclusively-owned
//     slot (`&self.buffer`'s slot at the CAS-reserved index) from behind a read lock, and `at`
//     reads a published slot (`index < len`, Acquire) from behind a read lock, on whichever
//     thread calls them — so elements may be written by one thread and read by another,
//     requiring `T: Send`; `at` clones a slot's `T` while other threads may concurrently hold
//     `&T` to slots they have already published, requiring `T: Sync`.
//   - `capacity`/`reserved`/`len` are `usize`/`AtomicUsize`, always `Send`.
//   - `Drop` for `BfVecShared<T>` runs on whichever thread drops the last reference and drops
//     every live `T` in place; `T: Send` covers a value being dropped by a thread other than the
//     one that wrote it.
unsafe impl<T: Send + Sync> Send for BfVecShared<T> {}

// SAFETY: `BfVec<T>` has one field, `shared: BfSharedMutex<BfVecShared<T>>`; it inherits its
// non-auto-`Send` status from `BfSharedMutex`'s own (see that type's `unsafe impl Send`).
//
// Note: `BfSharedMutex<U>: Send` (the impl this delegates to) is only *manually* asserted for
// `U: Send + Sync`, and the compiler does not itself re-check that `U = BfVecShared<T>`
// satisfies that bound when accepting this outer `unsafe impl` — no `unsafe impl Sync for
// BfVecShared<T>` exists in this file, and `BfVecShared<T>` does not auto-derive `Sync` either
// (blocked by the same `buffer: Option<NonNull<T>>` field noted above). The soundness of *this*
// impl therefore additionally relies on `&BfVecShared<T>` being safe to observe concurrently
// from multiple threads, which is true by construction even without a named `Sync` impl:
// `buffer`/`capacity` are read only from behind a `BfSharedMutex` read lock and mutated only
// from behind its write lock (mutually exclusive with every read lock by the busy-forbidden
// protocol), while `reserved`/`len` are plain atomics designed to be raced on by concurrent
// readers. Given `T: Send + Sync` (so cloning/reading/dropping the elements a `BfVecShared<T>`
// stores is itself sound, per the impl above), moving a `BfVec<T>`'s `shared` clone to another
// thread is sound.
unsafe impl<T: Send + Sync> Send for BfVec<T> {}

#[cfg(test)]
mod tests {
    use std::thread;

    use crate::BfVec;

    // These are just simple tests.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_push() {
        let mut threads = vec![];

        let shared_vector = BfVec::<u32>::new();
        let num_threads = 10;
        let num_iterations = 10000;

        for t in 0..num_threads {
            let shared_vector = shared_vector.share();
            threads.push(thread::spawn(move || {
                for _ in 0..num_iterations {
                    shared_vector.push(t);
                }
            }));
        }

        // Check whether threads have completed successfully.
        for thread in threads {
            thread.join().unwrap();
        }

        // Check the vector for some kind of consistency, correct total
        let mut total = 0;
        for i in 0..shared_vector.len() {
            total += shared_vector.at(i);
        }

        assert_eq!(total, num_threads * (num_threads - 1) * num_iterations / 2);
        assert_eq!(shared_vector.len(), (num_threads * num_iterations) as usize);
    }

    /// Boundary of the very first reserve (`capacity` goes from 0, so `new_capacity =
    /// max(0 * 2, 8) = 8`): exercises the empty-buffer branch of `reserve` (no old allocation to
    /// copy from or free) and checks every slot up to and including the boundary index that
    /// triggers the *second* reserve (`capacity` 8 -> 16).
    #[test]
    fn test_first_reserve_boundary() {
        let vector = BfVec::<u32>::new();
        assert!(vector.is_empty());

        for i in 0..8u32 {
            vector.push(i);
        }
        assert_eq!(vector.len(), 8);
        for i in 0..8u32 {
            assert_eq!(vector.at(i as usize), i);
        }

        // One more push crosses into the second reserve (8 -> 16), exercising the
        // non-empty branch of `reserve` (copy + free the old allocation).
        vector.push(99);
        assert_eq!(vector.len(), 9);
        assert_eq!(vector.at(8), 99);
        for i in 0..8u32 {
            assert_eq!(vector.at(i as usize), i, "elements below the boundary must survive the resize");
        }
    }

    #[test]
    fn test_clear_resets_length() {
        let vector = BfVec::<String>::new();
        vector.push("a".to_string());
        vector.push("b".to_string());
        assert_eq!(vector.len(), 2);

        // Dropping the vector afterwards must not drop the elements again.
        vector.clear();
        assert!(vector.is_empty());

        vector.push("c".to_string());
        assert_eq!(vector.at(0), "c");
    }
}
