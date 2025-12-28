use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::marker::PhantomData;
use std::ops::Deref;

use delegate::delegate;

use merc_collections::ProtectionIndex;
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

    /// TODO: How to actually hide this implementation?
    fn shared(&self) -> &SymbolIndex;
}

/// An alias for the type that is used to reference into the symbol set.
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
        THREAD_TERM_POOL.with_borrow(|tp| tp.protect_symbol(self))
    }

    /// Internal constructor to create a `SymbolRef` from a `SymbolIndex`.
    ///
    /// # Safety
    ///
    /// We must ensure that the lifetime `'a` is valid for the returned `SymbolRef`.
    pub unsafe fn from_index(index: &SymbolIndex) -> SymbolRef<'a> {
        SymbolRef {
            shared: index.copy(),
            marker: PhantomData,
        }
    }
}

impl SymbolRef<'_> {
    /// Internal constructo to convert any `Symb` to a `SymbolRef`.
    pub(crate) fn from_symbol<'a, 'b>(symbol: &'b impl Symb<'a, 'b>) -> Self {
        SymbolRef {
            shared: symbol.shared().copy(),
            marker: PhantomData,
        }
    }
}

impl<'a> Symb<'a, '_> for SymbolRef<'a> {
    fn name(&self) -> &'a str {
        unsafe { std::mem::transmute(self.shared.name()) }
    }

    fn arity(&self) -> usize {
        self.shared.arity()
    }

    fn copy(&self) -> SymbolRef<'a> {
        unsafe { SymbolRef::from_index(self.shared()) }
    }

    fn index(&self) -> usize {
        self.shared.index()
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
    pub fn new(name: impl Into<String> + AsRef<str>, arity: usize) -> Symbol {
        THREAD_TERM_POOL.with_borrow(|tp| tp.create_symbol(name, arity))
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

    /// Returns the root index, i.e., the index in the protection set. See `SharedTermProtection`.
    pub fn root(&self) -> ProtectionIndex {
        self.root
    }

    /// Create a copy of the symbol reference.
    pub fn copy(&self) -> SymbolRef<'_> {
        self.symbol.copy()
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
        THREAD_TERM_POOL.with_borrow(|tp| {
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

impl Deref for Symbol {
    type Target = SymbolRef<'static>;

    fn deref(&self) -> &Self::Target {
        &self.symbol
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

impl Borrow<SymbolRef<'static>> for Symbol {
    fn borrow(&self) -> &SymbolRef<'static> {
        &self.symbol
    }
}

impl Eq for Symbol {}
