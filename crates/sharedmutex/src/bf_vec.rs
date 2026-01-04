//! Authors: Maurice Laveaux, Flip van Spaendonck and Jan Friso Groote

use std::alloc;
use std::alloc::Layout;
use std::cmp::max;
use std::ops::Index;
use std::ptr;
use std::ptr::NonNull;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use crate::BfSharedMutex;

/// An implementation of [Vec<T, A>] based on the [BfSharedMutex] implementation
/// that can be safely send between threads. Elements in the vector can be written
/// concurrently iff type T is [Sync].
pub struct BfVec<T> {
    shared: BfSharedMutex<BfVecShared<T>>,
}

/// The internal shared data of the [BfVec].
pub struct BfVecShared<T> {
    buffer: Option<NonNull<T>>,
    capacity: usize,
    len: AtomicUsize,
}

impl<T> BfVec<T> {
    /// Create a new vector with zero capacity.
    pub fn new() -> BfVec<T> {
        BfVec {
            shared: BfSharedMutex::new(BfVecShared::<T> {
                buffer: None,
                capacity: 0,
                len: AtomicUsize::new(0),
            }),
        }
    }

    /// Append a new element to the vector.
    pub fn push(&self, value: T) {
        let mut read = self.shared.read().unwrap();

        // Reserve an index for the new element.
        let mut last_index = read.len.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        while last_index + 1 >= read.capacity {
            // Vector needs to be resized.
            let new_capacity = max(read.capacity * 2, 8);

            // Make the length consistent and reserve the new size.
            read.len.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            drop(read);
            self.reserve(new_capacity);

            // Acquire read access and try a new position.
            read = self.shared.read().unwrap();
            last_index = read.len.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        // Write the element on the specified index.
        unsafe {
            // We know that the buffer has been allocated by this point.
            let end = read.buffer.unwrap().as_ptr().add(last_index);
            ptr::write(end, value);
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

        let old_layout = Layout::array::<T>(write.capacity).unwrap();
        let layout = Layout::array::<T>(capacity).unwrap();

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

    /// Get access to the underlying data storage.
    fn data(&self) -> *const T {
        self.shared.read().unwrap().buffer.unwrap().as_ptr()
    }
}

impl<T> Default for BfVec<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Index<usize> for BfVec<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        debug_assert!(index < self.len());

        unsafe { &*self.data().add(index) }
    }
}

impl<T> BfVecShared<T> {
    /// Clears the vector by dropping all elements.
    pub fn clear(&mut self) {
        // Only drop items within the 0..len range since the other values are not initialised.
        for i in 0..self.len.load(Ordering::Relaxed) {
            unsafe {
                // We have exclusive access so dropping is safe.
                let ptr = self.buffer.unwrap().as_ptr().add(i);

                ptr::drop_in_place(ptr);
            }
        }
    }
}

impl<T> Drop for BfVecShared<T> {
    fn drop(&mut self) {
        self.clear();

        unsafe {
            // Deallocate the underlying storage.
            let layout = Layout::array::<T>(self.capacity).unwrap();
            if let Some(buffer) = self.buffer {
                alloc::dealloc(buffer.as_ptr() as *mut u8, layout);
            }
        }
    }
}

unsafe impl<T> Send for BfVec<T> {}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    // These are just simple tests.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_push() {
        let mut threads = vec![];

        let shared_vector = BfVec::<u32>::new();
        let num_threads = 10;
        let num_iterations = 100000;

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
            total += shared_vector[i];
        }

        assert_eq!(total, num_threads * (num_threads - 1) * num_iterations / 2);
        assert_eq!(shared_vector.len(), (num_threads * num_iterations) as usize);
    }
}
