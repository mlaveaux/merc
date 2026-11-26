//! Adapted from the `slice-dst` crate.

use std::alloc::Layout;
use std::alloc::LayoutError;
use std::ptr::NonNull;
use std::ptr::slice_from_raw_parts_mut;

use allocator_api2::alloc::AllocError;
use allocator_api2::alloc::Allocator;

/// Lets a dynamically sized type be constructed and dropped behind a
/// type-erased, thin pointer by reconstructing its slice-length metadata on
/// demand instead of storing a wide pointer.
///
/// # Safety
///
/// `layout_for(length)` must return the exact layout a value with that many
/// elements was allocated with. `retype` must turn an untyped pointer of that
/// layout into a valid `Self`, and `length` must report the element count the
/// layout was computed from. Any mismatch causes undefined behaviour on
/// access or deallocation.
pub unsafe trait SliceDst {
    /// Returns the layout of the slice containing `length` elements for this DST.
    fn layout_for(length: usize) -> Result<Layout, LayoutError>;

    /// Reinterprets an untyped slice pointer, whose length is `Self`'s element
    /// count, as a pointer to `Self`.
    fn retype(ptr: NonNull<[()]>) -> NonNull<Self>;

    /// Returns the number of elements in this DST, needed to reconstruct its
    /// layout on deallocation.
    fn length(&self) -> usize;
}

/// Blanket implemented for Sized T.
/// # Safety
///
/// `length` always reports `0`, so the two agree with each
/// other for every call.
unsafe impl<T> SliceDst for T {
    fn layout_for(_length: usize) -> Result<Layout, LayoutError> {
        Ok(Layout::new::<T>())
    }

    fn retype(ptr: NonNull<[()]>) -> NonNull<Self> {
        // SAFETY: casting a pointer preserves non-nullity.
        unsafe {
            let raw_ptr = ptr.as_ptr() as *mut Self;
            NonNull::new_unchecked(raw_ptr)
        }
    }

    fn length(&self) -> usize {
        0
    }
}

/// Computes the `#[repr(C)]` layout for a struct whose fields have the given
/// layouts, in declaration order and padded to the composite's alignment.
///
/// # Errors
///
/// Returns [`LayoutError`] if the combined size would overflow `isize`.
pub fn repr_c<const N: usize>(fields: &[Layout; N]) -> Result<Layout, LayoutError> {
    let mut layout = Layout::from_size_align(0, 1)?;
    for &field in fields {
        let (new_layout, _offset) = layout.extend(field)?;
        layout = new_layout;
    }

    // Remember to finalize with `pad_to_align`!
    Ok(layout.pad_to_align())
}

/// Extends an [`Allocator`] with the ability to allocate and deallocate
/// dynamically sized slices implementing [`SliceDst`].
///
/// # Safety
///
/// Implementors must allocate and deallocate using the layout
/// `T::layout_for(length)` computes, matching what [`SliceDst::retype`] and
/// [`SliceDst::length`] expect for the returned pointer.
pub unsafe trait AllocatorDst {
    /// Allocates uninitialized memory sized and aligned for `length` elements of `T`.
    ///
    /// # Errors
    ///
    /// Returns [`AllocError`] if the allocation fails.
    fn allocate_slice_dst<T: SliceDst + ?Sized>(&self, length: usize) -> Result<NonNull<T>, AllocError>;

    /// Deallocates memory previously returned by
    /// [`allocate_slice_dst`](Self::allocate_slice_dst) for the same `T` and `length`.
    fn deallocate_slice_dst<T: ?Sized + SliceDst>(&self, ptr: NonNull<T>, length: usize);
}

// SAFETY: `allocate_slice_dst` allocates using `T::layout_for(length)` and
// `deallocate_slice_dst` deallocates using that same `T::layout_for(length)`
// for the caller-supplied `length.
unsafe impl<A: Allocator> AllocatorDst for A {
    fn allocate_slice_dst<T: SliceDst + ?Sized>(&self, length: usize) -> Result<NonNull<T>, AllocError> {
        let ptr = self.allocate(T::layout_for(length).expect("Invalid layout for SliceDst"))?;
        // Create a slice of the correct length for proper metadata
        // SAFETY: `ptr` is `NonNull`, so the cast data pointer is never null,
        // making the resulting slice pointer non-null too.
        let slice_ptr = unsafe { NonNull::new_unchecked(slice_from_raw_parts_mut(ptr.as_ptr() as *mut (), length)) };
        Ok(T::retype(slice_ptr))
    }

    fn deallocate_slice_dst<T: ?Sized + SliceDst>(&self, ptr: NonNull<T>, length: usize) {
        // SAFETY: `ptr` is `NonNull`, so the cast byte pointer is never null.
        unsafe {
            self.deallocate(
                NonNull::new_unchecked(ptr.as_ptr() as *mut u8),
                T::layout_for(length).expect("Invalid layout for SliceDst"),
            );
        }
    }
}

#[cfg(kani)]
mod verification {
    use super::*;

    #[kani::proof]
    fn sized_type_layout_matches_layout_new() {
        let layout_zero = <u32 as SliceDst>::layout_for(0).expect("layout for sized must succeed");
        assert_eq!(layout_zero, Layout::new::<u32>());

        let n: usize = kani::any();
        let layout_n = <u32 as SliceDst>::layout_for(n).expect("layout for sized ignores length");
        assert_eq!(layout_n, Layout::new::<u32>());
    }

    #[kani::proof]
    fn sized_type_length_is_zero() {
        let value: u32 = kani::any();
        assert_eq!(<u32 as SliceDst>::length(&value), 0);
    }

    #[kani::proof]
    #[kani::unwind(5)]
    fn repr_c_is_at_least_as_large_as_any_field() {
        let align_a: u32 = kani::any();
        let align_b: u32 = kani::any();
        kani::assume(align_a.is_power_of_two());
        kani::assume(align_b.is_power_of_two());

        let size_a: usize = kani::any();
        let size_b: usize = kani::any();
        // Keep both fields comfortably inside `isize::MAX` so the padded
        // composite layout has room to be constructed without overflowing.
        kani::assume(size_a <= isize::MAX as usize / 4);
        kani::assume(size_b <= isize::MAX as usize / 4);

        let Ok(a) = Layout::from_size_align(size_a, align_a as usize) else {
            return;
        };
        let Ok(b) = Layout::from_size_align(size_b, align_b as usize) else {
            return;
        };
        let Ok(composite) = repr_c(&[a, b]) else {
            return;
        };

        // pad_to_align preserves the maximum field alignment.
        assert!(composite.align() >= a.align());
        assert!(composite.align() >= b.align());
        // The composite must be large enough to hold both fields back-to-back.
        assert!(composite.size() >= a.size() + b.size());
    }
}
