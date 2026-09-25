use std::hash::Hash;
use std::hash::Hasher;

use equivalent::Equivalent;
use log::debug;
use rustc_hash::FxBuildHasher;

use merc_unsafety::AllocBlock;
use merc_unsafety::BlockAllocatorSafe;
use merc_unsafety::StablePointer;
use merc_unsafety::StablePointerSet;

use crate::Symb;
use crate::SymbolIndex;
use crate::SymbolRef;

/// Pool for maximal sharing of function symbols, see [SymbolRef]. Ensures that function symbols
/// with the same name and arity point to the same [SharedSymbol] object.
/// Returns [crate::Symbol] that can be used to refer to the shared symbol, avoiding
/// garbage collection of the underlying shared symbol.
pub(crate) struct SymbolPool {
    /// Unique table of all function symbols
    symbols: StablePointerSet<SharedSymbol, FxBuildHasher, AllocBlock<SharedSymbol, 1024>>,

    /// The pool's own reserved marker symbols, which are created by
    /// create_reserved and cannot be created by the public Symbol::new.
    reserved: Vec<SymbolIndex>,
}

impl SymbolPool {
    /// Creates a new empty symbol pool.
    pub(crate) fn new() -> Self {
        Self {
            symbols: StablePointerSet::with_hasher_in(FxBuildHasher, AllocBlock::new()),
            reserved: Vec::new(),
        }
    }

    /// Creates or retrieves a function symbol with the given name and arity.
    ///
    /// # Panics
    ///
    /// Panics if the name and arity collide with one of the pool's own reserved
    /// marker symbols (see [`create_reserved`](Self::create_reserved)).
    ///
    /// The returned pointer is unprotected, so the caller must protect it
    /// before the lock it was created under is released.
    pub(crate) fn create<N>(&self, name: N, arity: usize) -> StablePointer<SharedSymbol>
    where
        N: Into<String> + AsRef<str>,
    {
        self.create_impl::<false, N>(name, arity)
    }

    /// Creates one of the pool's own reserved marker symbols (e.g.
    /// `<aterm_int>`).
    ///
    /// The returned pointer is unprotected, so the caller must protect it
    /// before the lock it was created under is released.
    pub(crate) fn create_reserved<N>(&mut self, name: N, arity: usize) -> StablePointer<SharedSymbol>
    where
        N: Into<String> + AsRef<str>,
    {
        let result = self.create_impl::<true, N>(name, arity);
        // SAFETY: `result` points into `self.symbols`, whose entries are never removed
        // for reserved symbols.
        self.reserved.push(unsafe { result.copy() });
        result
    }

    /// `RESERVED` is a const generic rather than a runtime field so that the
    /// collision check below (and the field it would otherwise need on every
    /// [`SharedSymbol`]) is compiled away entirely for `create_reserved`'s call,
    /// and costs only a handful of pointer comparisons for `create`'s call.
    fn create_impl<const RESERVED: bool, N>(&self, name: N, arity: usize) -> StablePointer<SharedSymbol>
    where
        N: Into<String> + AsRef<str>,
    {
        // Get or create symbol index. A colliding name/arity resolves to the
        // existing reserved entry rather than inserting a new one, since both
        // paths key on the same `(name, arity)`.
        let (shared_symbol, _inserted) = self.symbols.insert_equiv(&SharedSymbolLookup { name, arity });

        if !RESERVED && self.reserved.contains(&shared_symbol) {
            // SAFETY: `shared_symbol` was just returned by `insert_equiv` above, so it
            // is resident in `self.symbols`.
            let symbol = unsafe { shared_symbol.deref() };
            panic!(
                "cannot create a symbol named \"{}\" with arity {}: \
                 the name is reserved for internal use by the term pool",
                symbol.name(),
                symbol.arity()
            );
        }

        shared_symbol
    }

    /// Return the symbol of the SharedTerm for the given ATermRef
    #[allow(dead_code)]
    pub(crate) fn symbol_name<'a>(&self, symbol: &'a SymbolRef<'a>) -> &'a str {
        // SAFETY: `symbol` is a `SymbolRef<'a>`, so its symbol is alive for `'a`.
        unsafe { symbol.shared().deref() }.name()
    }

    /// Returns the arity of the function symbol
    #[allow(dead_code)]
    pub(crate) fn symbol_arity<'a, 'b, S: Symb<'a, 'b>>(&self, symbol: &'b S) -> usize {
        // SAFETY: `symbol` borrows a live symbol for the duration of the call.
        unsafe { symbol.shared().deref() }.arity()
    }

    /// Returns the number of symbols in the pool.
    #[allow(dead_code)]
    pub(crate) fn len(&self) -> usize {
        self.symbols.len()
    }

    /// Returns true if the pool is empty.
    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    /// Returns the capacity of the pool.
    #[allow(dead_code)]
    pub(crate) fn capacity(&self) -> usize {
        self.symbols.capacity()
    }

    /// Retain only symbols satisfying the given predicate.
    ///
    /// # Safety
    ///
    /// Removal invalidates every [`SymbolIndex`] to a removed symbol; the caller must guarantee
    /// that no index to a removed symbol is dereferenced afterwards.
    pub(crate) unsafe fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&SymbolIndex) -> bool,
    {
        // SAFETY: The caller guarantees that indices of removed symbols are not used again.
        unsafe {
            self.symbols.retain(|element| f(element));
        }

        let removed_blocks = self.symbols.allocator_mut().remove_free_blocks();
        debug!("Removed {} blocks from the symbol pool", removed_blocks);
    }
}

/// Represents a function symbol with a name and arity.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SharedSymbol {
    /// Name of the function
    name: String,
    /// Number of arguments
    arity: usize,
}

// SAFETY: `BlockAllocatorSafe`'s two required properties (see its own `# Safety` doc in
// `merc_unsafety`), checked against `SharedSymbol { name: String, arity: usize }`:
//
// 1. The sentinel (`usize::MAX`, i.e. all bytes `0xFF`) must never occur as the first
//    `size_of::<*mut _>()` bytes of a live `SharedSymbol`. With the layout this type currently
//    gets from rustc (verified below), those bytes are `name`'s internal heap pointer (or, for
//    an unallocated `String`, `NonNull::dangling()`, `align_of::<u8>() == 1`): no allocator in
//    this codebase or on the platforms it targets ever hands out the address `usize::MAX` (not a
//    valid, mappable heap address on any supported target), which is exactly why that bit
//    pattern was chosen as the sentinel. That allocator-never-returns-this-address part is an
//    axiom about the platform, not something a test or a Kani proof can establish -- no more
//    than either could prove the standard library's own niche-value optimisations.
// 2. Those same bytes must always be fully initialized: `RawVec::ptr` carries no padding and is
//    never uninitialized for a constructed `String` (empty or not), so this half holds
//    regardless of which field ends up first.
//
// Caveat this impl does NOT fully discharge: `SharedSymbol` is not `#[repr(C)]` (unlike
// `SharedTermFixed`/`SharedTermInt` in `aterm_storage.rs`, which are, with an analogous
// first-field comment), so property 1 additionally assumes `name` occupies the struct's first
// `size_of::<*mut _>()` bytes -- true for the layout this rustc currently produces (checked with
// `std::mem::offset_of!` below), but not a guarantee `#[repr(Rust)]` makes, and not pinned by a
// static assertion. If a future compiler (or a different codegen configuration) placed `arity:
// usize` first instead, this impl would depend on `arity` itself never reaching exactly
// `usize::MAX` -- a value `Symbol::new`/`SharedSymbol::new` accept without any bound.
// See the review report for this crate for the accompanying finding and suggested fix
// (`#[repr(C)]` on `SharedSymbol`, pinned by an `offset_of!` static assertion as the other two
// `BlockAllocatorSafe` types already have).
unsafe impl BlockAllocatorSafe for SharedSymbol {}

impl SharedSymbol {
    /// Creates a new function symbol.
    pub fn new<N: Into<String>>(name: N, arity: usize) -> Self {
        Self {
            name: name.into(),
            arity,
        }
    }

    /// Returns the name of the function symbol
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the arity of the function symbol
    pub fn arity(&self) -> usize {
        self.arity
    }

    /// Returns a unique index for this shared symbol
    pub fn index(&self) -> usize {
        self as *const Self as *const u8 as usize
    }
}

/// A cheap way to look up SharedSymbol
struct SharedSymbolLookup<T: Into<String> + AsRef<str>> {
    name: T,
    arity: usize,
}

impl<T: Into<String> + AsRef<str>> From<&SharedSymbolLookup<T>> for SharedSymbol {
    fn from(lookup: &SharedSymbolLookup<T>) -> Self {
        // TODO: Not optimal
        let string = lookup.name.as_ref().to_string();
        Self::new(string, lookup.arity)
    }
}

impl<T: Into<String> + AsRef<str>> Equivalent<SharedSymbol> for SharedSymbolLookup<T> {
    fn equivalent(&self, other: &SharedSymbol) -> bool {
        self.name.as_ref() == other.name && self.arity == other.arity
    }
}

/// These hash implementations should be the same as `SharedSymbol`.
impl<T: Into<String> + AsRef<str>> Hash for SharedSymbolLookup<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.as_ref().hash(state);
        self.arity.hash(state);
    }
}

impl Hash for SharedSymbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.arity.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use std::mem::offset_of;

    use crate::Symbol;

    use super::SharedSymbol;

    #[test]
    fn test_symbol_sharing() {
        merc_utilities::test_logger();

        let f1 = Symbol::new("f", 2);
        let f2 = Symbol::new("f", 2);

        // Should be the same object
        assert_eq!(f1, f2);
    }

    // Pins the field layout the `BlockAllocatorSafe` impl above depends on: `SharedSymbol` is
    // not `#[repr(C)]`, so nothing else guarantees `name` (whose internal heap pointer must
    // avoid the allocator's `usize::MAX` sentinel) occupies the struct's first
    // `size_of::<*mut _>()` bytes rather than `arity` (an unbounded `usize` that could
    // legitimately equal the sentinel). This assertion turns a silent layout change into a
    // compile failure instead of a latent soundness gap; see the accompanying review report for
    // the recommended fix (`#[repr(C)]` on `SharedSymbol`).
    const _: () = assert!(offset_of!(SharedSymbol, name) == 0);
}
