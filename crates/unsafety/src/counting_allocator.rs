use std::alloc::GlobalAlloc;
use std::alloc::Layout;
use std::alloc::System;
use std::fmt;
use std::ptr::NonNull;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use allocator_api2::alloc::AllocError;
use allocator_api2::alloc::Allocator;

use merc_io::BytesFormatter;

/// An allocator that can be used to count performance metrics
/// on the allocations performed.
pub struct AllocCounter {
    number_of_allocations: AtomicUsize,
    size_of_allocations: AtomicUsize,

    total_number_of_allocations: AtomicUsize,
    total_size_of_allocations: AtomicUsize,

    max_number_of_allocations: AtomicUsize,
    max_size_of_allocations: AtomicUsize,
}

pub struct AllocMetrics {
    number_of_allocations: usize,
    size_of_allocations: usize,

    total_number_of_allocations: usize,
    total_size_of_allocations: usize,

    max_number_of_allocations: usize,
    max_size_of_allocations: usize,
}

impl fmt::Display for AllocMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Current allocations: {} (size: {} bytes)",
            self.number_of_allocations,
            BytesFormatter(self.size_of_allocations)
        )?;
        writeln!(
            f,
            "Total allocations: {} (size: {} bytes)",
            self.total_number_of_allocations,
            BytesFormatter(self.total_size_of_allocations)
        )?;
        write!(
            f,
            "Peak allocations: {} (size: {} bytes)",
            self.max_number_of_allocations,
            BytesFormatter(self.max_size_of_allocations)
        )
    }
}

impl Default for AllocCounter {
    /// Creates a new allocation counter with all metrics initialized to zero
    fn default() -> Self {
        Self::new()
    }
}

impl AllocCounter {
    /// Creates a new allocation counter with all metrics initialized to zero
    pub const fn new() -> Self {
        Self {
            number_of_allocations: AtomicUsize::new(0),
            size_of_allocations: AtomicUsize::new(0),
            total_number_of_allocations: AtomicUsize::new(0),
            total_size_of_allocations: AtomicUsize::new(0),
            max_number_of_allocations: AtomicUsize::new(0),
            max_size_of_allocations: AtomicUsize::new(0),
        }
    }

    /// Returns the performance metrics of the allocator
    pub fn get_metrics(&self) -> AllocMetrics {
        AllocMetrics {
            number_of_allocations: self.number_of_allocations.load(Ordering::Relaxed),
            size_of_allocations: self.size_of_allocations.load(Ordering::Relaxed),

            total_number_of_allocations: self.total_number_of_allocations.load(Ordering::Relaxed),
            total_size_of_allocations: self.total_size_of_allocations.load(Ordering::Relaxed),

            max_number_of_allocations: self.max_number_of_allocations.load(Ordering::Relaxed),
            max_size_of_allocations: self.max_size_of_allocations.load(Ordering::Relaxed),
        }
    }

    /// Resets all current allocation metrics (but preserves total and max metrics)
    pub fn reset(&self) {
        self.number_of_allocations.store(0, Ordering::Relaxed);
        self.size_of_allocations.store(0, Ordering::Relaxed);
    }

    fn alloc(&self, layout: Layout) -> *mut u8 {
        let ret = unsafe { System.alloc(layout) };

        if !ret.is_null() {
            // Update allocation counters atomically
            self.number_of_allocations.fetch_add(1, Ordering::Relaxed);
            self.size_of_allocations.fetch_add(layout.size(), Ordering::Relaxed);

            self.total_number_of_allocations.fetch_add(1, Ordering::Relaxed);
            self.total_size_of_allocations
                .fetch_add(layout.size(), Ordering::Relaxed);

            // Update max counters using compare-and-swap loops
            let current_allocs = self.number_of_allocations.load(Ordering::Relaxed);
            let mut max_allocs = self.max_number_of_allocations.load(Ordering::Relaxed);
            while current_allocs > max_allocs {
                match self.max_number_of_allocations.compare_exchange_weak(
                    max_allocs,
                    current_allocs,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(val) => max_allocs = val,
                }
            }

            let current_size = self.size_of_allocations.load(Ordering::Relaxed);
            let mut max_size = self.max_size_of_allocations.load(Ordering::Relaxed);
            while current_size > max_size {
                match self.max_size_of_allocations.compare_exchange_weak(
                    max_size,
                    current_size,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(val) => max_size = val,
                }
            }
        }

        ret
    }

    fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            System.dealloc(ptr, layout);
        }

        // Update allocation counters atomically
        self.number_of_allocations.fetch_sub(1, Ordering::Relaxed);
        self.size_of_allocations.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for AllocCounter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.dealloc(ptr, layout)
    }
}

unsafe impl Allocator for AllocCounter {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        let ptr = self.alloc(layout);

        if ptr.is_null() {
            return Err(AllocError);
        }

        let slice_ptr = std::ptr::slice_from_raw_parts_mut(ptr, layout.size());
        Ok(NonNull::new(slice_ptr).expect("The resulting ptr will never be null"))
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        self.dealloc(ptr.as_ptr(), layout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn test_basic_allocation_tracking() {
        let counter = AllocCounter::new();
        let metrics = counter.get_metrics();

        // Initially all metrics should be zero
        assert_eq!(metrics.number_of_allocations, 0);
        assert_eq!(metrics.size_of_allocations, 0);
        assert_eq!(metrics.total_number_of_allocations, 0);
        assert_eq!(metrics.total_size_of_allocations, 0);
        assert_eq!(metrics.max_number_of_allocations, 0);
        assert_eq!(metrics.max_size_of_allocations, 0);
    }

    #[test]
    fn test_thread_safety() {
        let counter = Arc::new(AllocCounter::new());
        let num_threads = 4;
        let allocations_per_thread = 1000;

        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let counter = Arc::clone(&counter);
                thread::spawn(move || {
                    for _ in 0..allocations_per_thread {
                        let layout = Layout::from_size_align(64, 8).unwrap();
                        let ptr = counter.alloc(layout);
                        if !ptr.is_null() {
                            counter.dealloc(ptr, layout);
                        }
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        let metrics = counter.get_metrics();

        // After all threads complete, current allocations should be 0
        assert_eq!(metrics.number_of_allocations, 0);
        assert_eq!(metrics.size_of_allocations, 0);

        // Total allocations should equal num_threads * allocations_per_thread
        assert_eq!(
            metrics.total_number_of_allocations,
            num_threads * allocations_per_thread
        );
        assert_eq!(
            metrics.total_size_of_allocations,
            num_threads * allocations_per_thread * 64
        );
    }

    #[test]
    fn test_reset_functionality() {
        let counter = AllocCounter::new();

        // Simulate some allocations
        let layout = Layout::from_size_align(32, 8).unwrap();
        let ptr = counter.alloc(layout);

        let metrics_before = counter.get_metrics();
        assert!(metrics_before.number_of_allocations > 0);

        counter.reset();
        let metrics_after = counter.get_metrics();

        // Current metrics should be reset
        assert_eq!(metrics_after.number_of_allocations, 0);
        assert_eq!(metrics_after.size_of_allocations, 0);

        // Total and max metrics should be preserved
        assert_eq!(
            metrics_after.total_number_of_allocations,
            metrics_before.total_number_of_allocations
        );
        assert_eq!(
            metrics_after.max_number_of_allocations,
            metrics_before.max_number_of_allocations
        );

        // Clean up
        if !ptr.is_null() {
            counter.dealloc(ptr, layout);
        }
    }
}
