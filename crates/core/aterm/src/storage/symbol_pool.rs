use std::alloc::Layout;
use std::alloc::LayoutError;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::ptr::NonNull;
use std::ptr::slice_from_raw_parts_mut;

use equivalent::Equivalent;
use rustc_hash::FxBuildHasher;

use merc_unsafety::Erasable;
use merc_unsafety::ErasedPtr;
use merc_unsafety::SliceDst;
use merc_unsafety::StablePointer;
use merc_unsafety::StablePointerSet;
use merc_unsafety::repr_c;
use merc_utilities::AtomicTaggedUsize;

use crate::Symb;
use crate::SymbolIndex;
use crate::SymbolRef;

/// Pool for maximal sharing of function symbols, see [SymbolRef]. Ensures that function symbols
/// with the same name and arity point to the same [SharedSymbol] object.
/// Returns [crate::Symbol] that can be used to refer to the shared symbol, avoiding
/// garbage collection of the underlying shared symbol.
pub(crate) struct SymbolPool {
    /// Unique table of all function symbols
    symbols: StablePointerSet<SharedSymbol, FxBuildHasher>,

    /// The pool's own reserved marker symbols, which are created by
    /// create_reserved and cannot be created by the public Symbol::new.
    reserved: Vec<SymbolIndex>,
}

impl SymbolPool {
    /// Creates a new empty symbol pool.
    pub(crate) fn new() -> Self {
        Self {
            symbols: StablePointerSet::with_hasher(FxBuildHasher),
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
    pub(crate) fn create(&self, name: &str, arity: usize) -> StablePointer<SharedSymbol> {
        self.create_impl::<false>(name, arity)
    }

    /// Creates a function symbol with the given name and arity, or returns `None` when the pool
    /// already contains one. Checking and inserting is a single step, so of several threads
    /// that ask for the same new symbol exactly one receives it.
    ///
    /// The returned pointer is unprotected, so the caller must protect it
    /// before the lock it was created under is released.
    pub(crate) fn create_unused(&self, name: &str, arity: usize) -> Option<StablePointer<SharedSymbol>> {
        let (shared_symbol, inserted) = self.insert(name, arity);
        inserted.then_some(shared_symbol)
    }

    /// Creates one of the pool's own reserved marker symbols (e.g.
    /// `<aterm_int>`).
    ///
    /// The returned pointer is unprotected, so the caller must protect it
    /// before the lock it was created under is released.
    pub(crate) fn create_reserved(&mut self, name: &str, arity: usize) -> StablePointer<SharedSymbol> {
        let result = self.create_impl::<true>(name, arity);
        // SAFETY: `result` points into `self.symbols`, whose entries are never removed
        // for reserved symbols.
        self.reserved.push(unsafe { result.copy() });
        result
    }

    /// `RESERVED` is a const generic rather than a runtime field so that the
    /// collision check below (and the field it would otherwise need on every
    /// [`SharedSymbol`]) is compiled away entirely for `create_reserved`'s call,
    /// and costs only a handful of pointer comparisons for `create`'s call.
    fn create_impl<const RESERVED: bool>(&self, name: &str, arity: usize) -> StablePointer<SharedSymbol> {
        // A colliding name/arity resolves to the existing reserved entry rather
        // than inserting a new one, since both paths key on the same `(name, arity)`.
        let (shared_symbol, _inserted) = self.insert(name, arity);

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

    /// Gets or creates the symbol with the given name and arity, and returns whether it was
    /// created. The name is only copied into the symbol's own allocation when it is inserted.
    fn insert(&self, name: &str, arity: usize) -> (StablePointer<SharedSymbol>, bool) {
        // SAFETY: `construct` fully initialises the symbol allocated for `name.len()` bytes.
        unsafe {
            self.symbols
                .insert_equiv_dst(&SharedSymbolLookup { name, arity }, name.len(), |ptr, key| {
                    SharedSymbol::construct(ptr, key.name, key.arity)
                })
        }
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
    }
}

/// The fixed-size header of a [SharedSymbol], which precedes its inline name.
///
/// Terms point to this header directly (see `TermSymbol`), so that reading the
/// arity on the term hot path does not need to reconstruct the wide pointer.
#[repr(C)]
pub(crate) struct SymbolHeader {
    /// Number of arguments, tagged with the garbage collection mark so that marking needs no
    /// separate set. As for terms, the mark is only changed while the pool is held exclusively,
    /// but other threads may read the arity meanwhile.
    arity: AtomicTaggedUsize,
    /// Length of the name in bytes
    len: usize,
}

/// Represents a function symbol with a name and arity.
///
/// Uses a C representation and is a dynamically sized type that stores its
/// name inline after a fixed-size header holding the arity and the length of
/// the name, implementing [SliceDst] and
/// [Erasable]. This way a symbol is a single allocation, and is referred to by
/// a thin pointer.
#[repr(C)]
pub struct SharedSymbol {
    header: SymbolHeader,
    /// The UTF-8 encoded name of the function
    name: [u8],
}

impl SharedSymbol {
    /// Returns the name of the function symbol
    pub fn name(&self) -> &str {
        // SAFETY: `construct` only ever copies the bytes of a `str` into `name`.
        unsafe { std::str::from_utf8_unchecked(&self.name) }
    }

    /// Returns the arity of the function symbol
    pub fn arity(&self) -> usize {
        self.header.arity.load()
    }

    /// Marks the symbol as reachable and returns true iff it was not marked before.
    pub(crate) fn mark(&self) -> bool {
        !self.header.arity.set_tag()
    }

    /// Returns true iff the symbol is marked as reachable.
    pub(crate) fn is_marked(&self) -> bool {
        self.header.arity.is_tagged()
    }

    /// Removes the reachability mark of the symbol.
    pub(crate) fn unmark(&self) {
        self.header.arity.clear_tag()
    }

    /// Returns a unique index for this shared symbol
    pub fn index(&self) -> usize {
        self as *const Self as *const u8 as usize
    }

    /// Returns the arity of the symbol at `ptr` by reading its header, without
    /// reconstructing the wide pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a live [SharedSymbol].
    pub(crate) unsafe fn arity_of(ptr: ErasedPtr) -> usize {
        // SAFETY: the caller guarantees that `ptr` points to a live symbol, which starts with its header.
        unsafe { ptr.cast::<SymbolHeader>().as_ref().arity.load() }
    }

    /// Constructs a symbol in the uninitialised memory at `ptr`, which must be
    /// allocated for a name of `name.len()` bytes.
    unsafe fn construct(ptr: *mut SharedSymbol, name: &str, arity: usize) {
        // SAFETY: the caller guarantees `ptr` is allocated with the layout for `name.len()` bytes.
        unsafe {
            ptr.cast::<SymbolHeader>().write(SymbolHeader {
                arity: AtomicTaggedUsize::new(arity),
                len: name.len(),
            });
            std::ptr::copy_nonoverlapping(name.as_ptr(), (&raw mut (*ptr).name).cast::<u8>(), name.len());
        }
    }
}

// SAFETY: `layout_for` computes the `#[repr(C)]` layout of the header followed
// by `len` bytes, and `length` returns the name length stored in the header,
// which `construct` sets to the length the symbol was allocated for.
unsafe impl SliceDst for SharedSymbol {
    fn layout_for(len: usize) -> Result<Layout, LayoutError> {
        repr_c(&[Layout::new::<SymbolHeader>(), Layout::array::<u8>(len)?])
    }

    fn retype(ptr: NonNull<[()]>) -> NonNull<Self> {
        // SAFETY: casting a pointer preserves non-nullity and the slice length.
        unsafe { NonNull::new_unchecked(ptr.as_ptr() as *mut Self) }
    }

    fn length(&self) -> usize {
        self.header.len
    }
}

// SAFETY: `unerase` reconstructs the slice length from the header that every
// symbol starts with, so `unerase(erase(p)) == p`.
unsafe impl Erasable for SharedSymbol {
    fn erase(this: NonNull<Self>) -> ErasedPtr {
        this.cast()
    }

    unsafe fn unerase(this: ErasedPtr) -> NonNull<Self> {
        // SAFETY: the caller guarantees that `this` was produced by `erase`, so it points to a symbol header.
        unsafe {
            let len = this.cast::<SymbolHeader>().as_ref().len;
            Self::retype(NonNull::new_unchecked(slice_from_raw_parts_mut(
                this.as_ptr().cast(),
                len,
            )))
        }
    }
}

impl PartialEq for SharedSymbol {
    fn eq(&self, other: &Self) -> bool {
        self.arity() == other.arity() && self.name == other.name
    }
}

impl Eq for SharedSymbol {}

impl fmt::Debug for SharedSymbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedSymbol")
            .field("name", &self.name())
            .field("arity", &self.arity())
            .finish()
    }
}

/// A cheap way to look up SharedSymbol
struct SharedSymbolLookup<'a> {
    name: &'a str,
    arity: usize,
}

impl Equivalent<SharedSymbol> for SharedSymbolLookup<'_> {
    fn equivalent(&self, other: &SharedSymbol) -> bool {
        self.arity == other.arity() && self.name == other.name()
    }
}

/// These hash implementations should be the same as `SharedSymbol`.
impl Hash for SharedSymbolLookup<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
        self.arity.hash(state);
    }
}

impl Hash for SharedSymbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name().hash(state);
        self.arity().hash(state);
    }
}

#[cfg(test)]
mod tests {
    use crate::Symb;
    use crate::Symbol;

    #[test]
    fn test_symbol_sharing() {
        merc_utilities::test_logger();

        let f1 = Symbol::new("f", 2);
        let f2 = Symbol::new("f", 2);

        // Should be the same object
        assert_eq!(f1, f2);
    }

    #[test]
    fn test_symbol_name_and_arity() {
        let empty = Symbol::new("", 0);
        assert_eq!(empty.name(), "");
        assert_eq!(empty.arity(), 0);

        let unicode = Symbol::new("λx.→", 3);
        assert_eq!(unicode.name(), "λx.→");
        assert_eq!(unicode.arity(), 3);

        // Same name with a different arity is a different symbol.
        assert_ne!(Symbol::new("g", 1), Symbol::new("g", 2));
    }

    #[test]
    fn test_symbol_fmt() {
        let n = 42;
        let s1 = Symbol::new_fmt(format_args!("x{n}"), 1);
        let s2 = Symbol::new("x42", 1);
        assert_eq!(s1, s2);
        assert_eq!(s1.name(), "x42");

        // A plain literal takes the `as_str` path.
        assert_eq!(Symbol::new_fmt(format_args!("lit"), 0), Symbol::new("lit", 0));
    }
}
