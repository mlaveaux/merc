
use std::hash::RandomState;
use std::hash::Hash;

use dashmap::Equivalent;
use mcrl3_unsafety::{AllocBlock, BlockAllocator, StablePointer, StablePointerSet};
use rustc_hash::FxBuildHasher;

use crate::{ATerm, ATermIndex, SharedTerm, SymbolRef};

/// Storage for ATerms with a fixed number of arguments.
///
/// Should be the same layout as `SharedTerm`.
#[repr(C)]
#[derive(Hash, Eq, PartialEq)]
struct SharedTermFixed<const N: usize> {
    symbol: SymbolRef<'static>,
    args: [ATermIndex; N],
}

/// Storage for ATerms with a fixed number of arguments.
///
/// Should be the same layout as `SharedTerm`.
#[repr(C)]
#[derive(Hash, Eq, PartialEq)]
pub(crate) struct SharedTermInt {
    symbol: SymbolRef<'static>,
    args: [usize; 1],
}

pub(crate) struct ATermStorage {
    terms: StablePointerSet<SharedTerm>,

    int_terms: StablePointerSet<SharedTermFixed<1>, RandomState, AllocBlock<SharedTermFixed<1>, 1024>>,
}

impl ATermStorage {
    pub fn new() -> Self {
        Self {
            terms: StablePointerSet::new(),
            int_terms: StablePointerSet::with_capacity_in(1000, AllocBlock::new()),
        }
    }

    /// Returns the number of stored terms.
    pub fn len(&self) -> usize {
        self.int_terms.len() +
        self.terms.len()
    }

    pub fn retain<F>(&self, mut f: F)
    where
        F: FnMut(&StablePointer<SharedTerm>) -> bool,
    {
        // self.int_terms.retain(|term| f(term));
        self.terms.retain(|term| f(term));
    }

    pub unsafe fn insert_equiv_dst<'a, Q, C>(
        &self,
        value: &'a Q,
        length: usize,
        construct: C,
    ) -> (StablePointer<SharedTerm>, bool) 
    where
        Q: Hash + Equivalent<SharedTerm>,
        C: Fn(*mut SharedTerm, &'a Q),
    {
        unsafe { self.terms.insert_equiv_dst(value, length, construct) }
    }
}
