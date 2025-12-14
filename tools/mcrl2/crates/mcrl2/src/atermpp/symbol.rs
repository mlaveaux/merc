use std::borrow::Borrow;
use std::marker::PhantomData;

use std::cmp::Ordering;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::ops::Deref;

use mcrl2_sys::atermpp::ffi::mcrl2_function_symbol_drop;
use mcrl2_sys::atermpp::ffi::mcrl2_function_symbol_get_arity;
use mcrl2_sys::atermpp::ffi::mcrl2_function_symbol_get_name;
use mcrl2_sys::atermpp::ffi::mcrl2_function_symbol_protect;
use mcrl2_sys::atermpp::ffi::{self};

/// A Symbol references to an aterm function symbol, which has a name and an arity.
#[derive(Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct SymbolRef<'a> {
    symbol: *const ffi::_function_symbol,
    marker: PhantomData<&'a ()>,
}

impl<'a> SymbolRef<'a> {
    /// Protects the symbol and returns an owned Symbol
    pub fn protect(&self) -> Symbol {
        Symbol::new(self.symbol)
    }

    /// Creates a (cheap) copy of the SymbolRef
    pub fn copy(&self) -> SymbolRef<'_> {
        SymbolRef::new(self.symbol)
    }

    /// Creates a new SymbolRef from the given pointer, does not change protection.
    pub(crate) fn new(symbol: *const ffi::_function_symbol) -> SymbolRef<'a> {
        SymbolRef {
            symbol,
            marker: PhantomData,
        }
    }

    /// Obtains the underlying pointer as reference
    pub(crate) fn get(&self) -> &ffi::_function_symbol {
        // # Safety
        // If we have a reference to the SymbolRef, it must also be safe to dereference the pointer.
        unsafe { self.symbol.as_ref().expect("Pointer should be valid") }
    }
}

impl SymbolRef<'_> {
    /// Obtain the symbol's name
    pub fn name(&self) -> &str {
        mcrl2_function_symbol_get_name(self.get())
    }

    /// Obtain the symbol's arity
    pub fn arity(&self) -> usize {
        mcrl2_function_symbol_get_arity(self.get())
    }

    /// Returns the index of the function symbol
    pub fn address(&self) -> *const ffi::_function_symbol {
        self.symbol
    }
}

impl fmt::Display for SymbolRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl fmt::Debug for SymbolRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{} [{}]", self.name(), self.arity(), self.address() as usize,)
    }
}

impl From<*const ffi::_function_symbol> for SymbolRef<'_> {
    fn from(symbol: *const ffi::_function_symbol) -> Self {
        SymbolRef {
            symbol,
            marker: PhantomData,
        }
    }
}

/// A Symbol owns a function symbol from the aterm pool.
pub struct Symbol {
    symbol: SymbolRef<'static>,
}

impl Symbol {
    /// Takes ownership of the given pointer without changing the reference counter.
    pub(crate) fn take(symbol: *const ffi::_function_symbol) -> Symbol {
        Symbol {
            symbol: SymbolRef::new(symbol),
        }
    }

    /// Protects the given pointer.
    pub(crate) fn new(symbol: *const ffi::_function_symbol) -> Symbol {
        let result = Symbol {
            symbol: SymbolRef::new(symbol),
        };
        mcrl2_function_symbol_protect(result.get());
        result
    }

    pub fn get(&self) -> &ffi::_function_symbol {
        unsafe { self.symbol.address().as_ref().expect("Pointer should be valid") }
    }
}

impl Drop for Symbol {
    fn drop(&mut self) {
        mcrl2_function_symbol_drop(self.get());
    }
}

impl Symbol {
    pub fn copy(&self) -> SymbolRef<'_> {
        self.symbol.copy()
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

impl PartialOrd for Symbol {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(&other))
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
