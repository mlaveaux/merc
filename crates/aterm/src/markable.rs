#![forbid(unsafe_code)]

use std::collections::VecDeque;

use merc_collections::IndexedSet;

use crate::SymbolRef;
use crate::aterm::ATermRef;
use crate::storage::GcMutex;
use crate::storage::Marker;

/// This trait should be used on all objects and containers related to storing unprotected terms, or unprotected symmbols.
///
/// The implementation should mark all contained aterms and symbols that must be kept alive using the provided `Marker`.
pub trait Markable {
    /// Marks all the ATermRefs to prevent them from being garbage collected.
    fn mark(&self, marker: &mut Marker);

    /// Should return true iff the given term is contained in the object. Used for runtime checks.
    fn contains_term(&self, term: &ATermRef<'_>) -> bool;

    /// Should return true iff the given symbol is contained in the object. Used for runtime checks.
    fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool;

    /// Returns the number of terms in the instance, used to delay garbage collection.
    fn len(&self) -> usize;

    /// Returns true iff the container is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T: Markable> Markable for Vec<T> {
    fn mark(&self, marker: &mut Marker) {
        for value in self {
            value.mark(marker);
        }
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        self.iter().any(|v| v.contains_term(term))
    }

    fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool {
        self.iter().any(|v| v.contains_symbol(symbol))
    }

    fn len(&self) -> usize {
        self.len()
    }
}

impl<T: Markable> Markable for VecDeque<T> {
    fn mark(&self, marker: &mut Marker) {
        for value in self {
            value.mark(marker);
        }
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        self.iter().any(|v| v.contains_term(term))
    }

    fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool {
        self.iter().any(|v| v.contains_symbol(symbol))
    }

    fn len(&self) -> usize {
        self.len()
    }
}

impl<T: Markable> Markable for GcMutex<T> {
    fn mark(&self, marker: &mut Marker) {
        self.write().mark(marker);
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        self.read().contains_term(term)
    }

    fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool {
        self.read().contains_symbol(symbol)
    }

    fn len(&self) -> usize {
        self.read().len()
    }
}

impl<T: Markable> Markable for IndexedSet<T> {
    fn mark(&self, marker: &mut Marker) {
        for (_, value) in self.iter() {
            value.mark(marker);
        }
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        self.iter().any(|(_, v)| v.contains_term(term))
    }

    fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool {
        self.iter().any(|(_, v)| v.contains_symbol(symbol))
    }

    fn len(&self) -> usize {
        self.len()
    }
}

impl<T: Markable> Markable for Option<T> {
    fn mark(&self, marker: &mut Marker) {
        if let Some(value) = self {
            value.mark(marker);
        }
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        if let Some(value) = self {
            value.contains_term(term)
        } else {
            false
        }
    }

    fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool {
        if let Some(value) = self {
            value.contains_symbol(symbol)
        } else {
            false
        }
    }

    fn len(&self) -> usize {
        if let Some(value) = self { value.len() } else { 0 }
    }
}

// In Rust Its not yet possible to implement it for any tuples, so we implement it for some common sizes.
impl<T1: Markable, T2: Markable> Markable for (T1, T2) {
    fn mark(&self, marker: &mut Marker) {
        self.0.mark(marker);
        self.1.mark(marker);
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        self.0.contains_term(term) || self.1.contains_term(term)
    }

    fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool {
        self.0.contains_symbol(symbol) || self.1.contains_symbol(symbol)
    }

    fn len(&self) -> usize {
        self.0.len() + self.1.len()
    }
}

impl Markable for bool {
    fn mark(&self, _marker: &mut Marker) {
        // Nothing to mark
    }

    fn contains_term(&self, _term: &ATermRef<'_>) -> bool {
        false
    }

    fn contains_symbol(&self, _symbol: &SymbolRef<'_>) -> bool {
        false
    }

    fn len(&self) -> usize {
        0
    }
}
