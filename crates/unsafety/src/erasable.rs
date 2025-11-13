//! This is adapted from the `erasable` crate, but actually allows one to pass an `?Sized` type that stores its length inline. For example types implementing the `SliceDst` trait.

use std::marker::PhantomData;
use std::ptr::NonNull;

/// A thin, type-erased pointer. This should mimic the interface of NonNull, but
/// with the ability to erase the type information.
#[derive(Clone)]
pub struct Thin<T: ?Sized + Erasable> {
    ptr: ErasedPtr,
    marker: PhantomData<fn() -> T>,
}

impl<T: ?Sized + Erasable + Copy> Copy for Thin<T> {}

impl<T: ?Sized + Erasable> Thin<T> {
    pub fn new(ptr: NonNull<T>) -> Self {
        Self {
            ptr: T::erase(ptr),
            marker: PhantomData,
        }
    }
}

impl<T: ?Sized + Erasable> Thin<T> {
    pub fn as_ptr(&self) -> *mut T {
        unsafe { T::unerase(self.ptr) }.as_ptr()
    }

    pub fn as_nonnull(&self) -> NonNull<T> {
        unsafe { T::unerase(self.ptr) }
    }

    pub unsafe fn as_ref(&self) -> &T {
        unsafe { T::unerase(self.ptr).as_ref() }
    }
}

/// This is the trait that allows a type to be erased and unerased.
pub unsafe trait Erasable {
    /// Turn this erasable pointer into an erased pointer.
    ///
    /// To retrieve the original pointer, use `unerase`.
    ///
    /// # Safety
    ///
    /// The returned erased pointer must only be used with `unerase` for the same type.
    fn erase(this: NonNull<Self>) -> ErasedPtr;

    /// Unerase this erased pointer.
    ///
    /// # Safety
    ///
    /// The erased pointer must have been created by `erase`.
    unsafe fn unerase(this: ErasedPtr) -> NonNull<Self>;
}

unsafe impl<T: Sized> Erasable for T {
    fn erase(this: NonNull<Self>) -> ErasedPtr {
        // If the type is Sized, we can safely cast it to a pointer.
        this.cast::<Erased>().cast()
    }

    unsafe fn unerase(this: ErasedPtr) -> NonNull<Self> {
        // If the type is Sized, we can safely cast it back to a pointer.
        this.cast::<Self>()
    }
}

/// This is simply a u8, but with a concrete type to avoid confusion. Must be a
/// type that has size one and alignment one. Can be converted to an extern type
/// when `extern type` is stabilized.
pub struct Erased(#[allow(unused)] u8);

/// Static assertion to ensure that `ErasedPtr` is the same size as a `usize`.
const _: () = assert!(std::mem::size_of::<ErasedPtr>() == std::mem::size_of::<usize>());

/// A thin, type-erased pointer.
///
/// The `Erased` type is private, and should be treated as an opaque type.
/// When `extern type` is stabilized, `Erased` will be defined as one.
///
/// The current implementation uses a `struct Erased` with size 0 and align 1.
/// If you want to offset the pointer, make sure to cast to a `u8` or other known type pointer first.
/// When `Erased` becomes an extern type, it will properly have unknown size and align.
pub type ErasedPtr = NonNull<Erased>;
