use std::hash::Hash;
use std::ptr::NonNull;
use std::ptr::slice_from_raw_parts_mut;

use log::debug;
use rustc_hash::FxBuildHasher;

use merc_unsafety::BlockAllocatorSafe;
use merc_unsafety::ChainedPointerSet;
use merc_unsafety::StablePointer;
use merc_unsafety::StablePointerSet;

use crate::ATermIndex;
use crate::ATermRef;
use crate::Symb;
use crate::SymbolRef;
use crate::Term;
use crate::storage::SharedTerm;
use crate::storage::SharedTermLookup;
use crate::storage::TermSymbol;

/// The actual storage for [crate::ATerm]. Terms are stored in separate
/// `StablePointerSet`s based on their arity, and whether they have annotations
/// or not.
pub(crate) struct ATermStorage {
    /// Stores terms of any arity.
    terms: StablePointerSet<SharedTerm, FxBuildHasher>,

    /// Stores the fixed size [SharedTermInt] integer terms.
    int_terms: FixedSet<SharedTermInt>,

    /// Stores terms of fixed arity, see [SharedTermFixed].
    terms_0: FixedSet<SharedTermFixed<0>>,
    terms_1: FixedSet<SharedTermFixed<1>>,
    terms_2: FixedSet<SharedTermFixed<2>>,
    terms_3: FixedSet<SharedTermFixed<3>>,
    terms_4: FixedSet<SharedTermFixed<4>>,
    terms_5: FixedSet<SharedTermFixed<5>>,
    terms_6: FixedSet<SharedTermFixed<6>>,
    terms_7: FixedSet<SharedTermFixed<7>>,
}

/// The initial capacity for the term storage.
const INITIAL_CAPACITY: usize = 1024;

/// The largest arity stored in the fixed-size tables; larger terms go into the
/// dynamically sized `terms` storage.
pub(crate) const MAX_FIXED_ARITY: usize = 7;

impl ATermStorage {
    /// Creates a new, empty storage.
    pub(crate) fn new() -> Self {
        Self {
            terms: StablePointerSet::with_capacity_and_hasher(INITIAL_CAPACITY, FxBuildHasher),
            int_terms: new_fixed_set(),
            terms_0: new_fixed_set(),
            terms_1: new_fixed_set(),
            terms_2: new_fixed_set(),
            terms_3: new_fixed_set(),
            terms_4: new_fixed_set(),
            terms_5: new_fixed_set(),
            terms_6: new_fixed_set(),
            terms_7: new_fixed_set(),
        }
    }

    /// Inserts a term with the given symbol and arguments into the storage,
    /// returning a pointer to the stored term and whether it was newly
    /// inserted.
    pub(crate) fn insert<'a, 'b, 'c, S: Symb<'a, 'b>>(
        &'c self,
        symbol: &'b S,
        args: &'c [ATermRef<'c>],
    ) -> (StablePointer<SharedTerm>, bool) {
        debug_assert_eq!(
            symbol.arity(),
            args.len(),
            "The number of arguments does not match the arity of the symbol"
        );

        if symbol.arity() <= MAX_FIXED_ARITY {
            return self.insert_fixed(symbol, args);
        }

        let shared_term = SharedTermLookup {
            symbol: SymbolRef::from_symbol(symbol),
            arguments: args,
        };

        unsafe {
            self.terms
                .insert_equiv_dst(&shared_term, SharedTerm::length_for(&shared_term), |ptr, key| {
                    SharedTerm::construct(ptr, key)
                })
        }
    }

    /// Inserts a term whose arity is at most [MAX_FIXED_ARITY] into the corresponding fixed-size
    /// storage, building the `SharedTermFixed<N>` key straight from the argument slice.
    ///
    /// Taking the arguments as a generic `T: Term` slice lets callers pass the terms they already
    /// have (e.g. the input slice of [crate::ATerm::with_args]) without first materialising an
    /// intermediate `ATermRef` buffer.
    pub(crate) fn insert_fixed<'a, 'b, 'c, 'd, S, T>(
        &self,
        symbol: &'b S,
        args: &[T],
    ) -> (StablePointer<SharedTerm>, bool)
    where
        S: Symb<'a, 'b>,
        T: Term<'c, 'd>,
    {
        debug_assert_eq!(
            symbol.arity(),
            args.len(),
            "The number of arguments does not match the arity of the symbol"
        );

        // SAFETY: the copied argument indices are stored inside the inserted term, and the
        // GC marks the arguments of every live term, so each argument stays in the pool at
        // least as long as the term referencing it; the copies are dropped when the term
        // itself is reclaimed.
        self.insert_fixed_iter(symbol, args.iter().map(|arg| unsafe { arg.shared().copy() }))
    }

    /// Inserts a term whose arity is at most [MAX_FIXED_ARITY], pulling the argument indices
    /// straight from the iterator. Counterpart of [Self::insert_fixed] for callers that only
    /// have an iterator and would otherwise round-trip through an intermediate buffer.
    ///
    /// The caller produces the `ATermIndex` copies (an unsafe operation) and thereby
    /// guarantees they stay valid until the inserted term takes ownership of them.
    ///
    /// # Panics
    ///
    /// Panics when the iterator yields fewer items than the arity of the symbol.
    pub(crate) fn insert_fixed_iter<'a, 'b, S, I>(
        &self,
        symbol: &'b S,
        mut args: I,
    ) -> (StablePointer<SharedTerm>, bool)
    where
        S: Symb<'a, 'b>,
        I: Iterator<Item = ATermIndex>,
    {
        let mut arg = || {
            args.next()
                .expect("The iterator yields fewer arguments than the arity of the symbol")
        };

        let result = match symbol.arity() {
            0 => {
                let (result, inserted) = self.terms_0.insert(SharedTermFixed {
                    symbol: TermSymbol::new(&SymbolRef::from_symbol(symbol)),
                    args: [],
                });
                unsafe { (cast_to_shared_term_ptr(&result, 0), inserted) }
            }
            1 => {
                let (result, inserted) = self.terms_1.insert(SharedTermFixed {
                    symbol: TermSymbol::new(&SymbolRef::from_symbol(symbol)),
                    args: [arg()],
                });
                unsafe { (cast_to_shared_term_ptr(&result, 1), inserted) }
            }
            2 => {
                let (result, inserted) = self.terms_2.insert(SharedTermFixed {
                    symbol: TermSymbol::new(&SymbolRef::from_symbol(symbol)),
                    args: [arg(), arg()],
                });
                unsafe { (cast_to_shared_term_ptr(&result, 2), inserted) }
            }
            3 => {
                let (result, inserted) = self.terms_3.insert(SharedTermFixed {
                    symbol: TermSymbol::new(&SymbolRef::from_symbol(symbol)),
                    args: [arg(), arg(), arg()],
                });
                unsafe { (cast_to_shared_term_ptr(&result, 3), inserted) }
            }
            4 => {
                let (result, inserted) = self.terms_4.insert(SharedTermFixed {
                    symbol: TermSymbol::new(&SymbolRef::from_symbol(symbol)),
                    args: [arg(), arg(), arg(), arg()],
                });
                unsafe { (cast_to_shared_term_ptr(&result, 4), inserted) }
            }
            5 => {
                let (result, inserted) = self.terms_5.insert(SharedTermFixed {
                    symbol: TermSymbol::new(&SymbolRef::from_symbol(symbol)),
                    args: [arg(), arg(), arg(), arg(), arg()],
                });
                unsafe { (cast_to_shared_term_ptr(&result, 5), inserted) }
            }
            6 => {
                let (result, inserted) = self.terms_6.insert(SharedTermFixed {
                    symbol: TermSymbol::new(&SymbolRef::from_symbol(symbol)),
                    args: [arg(), arg(), arg(), arg(), arg(), arg()],
                });
                unsafe { (cast_to_shared_term_ptr(&result, 6), inserted) }
            }
            7 => {
                let (result, inserted) = self.terms_7.insert(SharedTermFixed {
                    symbol: TermSymbol::new(&SymbolRef::from_symbol(symbol)),
                    args: [arg(), arg(), arg(), arg(), arg(), arg(), arg()],
                });
                unsafe { (cast_to_shared_term_ptr(&result, 7), inserted) }
            }
            arity => unreachable!("insert_fixed_iter called with arity {arity} > {MAX_FIXED_ARITY}"),
        };

        debug_assert!(
            args.next().is_none(),
            "The iterator yields more arguments than the arity of the symbol"
        );
        result
    }

    /// Inserts an integer term into the storage, returning a pointer to the stored term
    /// and whether it was newly inserted.
    pub(crate) unsafe fn insert_int_term(
        &self,
        symbol: SymbolRef<'_>,
        value: usize,
    ) -> (StablePointer<SharedTerm>, bool) {
        unsafe {
            let (result, inserted) = self.int_terms.insert(SharedTermInt {
                symbol: TermSymbol::new(&symbol),
                annotation: value,
            });

            (cast_to_shared_term_ptr(&result, 0), inserted)
        }
    }

    /// Retains only the terms for which the given predicate returns `true`.
    ///
    /// # Safety
    ///
    /// Removal invalidates every [`StablePointer`] to a removed term; the caller must guarantee
    /// that no pointer to a removed term is dereferenced afterwards.
    pub(crate) unsafe fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&StablePointer<SharedTerm>) -> bool,
    {
        // The fixed-arity tables free their removed terms and empty blocks themselves.
        // SAFETY: The caller guarantees that pointers to removed terms are not used again.
        unsafe {
            self.terms.retain(|term| f(term));

            self.int_terms.retain(|term| f(&cast_to_shared_term_ptr(term, 0)));
            self.terms_0.retain(|term| f(&cast_to_shared_term_ptr(term, 0)));
            self.terms_1.retain(|term| f(&cast_to_shared_term_ptr(term, 1)));
            self.terms_2.retain(|term| f(&cast_to_shared_term_ptr(term, 2)));
            self.terms_3.retain(|term| f(&cast_to_shared_term_ptr(term, 3)));
            self.terms_4.retain(|term| f(&cast_to_shared_term_ptr(term, 4)));
            self.terms_5.retain(|term| f(&cast_to_shared_term_ptr(term, 5)));
            self.terms_6.retain(|term| f(&cast_to_shared_term_ptr(term, 6)));
            self.terms_7.retain(|term| f(&cast_to_shared_term_ptr(term, 7)));
        }

    }

    /// Returns true if one of the fixed-arity tables should grow its hash index, see
    /// [Self::grow].
    pub(crate) fn needs_grow(&self) -> bool {
        self.int_terms.needs_grow()
            || self.terms_0.needs_grow()
            || self.terms_1.needs_grow()
            || self.terms_2.needs_grow()
            || self.terms_3.needs_grow()
            || self.terms_4.needs_grow()
            || self.terms_5.needs_grow()
            || self.terms_6.needs_grow()
            || self.terms_7.needs_grow()
    }

    /// Grows the hash index of the fixed-arity tables that need it; the terms do not move.
    pub(crate) fn grow(&mut self) {
        self.int_terms.grow();
        self.terms_0.grow();
        self.terms_1.grow();
        self.terms_2.grow();
        self.terms_3.grow();
        self.terms_4.grow();
        self.terms_5.grow();
        self.terms_6.grow();
        self.terms_7.grow();
    }

    /// Returns the number of stored terms.
    pub(crate) fn len(&self) -> usize {
        self.int_terms.len()
            + self.terms_0.len()
            + self.terms_1.len()
            + self.terms_2.len()
            + self.terms_3.len()
            + self.terms_4.len()
            + self.terms_5.len()
            + self.terms_6.len()
            + self.terms_7.len()
            + self.terms.len()
    }

    /// Returns the total number of terms that fit before any table must grow.
    pub(crate) fn capacity(&self) -> usize {
        self.int_terms.capacity()
            + self.terms_0.capacity()
            + self.terms_1.capacity()
            + self.terms_2.capacity()
            + self.terms_3.capacity()
            + self.terms_4.capacity()
            + self.terms_5.capacity()
            + self.terms_6.capacity()
            + self.terms_7.capacity()
            + self.terms.capacity()
    }
}

/// The set that stores the terms of one fixed arity: a lock-free hash set with CAS chaining,
/// see `experiments/aterm-lockfree-table.md` for the comparison with the DashSet-based
/// `StablePointerSet` and with open addressing.
type FixedSet<T> = ChainedPointerSet<T, FxBuildHasher>;

/// Initial number of buckets of every fixed-arity table.
const INITIAL_BUCKETS: usize = 1 << 14;

fn new_fixed_set<T: Hash + Eq + Send + Sync + BlockAllocatorSafe + merc_unsafety::Erasable>() -> FixedSet<T> {
    FixedSet::with_capacity_and_hasher(INITIAL_BUCKETS, FxBuildHasher)
}

/// Casts a pointer to a term in a fixed-size storage to a pointer to a
/// [`SharedTerm`].
///
/// SAFETY: The caller must ensure that the given pointer points to a valid term
/// of the given arity.
unsafe fn cast_to_shared_term_ptr<T>(ptr: &StablePointer<T>, arity: usize) -> StablePointer<SharedTerm> {
    // Build a fat pointer for SharedTerm with metadata equal to the term arity.
    // SAFETY: this function's own contract (`ptr` points at a valid term of the given arity)
    // is exactly `ptr()`'s precondition too.
    let raw = unsafe { slice_from_raw_parts_mut(ptr.ptr().as_ptr(), arity) as *mut SharedTerm };
    unsafe { StablePointer::from_related_ptr(NonNull::new_unchecked(raw), ptr) }
}

/// Storage for ATerms with a fixed number of arguments.
///
/// Should be the same layout as [SharedTerm] for the shared fields.
#[repr(C)]
#[derive(Hash, Eq, PartialEq)]
pub(crate) struct SharedTermFixed<const N: usize> {
    pub(crate) symbol: TermSymbol,
    pub(crate) args: [ATermIndex; N],
}

// Safety: The first field is a heap pointer, possibly with its lowest bit set as the garbage collection mark. Heap
// pointers are always well above the small alignment value returned by `std::ptr::dangling_mut()`, so the sentinel can
// never collide with a live entry.
unsafe impl<const N: usize> BlockAllocatorSafe for SharedTermFixed<N> {}

/// Storage for integer ATerms.
///
/// Should be the same layout as [SharedTerm] for the shared fields.
#[repr(C)]
#[derive(Hash, Eq, PartialEq)]
pub(crate) struct SharedTermInt {
    symbol: TermSymbol,

    /// The only important aspect is that `symbol` remains in the same position,
    /// and has arity 0.
    annotation: usize,
}

// Safety: Same reasoning as `SharedTermFixed` — the first field is a heap pointer.
unsafe impl BlockAllocatorSafe for SharedTermInt {}

impl SharedTermInt {
    /// Returns the value of the integer term.
    pub(crate) fn value(&self) -> usize {
        self.annotation
    }
}

#[cfg(test)]
mod tests {
    use std::mem::align_of;
    use std::mem::offset_of;
    use std::mem::size_of;

    use crate::ATermIndex;
    use crate::ATermRef;

    use super::SharedTermFixed;
    use super::SharedTermInt;

    // `symbol` must be at offset 0 in all term representations so that any pointer to a term
    // can safely be cast to `*const SymbolRef` to read the header.
    const _: () = assert!(offset_of!(SharedTermFixed<1>, symbol) == 0);
    const _: () = assert!(offset_of!(SharedTermInt, symbol) == 0);

    // The args (SharedTermFixed) and annotation (SharedTermInt) fields must start at the same
    // byte offset.
    const _: () = assert!(offset_of!(SharedTermFixed<1>, args) == offset_of!(SharedTermInt, annotation));

    // Both element types must have identical size and alignment so that indexing into the
    // argument array produces the same byte offsets in both representations.
    const _: () = assert!(size_of::<ATermIndex>() == size_of::<ATermRef<'static>>());
    const _: () = assert!(align_of::<ATermIndex>() == align_of::<ATermRef<'static>>());

    // With a thin `ATermIndex` (one word), a binary term is a symbol word plus two argument
    // words: 24 bytes, down from 40 when the arguments were wide slice pointers. Only holds in
    // release builds, where `StablePointer` drops its debug reference counter.
    #[cfg(not(debug_assertions))]
    const _: () = assert!(size_of::<SharedTermFixed<2>>() == 3 * size_of::<usize>());
}
