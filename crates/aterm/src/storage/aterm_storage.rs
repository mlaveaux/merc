use std::hash::Hash;
use std::ptr::NonNull;
use std::ptr::slice_from_raw_parts_mut;

use log::debug;
use rustc_hash::FxBuildHasher;

use merc_unsafety::AllocBlock;
use merc_unsafety::BlockAllocatorSafe;
use merc_unsafety::StablePointer;
use merc_unsafety::StablePointerSet;

use crate::ATermIndex;
use crate::ATermRef;
use crate::Symb;
use crate::SymbolRef;
use crate::Term;
use crate::storage::SharedTerm;
use crate::storage::SharedTermLookup;

/// The actual storage for [crate::ATerm]. Terms are stored in separate
/// `StablePointerSet`s based on their arity, and whether they have annotations
/// or not.
pub(crate) struct ATermStorage {
    /// Stores terms of any arity.
    terms: StablePointerSet<SharedTerm, FxBuildHasher>,

    /// Stores the fixed size [SharedTermInt] integer terms.
    int_terms: StablePointerSet<SharedTermInt, FxBuildHasher, AllocBlock<SharedTermInt, BLOCK_SIZE>>,

    /// Stores terms of fixed arity, see [SharedTermFixed].
    terms_0: StablePointerSet<SharedTermFixed<0>, FxBuildHasher, AllocBlock<SharedTermFixed<0>, BLOCK_SIZE>>,
    terms_1: StablePointerSet<SharedTermFixed<1>, FxBuildHasher, AllocBlock<SharedTermFixed<1>, BLOCK_SIZE>>,
    terms_2: StablePointerSet<SharedTermFixed<2>, FxBuildHasher, AllocBlock<SharedTermFixed<2>, BLOCK_SIZE>>,
    terms_3: StablePointerSet<SharedTermFixed<3>, FxBuildHasher, AllocBlock<SharedTermFixed<3>, BLOCK_SIZE>>,
    terms_4: StablePointerSet<SharedTermFixed<4>, FxBuildHasher, AllocBlock<SharedTermFixed<4>, BLOCK_SIZE>>,
    terms_5: StablePointerSet<SharedTermFixed<5>, FxBuildHasher, AllocBlock<SharedTermFixed<5>, BLOCK_SIZE>>,
    terms_6: StablePointerSet<SharedTermFixed<6>, FxBuildHasher, AllocBlock<SharedTermFixed<6>, BLOCK_SIZE>>,
    terms_7: StablePointerSet<SharedTermFixed<7>, FxBuildHasher, AllocBlock<SharedTermFixed<7>, BLOCK_SIZE>>,
}

/// The initial capacity for the term storage.
const INITIAL_CAPACITY: usize = 1024;

/// The number of terms stored in every block of the fixed-size storage.
const BLOCK_SIZE: usize = 1024;

/// The largest arity stored in the fixed-size tables; larger terms go into the
/// dynamically sized `terms` storage.
pub(crate) const MAX_FIXED_ARITY: usize = 7;

impl ATermStorage {
    /// Creates a new, empty storage.
    pub(crate) fn new() -> Self {
        Self {
            terms: StablePointerSet::with_capacity_and_hasher(INITIAL_CAPACITY, FxBuildHasher),
            int_terms: StablePointerSet::with_capacity_and_hasher_in(
                INITIAL_CAPACITY,
                FxBuildHasher,
                AllocBlock::new(),
            ),
            terms_0: StablePointerSet::with_capacity_and_hasher_in(INITIAL_CAPACITY, FxBuildHasher, AllocBlock::new()),
            terms_1: StablePointerSet::with_capacity_and_hasher_in(INITIAL_CAPACITY, FxBuildHasher, AllocBlock::new()),
            terms_2: StablePointerSet::with_capacity_and_hasher_in(INITIAL_CAPACITY, FxBuildHasher, AllocBlock::new()),
            terms_3: StablePointerSet::with_capacity_and_hasher_in(INITIAL_CAPACITY, FxBuildHasher, AllocBlock::new()),
            terms_4: StablePointerSet::with_capacity_and_hasher_in(INITIAL_CAPACITY, FxBuildHasher, AllocBlock::new()),
            terms_5: StablePointerSet::with_capacity_and_hasher_in(INITIAL_CAPACITY, FxBuildHasher, AllocBlock::new()),
            terms_6: StablePointerSet::with_capacity_and_hasher_in(INITIAL_CAPACITY, FxBuildHasher, AllocBlock::new()),
            terms_7: StablePointerSet::with_capacity_and_hasher_in(INITIAL_CAPACITY, FxBuildHasher, AllocBlock::new()),
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
                    symbol: SymbolRef::from_symbol(symbol),
                    args: [],
                });
                unsafe { (cast_to_shared_term_ptr(&result, 0), inserted) }
            }
            1 => {
                let (result, inserted) = self.terms_1.insert(SharedTermFixed {
                    symbol: SymbolRef::from_symbol(symbol),
                    args: [arg()],
                });
                unsafe { (cast_to_shared_term_ptr(&result, 1), inserted) }
            }
            2 => {
                let (result, inserted) = self.terms_2.insert(SharedTermFixed {
                    symbol: SymbolRef::from_symbol(symbol),
                    args: [arg(), arg()],
                });
                unsafe { (cast_to_shared_term_ptr(&result, 2), inserted) }
            }
            3 => {
                let (result, inserted) = self.terms_3.insert(SharedTermFixed {
                    symbol: SymbolRef::from_symbol(symbol),
                    args: [arg(), arg(), arg()],
                });
                unsafe { (cast_to_shared_term_ptr(&result, 3), inserted) }
            }
            4 => {
                let (result, inserted) = self.terms_4.insert(SharedTermFixed {
                    symbol: SymbolRef::from_symbol(symbol),
                    args: [arg(), arg(), arg(), arg()],
                });
                unsafe { (cast_to_shared_term_ptr(&result, 4), inserted) }
            }
            5 => {
                let (result, inserted) = self.terms_5.insert(SharedTermFixed {
                    symbol: SymbolRef::from_symbol(symbol),
                    args: [arg(), arg(), arg(), arg(), arg()],
                });
                unsafe { (cast_to_shared_term_ptr(&result, 5), inserted) }
            }
            6 => {
                let (result, inserted) = self.terms_6.insert(SharedTermFixed {
                    symbol: SymbolRef::from_symbol(symbol),
                    args: [arg(), arg(), arg(), arg(), arg(), arg()],
                });
                unsafe { (cast_to_shared_term_ptr(&result, 6), inserted) }
            }
            7 => {
                let (result, inserted) = self.terms_7.insert(SharedTermFixed {
                    symbol: SymbolRef::from_symbol(symbol),
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
                symbol: SymbolRef::from_index(symbol.shared()),
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

        // Removes empty blocks after removing entries.
        let mut blocks_removed = 0;
        blocks_removed += self.int_terms.allocator_mut().remove_free_blocks();
        blocks_removed += self.terms_0.allocator_mut().remove_free_blocks();
        blocks_removed += self.terms_1.allocator_mut().remove_free_blocks();
        blocks_removed += self.terms_2.allocator_mut().remove_free_blocks();
        blocks_removed += self.terms_3.allocator_mut().remove_free_blocks();
        blocks_removed += self.terms_4.allocator_mut().remove_free_blocks();
        blocks_removed += self.terms_5.allocator_mut().remove_free_blocks();
        blocks_removed += self.terms_6.allocator_mut().remove_free_blocks();
        blocks_removed += self.terms_7.allocator_mut().remove_free_blocks();
        debug!("Removed {} empty blocks from the fixed-size storage", blocks_removed);
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

/// Casts a pointer to a term in a fixed-size storage to a pointer to a
/// [`SharedTerm`].
///
/// SAFETY: The caller must ensure that the given pointer points to a valid term
/// of the given arity.
unsafe fn cast_to_shared_term_ptr<T>(ptr: &StablePointer<T>, arity: usize) -> StablePointer<SharedTerm> {
    // Build a fat pointer for SharedTerm with metadata equal to the term arity.
    let raw = slice_from_raw_parts_mut(ptr.ptr().as_ptr(), arity) as *mut SharedTerm;
    unsafe { StablePointer::from_related_ptr(NonNull::new_unchecked(raw), ptr) }
}

/// Storage for ATerms with a fixed number of arguments.
///
/// Should be the same layout as [SharedTerm] for the shared fields.
#[repr(C)]
#[derive(Hash, Eq, PartialEq)]
pub(crate) struct SharedTermFixed<const N: usize> {
    pub(crate) symbol: SymbolRef<'static>,
    pub(crate) args: [ATermIndex; N],
}

// Safety: The first field is a heap pointer. Heap pointers are always well above the small alignment value returned by
// `std::ptr::dangling_mut()`, so the sentinel can never collide with a live entry.
unsafe impl<const N: usize> BlockAllocatorSafe for SharedTermFixed<N> {}

/// Storage for integer ATerms.
///
/// Should be the same layout as [SharedTerm] for the shared fields.
#[repr(C)]
#[derive(Hash, Eq, PartialEq)]
pub(crate) struct SharedTermInt {
    symbol: SymbolRef<'static>,

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

    #[test]
    fn probe_cast_to_shared_term_ptr_arity_argument_effect() {
        // Characterization probe (not asserting a documented contract): does the `arity`
        // parameter of `cast_to_shared_term_ptr` actually affect the length later observed
        // through the returned `StablePointer<SharedTerm>`? `SharedTerm`'s `Erasable::erase`
        // truncates the fat pointer built from `arity` down to a thin, address-only pointer
        // before `StablePointer` ever stores it (see `Thin::new`); every later `.deref()` calls
        // `SharedTerm::unerase`, which recomputes the slice length from the symbol header
        // instead. So a wrong `arity` here should have no observable effect.
        use crate::ATerm;
        use crate::Symbol;
        use crate::SymbolRef;
        use crate::Term;

        use super::ATermStorage;
        use super::cast_to_shared_term_ptr;

        let storage = ATermStorage::new();
        let arg_symbol = Symbol::new("cast_arity_probe_arg", 0);
        let arg = ATerm::constant(&arg_symbol);
        let head_symbol = Symbol::new("cast_arity_probe_head", 1);

        let (fixed, _inserted) = storage.terms_1.insert(SharedTermFixed {
            symbol: SymbolRef::from_symbol(&head_symbol),
            // SAFETY: `arg` is protected (alive) for the duration of this test.
            args: [unsafe { arg.shared().copy() }],
        });

        // Deliberately pass 0 instead of the correct arity (1).
        // SAFETY: `fixed` points at a live `SharedTermFixed<1>`; only the `arity` argument is
        // "wrong" relative to the real term, which is exactly what this probe investigates.
        let wrong_arity_view = unsafe { cast_to_shared_term_ptr(&fixed, 0) };
        // SAFETY: `fixed` (and therefore `wrong_arity_view`, the same address) is live.
        let observed_len = unsafe { wrong_arity_view.deref() }.arguments().len();

        assert_eq!(
            observed_len, 1,
            "the `arity` parameter passed to `cast_to_shared_term_ptr` does not affect the \
             length later observed via the returned pointer; only the symbol's own arity does"
        );
    }
}

#[cfg(kani)]
mod verification {
    use std::ptr::NonNull;

    use merc_unsafety::StablePointer;

    use crate::Symb;
    use crate::SymbolIndex;
    use crate::SymbolRef;
    use crate::Term;
    use crate::storage::SharedSymbol;

    use super::SharedTermFixed;
    use super::SharedTermInt;
    use super::cast_to_shared_term_ptr;

    /// Builds a leaked, heap-backed `StablePointer<SharedSymbol>` for use as a proof fixture.
    /// Never freed: a `kani::proof` is a single bounded, non-reentrant execution, not a
    /// long-running process, so there is no leak to reclaim.
    fn leak_symbol(arity: usize) -> SymbolIndex {
        let boxed = Box::new(SharedSymbol::new("s", arity));
        let ptr = NonNull::from(Box::leak(boxed));
        // SAFETY: `ptr` points at a leaked allocation that is live for the rest of the proof
        // and is never mutated or freed elsewhere.
        unsafe { StablePointer::from_ptr(ptr) }
    }

    /// Builds a leaked, zero-argument `SharedTerm` pointer exactly the way
    /// `ATermStorage::insert_int_term` / `insert_fixed_iter(arity = 0)` do: by casting a
    /// `SharedTermFixed<0>` through [`cast_to_shared_term_ptr`].
    fn leak_zero_arity_term() -> StablePointer<super::SharedTerm> {
        let symbol_index = leak_symbol(0);
        // SAFETY: `symbol_index` is a leaked, live `SharedSymbol`.
        let symbol = unsafe { SymbolRef::from_index(&symbol_index) };
        let fixed = Box::new(SharedTermFixed::<0> { symbol, args: [] });
        let fixed_ptr = NonNull::from(Box::leak(fixed));
        // SAFETY: `fixed_ptr` points at a leaked, live `SharedTermFixed<0>`.
        let fixed_stable = unsafe { StablePointer::from_ptr(fixed_ptr) };
        // SAFETY: `fixed_stable` points at a `SharedTermFixed<0>`, whose layout matches
        // `SharedTerm` for a zero-length arguments slice (see the `offset_of!` static
        // assertions in `mod tests` above).
        unsafe { cast_to_shared_term_ptr(&fixed_stable, 0) }
    }

    /// `cast_to_shared_term_ptr` must reconstruct a `SharedTerm` fat pointer whose *address*
    /// is exactly the source pointer's address, and whose `arguments()` slice is empty for a
    /// zero-arity source. This is the load-bearing property that lets the fixed-arity storage
    /// tables (and `ATermInt::value_unchecked`, via the sibling proof below) alias a
    /// `SharedTermFixed<0>` / `SharedTermInt` allocation as a `SharedTerm`.
    #[kani::proof]
    fn cast_to_shared_term_ptr_arity_zero_preserves_address_and_length() {
        let symbol_index = leak_symbol(0);
        // SAFETY: `symbol_index` is a leaked, live `SharedSymbol`.
        let symbol = unsafe { SymbolRef::from_index(&symbol_index) };
        let fixed = Box::new(SharedTermFixed::<0> { symbol, args: [] });
        let fixed_ptr = NonNull::from(Box::leak(fixed));
        // SAFETY: `fixed_ptr` points at a leaked, live `SharedTermFixed<0>`.
        let fixed_stable = unsafe { StablePointer::from_ptr(fixed_ptr) };

        // SAFETY: `fixed_stable` points at a valid `SharedTermFixed<0>`.
        let shared_stable = unsafe { cast_to_shared_term_ptr(&fixed_stable, 0) };

        // The reconstructed pointer's address is exactly the source's address.
        assert!(std::ptr::eq(
            shared_stable.ptr().as_ptr() as *const u8,
            fixed_stable.ptr().as_ptr() as *const u8,
        ));

        // SAFETY: the pointee is the live `SharedTermFixed<0>` allocated above, whose layout
        // matches `SharedTerm` for a zero-length arguments slice.
        let shared_term = unsafe { shared_stable.deref() };
        assert_eq!(shared_term.arguments().len(), 0);
        assert_eq!(shared_term.symbol().shared().ptr(), symbol_index.ptr());
    }

    /// The same reconstruction with a populated argument slot: `cast_to_shared_term_ptr` must
    /// expose the `SharedTermFixed<1>::args[0]` entry as `SharedTerm::arguments()[0]`, at the
    /// identical address.
    #[kani::proof]
    fn cast_to_shared_term_ptr_with_one_argument_preserves_argument_identity() {
        let leaf = leak_zero_arity_term();

        let symbol_index = leak_symbol(1);
        // SAFETY: `symbol_index` is a leaked, live `SharedSymbol`.
        let symbol = unsafe { SymbolRef::from_index(&symbol_index) };
        // SAFETY: `leaf` points at a leaked, live zero-arity `SharedTerm`; copying the
        // `StablePointer` handle does not dereference it.
        let arg = unsafe { leaf.copy() };
        let fixed = Box::new(SharedTermFixed::<1> { symbol, args: [arg] });
        let fixed_ptr = NonNull::from(Box::leak(fixed));
        // SAFETY: `fixed_ptr` points at a leaked, live `SharedTermFixed<1>`.
        let fixed_stable = unsafe { StablePointer::from_ptr(fixed_ptr) };

        // SAFETY: `fixed_stable` points at a valid `SharedTermFixed<1>`.
        let shared_stable = unsafe { cast_to_shared_term_ptr(&fixed_stable, 1) };
        // SAFETY: see above; the pointee is live for the duration of the proof.
        let shared_term = unsafe { shared_stable.deref() };

        assert_eq!(shared_term.arguments().len(), 1);
        assert_eq!(shared_term.arguments()[0].shared().ptr(), leaf.ptr());
    }

    /// Mirrors the exact unsafe expression behind `ATermInt::value_unchecked`
    /// (`self.shared().ptr_with_len(0).cast::<SharedTermInt>().as_ref().value()`): given a
    /// `StablePointer<SharedTerm>` that is actually backed by a `SharedTermInt` allocation,
    /// reading it back through that expression must observe exactly the value written into
    /// `annotation`, for *every* representable `usize` (kani explores the full symbolic range,
    /// covering the boundary values 0 and `usize::MAX` along with everything in between).
    #[kani::proof]
    fn value_unchecked_pattern_reads_back_stored_annotation() {
        let value: usize = kani::any();

        let symbol_index = leak_symbol(0);
        // SAFETY: `symbol_index` is a leaked, live `SharedSymbol`.
        let symbol = unsafe { SymbolRef::from_index(&symbol_index) };
        let int_term = Box::new(SharedTermInt { symbol, annotation: value });
        let int_ptr = NonNull::from(Box::leak(int_term));
        // SAFETY: `int_ptr` points at a leaked, live `SharedTermInt`.
        let int_stable = unsafe { StablePointer::from_ptr(int_ptr) };

        // SAFETY: `int_stable` points at a `SharedTermInt`, whose layout matches `SharedTerm`
        // for a zero-length arguments slice (see the `offset_of!` static assertions in `mod
        // tests` above): `symbol` is at offset 0 in both, and `SharedTermInt::annotation`
        // starts at the same offset as `SharedTermFixed::args`.
        let shared_stable = unsafe { cast_to_shared_term_ptr(&int_stable, 0) };

        // SAFETY: `ptr_with_len(0)` reconstructs the address without reading the pointee (it
        // trusts the caller-supplied length instead of the symbol header); `int_stable` points
        // at a live `SharedTermInt`, so casting back to that type and reading it is valid.
        let observed = unsafe { shared_stable.ptr_with_len(0).cast::<SharedTermInt>().as_ref().value() };

        assert_eq!(observed, value);
    }
}
