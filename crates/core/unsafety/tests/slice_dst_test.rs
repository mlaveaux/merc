use std::alloc::Layout;
use std::alloc::LayoutError;
use std::ptr::NonNull;
use std::ptr::addr_of_mut;

use allocator_api2::alloc::Global;

use merc_unsafety::AllocatorDst;
use merc_unsafety::SliceDst;
use merc_unsafety::repr_c;

#[repr(C)]
struct WithHeader<T> {
    length: usize,
    array: [T],
}

// SAFETY: `layout_for` computes the `#[repr(C)]` layout of the header
// plus `length` array elements.
unsafe impl<T> SliceDst for WithHeader<T> {
    fn layout_for(length: usize) -> Result<Layout, LayoutError> {
        let header_layout = Layout::new::<usize>();
        let array_layout = Layout::array::<T>(length)?;

        repr_c(&[header_layout, array_layout])
    }

    fn length(&self) -> usize {
        self.length
    }

    fn retype(ptr: NonNull<[()]>) -> NonNull<Self> {
        // SAFETY: casting a pointer preserves non-nullity, and `ptr` is
        // `NonNull`, so `raw_ptr` is never null.
        unsafe {
            let raw_ptr = ptr.as_ptr() as *mut WithHeader<T>;
            NonNull::new_unchecked(raw_ptr)
        }
    }
}

/// The allocation carries the requested element count as slice metadata and is large enough
/// to write the header and every element (miri reports an out-of-bounds write otherwise).
#[test]
fn test_variable_sized_array() {
    let length = 5;
    let ptr = Global
        .allocate_slice_dst::<WithHeader<u64>>(length)
        .expect("Allocation failed in test");

    // SAFETY: `ptr` points to a fresh allocation with the layout of `WithHeader<u64>` for
    // `length` elements; every field is written before the value is read through a reference.
    unsafe {
        let raw = ptr.as_ptr();
        addr_of_mut!((*raw).length).write(length);
        let array = addr_of_mut!((*raw).array) as *mut u64;
        for i in 0..length {
            array.add(i).write(i as u64 * 10);
        }

        let value = ptr.as_ref();
        assert_eq!(value.array.len(), length);
        assert_eq!(value.length(), length);
        assert_eq!(value.array, [0, 10, 20, 30, 40]);
    }

    Global.deallocate_slice_dst(ptr, length);
}
