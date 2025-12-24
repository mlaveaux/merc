use std::collections::VecDeque;
use std::mem::transmute;

use merc_collections::IndexedSet;

use crate::SymbolRef;
use crate::aterm::ATermRef;

pub trait Transmutable {
    type Target<'a>
    where
        Self: 'a;

    /// Transmute the lifetime of the object to 'a, which is shorter than the given lifetime.
    fn transmute_lifetime<'a>(&'_ self) -> &'a Self::Target<'a>;

    /// Transmute the lifetime of the object to 'a, which is shorter than the given lifetime.
    fn transmute_lifetime_mut<'a>(&'_ mut self) -> &'a mut Self::Target<'a>;
}

impl Transmutable for ATermRef<'static> {
    type Target<'a> = ATermRef<'a>;

    fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a ATermRef<'a>>(self) }
    }

    fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut ATermRef<'a>>(self) }
    }
}

impl Transmutable for SymbolRef<'static> {
    type Target<'a> = SymbolRef<'a>;

    fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a SymbolRef<'a>>(self) }
    }

    fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut SymbolRef<'a>>(self) }
    }
}

impl<T: Transmutable> Transmutable for Option<T> {
    type Target<'a>
        = Option<T>
    where
        T: 'a;

    fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a Option<T>>(self) }
    }

    fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut Option<T>>(self) }
    }
}

impl<T: Transmutable> Transmutable for Vec<T> {
    type Target<'a>
        = Vec<T::Target<'a>>
    where
        T: 'a;

    fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a Vec<T::Target<'a>>>(self) }
    }

    fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut Vec<T::Target<'a>>>(self) }
    }
}

impl<T: Transmutable> Transmutable for VecDeque<T> {
    type Target<'a>
        = VecDeque<T::Target<'a>>
    where
        T: 'a;

    fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a VecDeque<T::Target<'a>>>(self) }
    }

    fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut VecDeque<T::Target<'a>>>(self) }
    }
}

impl<T: Transmutable> Transmutable for IndexedSet<T> {
    type Target<'a>
        = IndexedSet<T::Target<'a>>
    where
        T: 'a;

    fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a IndexedSet<T::Target<'a>>>(self) }
    }

    fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut IndexedSet<T::Target<'a>>>(self) }
    }
}

// In Rust Its not yet possible to implement it for any tuples, so we implement it for some common sizes.
impl<T1: Transmutable, T2: Transmutable> Transmutable for (T1, T2) {
    type Target<'a>
        = (T1::Target<'a>, T2::Target<'a>)
    where
        T1: 'a,
        T2: 'a;

    fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a (T1::Target<'a>, T2::Target<'a>)>(self) }
    }

    fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut (T1::Target<'a>, T2::Target<'a>)>(self) }
    }
}

impl Transmutable for bool {
    type Target<'a> = bool;

    fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        unsafe { transmute::<&Self, &'a bool>(self) }
    }

    fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        unsafe { transmute::<&mut Self, &'a mut bool>(self) }
    }
}
