use std::ptr::NonNull;
use std::sync::Arc;

use rand::RngExt;

use merc_unsafety::BlockAllocator;
use merc_unsafety::BlockAllocatorSafe;
use merc_utilities::random_test;

/// A value stored in the block allocator in these tests.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Value(usize);

// SAFETY: `Value` is a single fully initialized `usize`, and the tests never store the sentinel
// `usize::MAX` in it.
unsafe impl BlockAllocatorSafe for Value {}

#[test]
#[cfg_attr(miri, ignore)]
fn test_block_allocator() {
    random_test(100, |rng| {
        let mut allocator: BlockAllocator<Value, 32> = BlockAllocator::new();

        // Allocate 1000 elements and keep track of ptr to value mapping.
        let mut allocated: Vec<(NonNull<Value>, Value)> = Vec::new();
        for _ in 0..1000 {
            let ptr = allocator.allocate_object().unwrap();
            let value = Value(rng.random_range(0..=usize::MAX - 1));
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
        let mut deallocated = 0;
        for (ptr, value) in allocated {
            if rng.random_bool(0.5) {
                allocator.deallocate_object(ptr);
                deallocated += 1;
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

        let (_removed, free_size) = allocator.remove_free_blocks();
        assert_eq!(free_size, deallocated, "every deallocated object was on a free list");

        for _ in 0..500 {
            let ptr = allocator.allocate_object().unwrap();
            assert!(remaining.iter().all(|(live_ptr, _)| *live_ptr != ptr));
            let value = Value(rng.random_range(0..=usize::MAX - 1));
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
fn test_released_objects_are_reclaimed() {
    let mut allocator: BlockAllocator<Value, 8> = BlockAllocator::new();

    let mut live = Vec::new();
    for value in 0..64 {
        let ptr = allocator.allocate_object().unwrap();
        // SAFETY: `ptr` was just returned by `allocate_object` and is owned by this test.
        unsafe { ptr.as_ptr().write(Value(value)) };

        // Free every block's first half in either way, and keep its second half.
        match value % 8 {
            // SAFETY: `ptr` was just allocated and is not used afterwards.
            0..2 => unsafe { allocator.release_object(ptr) },
            2..4 => allocator.deallocate_object(ptr),
            _ => live.push((ptr, Value(value))),
        }
    }

    let (removed, _) = allocator.remove_free_blocks();
    assert_eq!(removed, 0, "every block still holds live objects");

    // The 32 freed slots are reused before new blocks are allocated, never a live slot.
    for value in 64..96 {
        let ptr = allocator.allocate_object().unwrap();
        assert!(live.iter().all(|(live_ptr, _)| *live_ptr != ptr));
        // SAFETY: `ptr` was just returned by `allocate_object` and is owned by this test.
        unsafe { ptr.as_ptr().write(Value(value)) };
        live.push((ptr, Value(value)));
    }

    for (ptr, expected) in &live {
        // SAFETY: `ptr` was not freed, so it still points to a live, initialized slot.
        unsafe { assert_eq!(*ptr.as_ref(), *expected) };
    }

    for (ptr, _) in live {
        // SAFETY: `ptr` is still allocated and is not used afterwards.
        unsafe { allocator.release_object(ptr) };
    }
    let (removed, _) = allocator.remove_free_blocks();
    assert_eq!(removed, 8, "all blocks are empty");
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "released twice")]
fn test_double_release_is_detected() {
    let allocator: BlockAllocator<Value, 8> = BlockAllocator::new();
    let ptr = allocator.allocate_object().unwrap();
    // SAFETY: The first release is valid; the second violates the contract on purpose to
    // check that the debug assertion catches it before anything is written.
    unsafe {
        allocator.release_object(ptr);
        allocator.release_object(ptr);
    }
}

/// Threads allocate concurrently, each writing values unique to itself, so an object handed
/// out to two threads at once shows up as a wrong value.
#[test]
#[cfg_attr(miri, ignore)]
fn test_block_allocator_parallel_freelist() {
    let block_allocator = Arc::new(BlockAllocator::<u32, 32>::new());

    let threads: Vec<_> = (0..3u32)
        .map(|thread| {
            let block_allocator = block_allocator.clone();

            std::thread::spawn(move || {
                for round in 0..10u32 {
                    let mut ptrs = Vec::new();
                    for i in 0..100u32 {
                        let value = thread * 1_000_000 + round * 1000 + i;
                        let ptr = block_allocator.allocate_object().unwrap();
                        // SAFETY: `ptr` was just returned by `allocate_object`
                        // and points to a freshly-allocated, uninitialized
                        // slot exclusively owned by this thread until
                        // deallocated.
                        unsafe {
                            ptr.as_ptr().write(value);
                        }
                        ptrs.push((ptr, value));
                    }

                    for (ptr, value) in ptrs {
                        // SAFETY: `ptr` was not yet deallocated, so it still
                        // points to a live, initialized slot owned by this
                        // thread.
                        unsafe {
                            assert_eq!(*ptr.as_ref(), value);
                        }
                        block_allocator.deallocate_object(ptr);
                    }
                }
            })
        })
        .collect();

    for thread in threads {
        thread.join().unwrap();
    }
}
