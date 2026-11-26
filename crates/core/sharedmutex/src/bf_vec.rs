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
        let mut read = self.shared.read().expect("Lock poisoned!");

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
                read = self.shared.read().expect("Lock poisoned!");
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
            let end = read
                .buffer
                .expect("The buffer is allocated, since last_index < capacity")
                .as_ptr()
                .add(last_index);
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
        self.shared.read().expect("Lock poisoned!").len.load(Ordering::Relaxed)
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
        let read = self.shared.read().expect("Lock poisoned!");

        // Acquire pairs with the publishing store in `push`, so the slots below `len` are initialised.
        assert!(
            index < read.len.load(Ordering::Acquire),
            "Index {index} is out of bounds"
        );

        // SAFETY: `index < len` (Acquire) guarantees the slot is initialised
        // and published, and the read lock keeps the buffer alive for the
        // clone.
        unsafe {
            (*read
                .buffer
                .expect("The buffer is allocated, since index < len")
                .as_ptr()
                .add(index))
            .clone()
        }
    }

    /// Drops the elements in the Vec, but keeps the capacity.
    pub fn clear(&self) {
        let mut write = self.shared.write().expect("Lock poisoned!");
        write.clear();
    }

    /// Reserve the given capacity.
    fn reserve(&self, capacity: usize) {
        let mut write = self.shared.write().expect("Lock poisoned!");

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

        let old_layout =
            Layout::array::<T>(write.capacity).expect("The capacity does not overflow the maximum layout size");
        let layout = Layout::array::<T>(capacity).expect("The capacity does not overflow the maximum layout size");

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
                let ptr = self
                    .buffer
                    .expect("The buffer is allocated, since len > 0")
                    .as_ptr()
                    .add(i);

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
            let layout =
                Layout::array::<T>(self.capacity).expect("The capacity does not overflow the maximum layout size");
            if let Some(buffer) = self.buffer {
                alloc::dealloc(buffer.as_ptr() as *mut u8, layout);
            }
        }
    }
}
// SAFETY: The shared state is accessed concurrently by every thread holding a
// read guard, so the element type must be both `Send` and `Sync` for these
// accesses to be free of data races.
unsafe impl<T: Send + Sync> Send for BfVecShared<T> {}

// SAFETY: A `BfVec` is shared across threads by handing each thread a `share()`
// clone, which requires `T: Send + Sync` (see `BfVecShared` above).
unsafe impl<T: Send + Sync> Send for BfVec<T> {}
