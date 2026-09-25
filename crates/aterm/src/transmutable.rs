use std::collections::VecDeque;
use std::mem::transmute;

use merc_collections::IndexedSet;

use crate::SymbolRef;
use crate::aterm::ATermRef;

/// A trait for transmuting the lifetime of an object to a shorter lifetime.
///
/// # Safety
///
/// Implementors must guarantee, for every `'a` with `Self: 'a`:
///
/// 1. **Layout identity.** `Self` and `Self::Target<'a>` have identical size, alignment, and
///    bit-for-bit representation; the only permitted difference is which lifetime is
///    substituted into reference-shaped fields nested inside (here, always `ATermRef<'_>` /
///    `SymbolRef<'_>`). A lifetime has no runtime representation, so this holds for free as long
///    as the implementation does not also change field order, add/remove fields, or otherwise
///    alter the value's bytes between `Self` and `Self::Target<'a>` -- e.g. every impl in this
///    module is `unsafe { transmute::<&Self, &'a Self::Target<'a>>(self) }` on the nose, which
///    is exactly `mem::transmute`'s own precondition (equal `size_of`/`align_of` on both sides).
///    Violating this makes `transmute_lifetime`/`transmute_lifetime_mut` return a reference that
///    reads as the wrong type at that address: immediate undefined behaviour on the first access
///    through it, independent of anything the *caller* does correctly.
/// 2. **Consistent target.** `Self::Target<'a>` for a shorter `'a` must itself be transmutable
///    (via nested impls, e.g. `Vec<T>` requires `T: Transmutable`) back down to the same
///    physical layout, so composing shrinks (`transmute_lifetime` on a `Target<'a>` down to
///    `'b: 'a`) stays sound by induction on the same argument.
pub unsafe trait Transmutable {
    type Target<'a>: ?Sized
    where
        Self: 'a;

    /// Transmute the lifetime of the object to 'a.
    ///
    /// # Safety
    ///
    /// Requires: every `ATermRef`/`SymbolRef` reachable through `self` must remain a live GC
    /// root -- i.e. registered in some protection set `mark_roots` visits, or otherwise kept
    /// alive by construction (e.g. a `Return`'s read guard) -- for the *entire* duration `'a`,
    /// not merely for the duration of the `&'_ self` borrow used to call this method. Nothing in
    /// the signature ties `'a` to that borrow the way an ordinary safe method's elided lifetime
    /// would, so the call type-checks for any `'a` the caller writes down, including `'static`;
    /// picking one that outlives the actual protection produces a dangling reference the moment
    /// the true underlying lifetime ends, and that mistake is entirely on the caller to avoid.
    ///
    /// Guarantees (given the requirement above is met): the returned `&'a Self::Target<'a>`
    /// aliases the exact same bytes `self` occupies -- no allocation, copy, or transformation
    /// occurs, per property 1 on the trait.
    unsafe fn transmute_lifetime<'a>(&'_ self) -> &'a Self::Target<'a>;

    /// Transmute the lifetime of the object to 'a.
    ///
    /// # Safety
    ///
    /// Same requirement as [`Transmutable::transmute_lifetime`], plus the ordinary exclusivity
    /// requirement any `&mut` carries: the returned `&'a mut Self::Target<'a>` must be the only
    /// live reference (through any lifetime or path) to this value for the whole of `'a`, not
    /// just for the `&'_ mut self` borrow used to call this method.
    unsafe fn transmute_lifetime_mut<'a>(&'_ mut self) -> &'a mut Self::Target<'a>;
}

unsafe impl Transmutable for ATermRef<'static> {
    type Target<'a> = ATermRef<'a>;

    unsafe fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a ATermRef<'a>>(self) }
    }

    unsafe fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut ATermRef<'a>>(self) }
    }
}

unsafe impl Transmutable for SymbolRef<'static> {
    type Target<'a> = SymbolRef<'a>;

    unsafe fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a SymbolRef<'a>>(self) }
    }

    unsafe fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut SymbolRef<'a>>(self) }
    }
}

unsafe impl<T: Transmutable> Transmutable for Option<T>
where
    for<'a> T::Target<'a>: Sized,
{
    type Target<'a>
        = Option<T::Target<'a>>
    where
        T: 'a;

    unsafe fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a Option<T::Target<'a>>>(self) }
    }

    unsafe fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut Option<T::Target<'a>>>(self) }
    }
}

unsafe impl<T: Transmutable> Transmutable for Vec<T>
where
    for<'a> T::Target<'a>: Sized,
{
    type Target<'a>
        = Vec<T::Target<'a>>
    where
        T: 'a;

    unsafe fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a Vec<T::Target<'a>>>(self) }
    }

    unsafe fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut Vec<T::Target<'a>>>(self) }
    }
}

unsafe impl<T: Transmutable> Transmutable for VecDeque<T>
where
    for<'a> T::Target<'a>: Sized,
{
    type Target<'a>
        = VecDeque<T::Target<'a>>
    where
        T: 'a;

    unsafe fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a VecDeque<T::Target<'a>>>(self) }
    }

    unsafe fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut VecDeque<T::Target<'a>>>(self) }
    }
}

unsafe impl<T: Transmutable> Transmutable for IndexedSet<T>
where
    for<'a> T::Target<'a>: Sized,
{
    type Target<'a>
        = IndexedSet<T::Target<'a>>
    where
        T: 'a;

    unsafe fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a IndexedSet<T::Target<'a>>>(self) }
    }

    unsafe fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut IndexedSet<T::Target<'a>>>(self) }
    }
}

// In Rust Its not yet possible to implement it for any tuples, so we implement it for some common sizes.
unsafe impl<T1: Transmutable, T2: Transmutable> Transmutable for (T1, T2)
where
    for<'a> T1::Target<'a>: Sized,
    for<'a> T2::Target<'a>: Sized,
{
    type Target<'a>
        = (T1::Target<'a>, T2::Target<'a>)
    where
        T1: 'a,
        T2: 'a;

    unsafe fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a (T1::Target<'a>, T2::Target<'a>)>(self) }
    }

    unsafe fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut (T1::Target<'a>, T2::Target<'a>)>(self) }
    }
}

unsafe impl<T: Transmutable> Transmutable for [T]
where
    for<'a> T::Target<'a>: Sized,
{
    type Target<'a>
        = [T::Target<'a>]
    where
        T: 'a;

    unsafe fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a [T::Target<'a>]>(self) }
    }

    unsafe fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut [T::Target<'a>]>(self) }
    }
}

unsafe impl Transmutable for bool {
    type Target<'a> = bool;

    unsafe fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a bool>(self) }
    }

    unsafe fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut bool>(self) }
    }
}
