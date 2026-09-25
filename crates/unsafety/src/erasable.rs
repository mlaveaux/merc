//! This is adapted from the `erasable` crate, but actually allows one to pass an `?Sized` type that stores its length inline. For example types implementing the `SliceDst` trait.

use std::marker::PhantomData;
use std::ptr::NonNull;

/// A thin, type-erased pointer that stays pointer-sized even when `T` is an
/// unsized type whose length is stored in the pointee rather than in a fat
/// pointer's metadata.
///
/// `repr(transparent)` guarantees the same layout and niche as the underlying
/// [`ErasedPtr`], so `Option<Thin<T>>` stays pointer-sized.
#[repr(transparent)]
pub struct Thin<T: ?Sized + Erasable> {
    ptr: ErasedPtr,
    marker: PhantomData<fn() -> T>,
}

// A `Thin` is just a type-erased pointer plus a `PhantomData`, so it is always
// `Copy`/`Clone` regardless of whether the pointee is.
impl<T: ?Sized + Erasable> Clone for Thin<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized + Erasable> Copy for Thin<T> {}

impl<T: ?Sized + Erasable> Thin<T> {
    pub fn new(ptr: NonNull<T>) -> Self {
        Self {
            ptr: T::erase(ptr),
            marker: PhantomData,
        }
    }
}

impl<T: ?Sized + Erasable> Thin<T> {
    /// Returns the raw erased address without reconstructing the fat pointer.
    ///
    /// Use this for identity operations (comparison, hashing) that only need the
    /// address, avoiding the `unerase` memory read that `as_ptr`/`as_ref` incur
    /// for slice DSTs.
    pub fn as_erased(&self) -> ErasedPtr {
        self.ptr
    }

    pub fn as_nonnull(&self) -> NonNull<T> {
        // SAFETY: `self.ptr` was produced by `T::erase` in `Thin::new`, the only
        // way to construct a `Thin<T>`, so it satisfies `unerase`'s precondition.
        unsafe { T::unerase(self.ptr) }
    }

    /// # Safety
    ///
    /// The caller must ensure that the underlying pointer is valid for reads.
    pub unsafe fn as_ref(&self) -> &T {
        // SAFETY: `self.ptr` was produced by `T::erase`, satisfying `unerase`'s
        // preconditio.
        unsafe { T::unerase(self.ptr).as_ref() }
    }
}

/// This is the trait that allows a type to be erased and unerased.
///
/// # Safety
///
/// See the documentation of the trait functions.
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

// SAFETY: for a `Sized` `T`, `erase`/`unerase` are inverse pointer casts with
// no metadata to lose, so `unerase(erase(p)) == p` always holds, satisfying
// the trait's round-trip contract.
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

/// A stand-in for an opaque pointee with size one and alignment one (a single
/// `u8`). Can be replaced by an `extern type` when that is stabilized.
pub struct Erased(#[allow(unused)] u8);

/// Static assertion to ensure that `ErasedPtr` is the same size as a `usize`.
const _: () = assert!(std::mem::size_of::<ErasedPtr>() == std::mem::size_of::<usize>());

/// A thin, type-erased pointer.
///
/// The `Erased` type is private, and should be treated as an opaque type.
/// When `extern type` is stabilized, `Erased` will be defined as one.
///
/// The current implementation uses a `struct Erased` of size 1 and align 1.
/// If you want to offset the pointer, make sure to cast to a `u8` or other known type pointer first.
/// When `Erased` becomes an extern type, it will properly have unknown size and align.
pub type ErasedPtr = NonNull<Erased>;

#[cfg(kani)]
mod verification {
    use super::*;

    /// Checks the `Erasable` trait's own round-trip contract (`# Safety` on
    /// [`Erasable::unerase`]) for the blanket `T: Sized` impl: for every non-null, well-aligned
    /// pointer `p: NonNull<T>`, `unerase(erase(p)) == p`. `erase`/`unerase` for `Sized` types are
    /// plain pointer casts (no metadata to lose), so this holds for arbitrary `T` without
    /// touching the pointee — verified here for `u64` as a representative sized type.
    #[kani::proof]
    fn erase_unerase_roundtrip_is_identity() {
        let mut value: u64 = kani::any();
        let ptr = NonNull::from(&mut value);

        let erased = <u64 as Erasable>::erase(ptr);
        // SAFETY: `erased` was just produced by `erase` on the same type, satisfying
        // `unerase`'s precondition.
        let unerased = unsafe { <u64 as Erasable>::unerase(erased) };

        assert_eq!(unerased, ptr);
    }

    /// `Thin::new`/`Thin::as_nonnull` must round-trip through the same `erase`/`unerase` pair,
    /// so a `Sized` pointee's address survives being stored in a `Thin<T>`.
    #[kani::proof]
    fn thin_new_as_nonnull_roundtrip_is_identity() {
        let mut value: u32 = kani::any();
        let ptr = NonNull::from(&mut value);

        let thin = Thin::new(ptr);
        assert_eq!(thin.as_nonnull(), ptr);
    }
}
