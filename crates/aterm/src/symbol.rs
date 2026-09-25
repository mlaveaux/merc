use std::cmp::Ordering;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::marker::PhantomData;

use delegate::delegate;

use merc_unsafety::ProtectionIndex;
use merc_unsafety::StablePointer;

use crate::Markable;
use crate::storage::Marker;
use crate::storage::SharedSymbol;
use crate::storage::THREAD_TERM_POOL;

/// The public interface for a function symbol. Can be used to write generic
/// functions that accept both [Symbol] and [SymbolRef].
///
/// See [crate::Term] for more information on how to use this trait with two lifetimes.
pub trait Symb<'a, 'b> {
    /// Obtain the symbol's name.
    fn name(&'b self) -> &'a str;

    /// Obtain the symbol's arity.
    fn arity(&self) -> usize;

    /// Create a copy of the symbol reference.
    fn copy(&'b self) -> SymbolRef<'a>;

    /// Returns a unique index for the symbol.
    fn index(&self) -> usize;

    /// Returns the underlying pool index of the symbol.
    fn shared(&self) -> &SymbolIndex;
}

/// An alias for the type that is used to reference into the [SharedSymbol] set.
pub type SymbolIndex = StablePointer<SharedSymbol>;

/// A reference to a function symbol in the symbol pool.
#[repr(transparent)]
#[derive(Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct SymbolRef<'a> {
    shared: SymbolIndex,
    marker: PhantomData<&'a ()>,
}

/// Check that the SymbolRef is the same size as a usize.
#[cfg(not(debug_assertions))]
const _: () = assert!(std::mem::size_of::<SymbolRef>() == std::mem::size_of::<usize>());

/// Check that the Option<SymbolRef> is the same size as a usize using niche value optimisation.
#[cfg(not(debug_assertions))]
const _: () = assert!(std::mem::size_of::<Option<SymbolRef>>() == std::mem::size_of::<usize>());

/// A reference to a function symbol with a known lifetime.
impl<'a> SymbolRef<'a> {
    /// Protects the symbol from garbage collection, yielding a `Symbol`.
    pub fn protect(&self) -> Symbol {
        THREAD_TERM_POOL.with(|tp| tp.protect_symbol(self))
    }

    /// Internal constructor to create a [SymbolRef] from a [SymbolIndex].
    ///
    /// # Safety
    ///
    /// We must ensure that the lifetime `'a` is valid for the returned `SymbolRef`.
    pub unsafe fn from_index(index: &SymbolIndex) -> SymbolRef<'a> {
        SymbolRef {
            // SAFETY: the caller guarantees the index remains valid for `'a`.
            shared: unsafe { index.copy() },
            marker: PhantomData,
        }
    }
}

impl SymbolRef<'_> {
    /// Internal constructor to convert any `Symb` to a `SymbolRef`.
    pub(crate) fn from_symbol<'a, 'b, S: Symb<'a, 'b>>(symbol: &'b S) -> Self {
        SymbolRef {
            // SAFETY: `symbol` keeps the index alive during this call.
            shared: unsafe { symbol.shared().copy() },
            marker: PhantomData,
        }
    }
}

impl<'a> Symb<'a, '_> for SymbolRef<'a> {
    fn name(&self) -> &'a str {
        unsafe { std::mem::transmute(self.shared.deref().name()) }
    }

    fn arity(&self) -> usize {
        // SAFETY: `self` is a `SymbolRef<'a>`, so the symbol is kept alive for
        // `'a` and the pointee is valid to read.
        unsafe { self.shared.deref() }.arity()
    }

    fn copy(&self) -> SymbolRef<'a> {
        unsafe { SymbolRef::from_index(self.shared()) }
    }

    fn index(&self) -> usize {
        // SAFETY: see `arity`; the symbol is alive for `'a`.
        unsafe { self.shared.deref() }.index()
    }

    fn shared(&self) -> &SymbolIndex {
        &self.shared
    }
}

impl Markable for SymbolRef<'_> {
    fn mark(&self, marker: &mut Marker) {
        marker.mark_symbol(self);
    }

    fn contains_term(&self, _term: &crate::aterm::ATermRef<'_>) -> bool {
        false
    }

    fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool {
        self == symbol
    }

    fn len(&self) -> usize {
        1
    }
}

impl fmt::Display for SymbolRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl fmt::Debug for SymbolRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// A protected function symbol, with the same interface as [SymbolRef].
pub struct Symbol {
    symbol: SymbolRef<'static>,
    root: ProtectionIndex,
}

impl Symbol {
    /// Create a new symbol with the given name and arity.
    pub fn new<N>(name: N, arity: usize) -> Symbol
    where
        N: Into<String> + AsRef<str>,
    {
        THREAD_TERM_POOL.with(|tp| tp.create_symbol(name, arity))
    }
}

impl Symbol {
    /// Internal constructor to create a symbol from an index and a root.
    pub(crate) unsafe fn from_index(index: &SymbolIndex, root: ProtectionIndex) -> Symbol {
        Self {
            symbol: unsafe { SymbolRef::from_index(index) },
            root,
        }
    }

    /// Returns the root index, i.e., the index in the thread's protection set.
    pub fn root(&self) -> ProtectionIndex {
        self.root
    }

    /// Create a copy of the symbol reference.
    pub fn copy(&self) -> SymbolRef<'_> {
        self.symbol.copy()
    }

    /// Returns the symbol as a borrowed [SymbolRef] whose lifetime is bounded
    /// by this protected symbol.
    pub fn get(&self) -> &SymbolRef<'_> {
        &self.symbol
    }
}

impl<'a> Symb<'a, '_> for &'a Symbol {
    delegate! {
        to self.symbol {
            fn name(&self) -> &'a str;
            fn arity(&self) -> usize;
            fn copy(&self) -> SymbolRef<'a>;
            fn index(&self) -> usize;
            fn shared(&self) -> &SymbolIndex;
        }
    }
}

impl<'a, 'b> Symb<'a, 'b> for Symbol
where
    'b: 'a,
{
    delegate! {
        to self.symbol {
            fn name(&self) -> &'a str;
            fn arity(&self) -> usize;
            fn copy(&self) -> SymbolRef<'a>;
            fn index(&self) -> usize;
            fn shared(&self) -> &SymbolIndex;
        }
    }
}

impl Drop for Symbol {
    fn drop(&mut self) {
        THREAD_TERM_POOL.with(|tp| {
            tp.drop_symbol(self);
        })
    }
}

impl From<&SymbolRef<'_>> for Symbol {
    fn from(value: &SymbolRef) -> Self {
        value.protect()
    }
}

impl Clone for Symbol {
    fn clone(&self) -> Self {
        self.copy().protect()
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl Hash for Symbol {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.copy().hash(state)
    }
}

impl PartialEq for Symbol {
    fn eq(&self, other: &Self) -> bool {
        self.copy().eq(&other.copy())
    }
}

impl PartialEq<SymbolRef<'_>> for Symbol {
    fn eq(&self, other: &SymbolRef<'_>) -> bool {
        self.copy().eq(other)
    }
}

impl PartialOrd for Symbol {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Symbol {
    fn cmp(&self, other: &Self) -> Ordering {
        self.copy().cmp(&other.copy())
    }
}

impl Eq for Symbol {}

#[cfg(kani)]
mod verification {
    use std::ptr::NonNull;

    use merc_unsafety::StablePointer;

    use super::*;

    /// `SymbolRef::from_index` must produce a reference whose pointer identity matches the
    /// source index exactly, and whose subsequent `name()`/`arity()` reads observe the same
    /// `SharedSymbol` the index pointed at -- the property every other unsafe read of a
    /// `SymbolRef` in this crate (`arity`, `index`, `Markable::mark_symbol`, ...) relies on.
    /// This does not, and cannot, check the temporal half of `from_index`'s safety contract
    /// (that `'a` does not outlive the pointee) -- Kani verifies a single bounded execution, not
    /// a claim about the future, so that half is covered by the mathematical `# Safety` contract
    /// on `from_index` and by the GC/protection-set regression tests instead.
    #[kani::proof]
    fn symbol_ref_from_index_preserves_identity_and_reads() {
        let arity: usize = kani::any();
        kani::assume(arity <= 16);

        let boxed = Box::new(SharedSymbol::new("s", arity));
        let ptr = NonNull::from(Box::leak(boxed));
        // SAFETY: `ptr` points at a leaked allocation that is live for the rest of this
        // (single, bounded) proof execution.
        let index: SymbolIndex = unsafe { StablePointer::from_ptr(ptr) };

        // SAFETY: the elided `'a` does not outlive this proof's scope, and `index` stays valid
        // for that whole scope.
        let symbol_ref: SymbolRef<'_> = unsafe { SymbolRef::from_index(&index) };

        assert_eq!(symbol_ref.shared().ptr(), index.ptr());
        assert_eq!(symbol_ref.arity(), arity);
        assert_eq!(symbol_ref.name(), "s");
    }
}
