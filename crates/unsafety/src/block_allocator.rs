use std::alloc::Layout;
use std::array;
use std::fmt;
use std::mem::ManuallyDrop;
use std::ptr::NonNull;
use std::ptr::{self};
use std::sync::Mutex;

use allocator_api2::alloc::AllocError;
use allocator_api2::alloc::Allocator;
use itertools::Itertools;

/// This is a slab allocator or also called block allocator for a concrete type `T`. It stores blocks of `Size` to minimize the overhead of individual memory allocations (which are typically in the range of one or two words).
///
/// Behaves like `Allocator`, except that it only allocates for layouts of `T`.
///
/// # Details
///
/// Internally stores blocks of `N` elements
struct BlockAllocator<T, const N: usize> {
    /// This is the block that contains unoccupied entries.
    head_block: Option<Box<Block<T, N>>>,

    /// The start of the freelist
    free: Option<NonNull<Entry<T>>>,
}

impl<T, const N: usize> BlockAllocator<T, N> {
    pub fn new() -> Self {
        Self {
            head_block: None,
            free: None,
        }
    }

    /// Similar to the [Allocator] trait, but instead of passing a layout we allocate just an object of type `T`.
    pub fn allocate_object(&mut self) -> Result<NonNull<T>, AllocError> {
        if let Some(free) = self.free {
            unsafe {
                // Safety: By invariant of the freelist the next must point to the next free element.
                self.free = Some(free.as_ref().next);
            }
            return Ok(free.cast::<T>());
        }

        let block = match &mut self.head_block {
            Some(block) => {
                if block.is_full() {
                    let mut new_block = Box::new(Block::new());
                    std::mem::swap(block, &mut new_block);
                    block.next = Some(new_block);
                }

                block
            }
            None => {
                let block = Box::new(Block::new());
                self.head_block = Some(block);
                self.head_block.as_mut().expect("Is initialized in the previous line")
            }
        };

        // After this the first block definitely has space
        let length = block.length;
        block.length += 1;
        unsafe {
            // Safety: We take a pointer (value does not have to be initialized) to a ManuallDrop<T>, which has the same layout as T.
            Ok(NonNull::new_unchecked(
                &mut block.data[length].data as *mut ManuallyDrop<T> as *mut T,
            ))
        }
    }

    /// Deallocate the given pointer.
    pub fn deallocate_object(&mut self, _ptr: NonNull<T>) {}

    /// Returns an iterator over the free list entries.
    fn iter_free(&self) -> FreeListIterator<T> {
        FreeListIterator { current: self.free }
    }
}

/// A type that can implement `Allocator` using the underlying `BlockAllocator`.
struct AllocBlock<T, const N: usize> {
    block_allocator: Mutex<BlockAllocator<T, N>>,
}

unsafe impl<T, const N: usize> Allocator for AllocBlock<T, N> {
    fn allocate(&self, layout: std::alloc::Layout) -> Result<NonNull<[u8]>, AllocError> {
        debug_assert_eq!(
            layout,
            Layout::new::<T>(),
            "The requested layout should match the type T"
        );

        let ptr = self
            .block_allocator
            .lock()
            .expect("Mutex should not be poisened")
            .allocate_object()?;

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
        self.block_allocator
            .lock()
            .expect("Mutex should not be poisened")
            .deallocate_object(ptr.cast::<T>());
    }
}

union Entry<T> {
    data: ManuallyDrop<T>,
    next: NonNull<Entry<T>>,
}

///
struct Block<T, const N: usize> {
    ///
    data: [Entry<T>; N],
    length: usize,
    next: Option<Box<Block<T, N>>>,
}

impl<T, const N: usize> Block<T, N> {
    fn new() -> Self {
        Self {
            data: array::from_fn(|_i| Entry {
                next: NonNull::dangling(),
            }),
            length: 0,
            next: None,
        }
    }

    fn with_next(next: Box<Block<T, N>>) -> Self {
        Self {
            data: array::from_fn(|_i| Entry {
                next: NonNull::dangling(),
            }),
            length: 0,
            next: Some(next),
        }
    }

    /// Returns true iff this block is full.
    fn is_full(&self) -> bool {
        self.length == N
    }
}

/// Iterator over the free list entries in a BlockAllocator.
struct FreeListIterator<T> {
    current: Option<NonNull<Entry<T>>>,
}

impl<T> Iterator for FreeListIterator<T> {
    type Item = NonNull<Entry<T>>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(current) = self.current {
            // Safety: We assume the free list is properly constructed and current points to a valid Entry
            unsafe {
                self.current = Some(current.as_ref().next);
            }
            Some(current)
        } else {
            None
        }
    }
}

impl<T, const N: usize> fmt::Debug for BlockAllocator<T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "freelist = {:?}", self.iter_free().format(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_block_allocator() {
        let mut allocator: BlockAllocator<usize, 256> = BlockAllocator::new();

        let object = allocator.allocate_object();
    }
}
