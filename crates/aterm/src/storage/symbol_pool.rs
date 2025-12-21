#![forbid(unsafe_code)]

use std::hash::Hash;
use std::hash::Hasher;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use dashmap::DashMap;
use equivalent::Equivalent;
use merc_unsafety::StablePointer;
use rustc_hash::FxBuildHasher;

use merc_unsafety::StablePointerSet;

use crate::Symb;
use crate::SymbolIndex;
use crate::SymbolRef;

/// Pool for maximal sharing of function symbols, see [crate::SymbolRef]. Ensures that function symbols
/// with the same name and arity point to the same [SharedSymbol] object.
/// Returns [crate::Symbol] that can be used to refer to the shared symbol, avoiding
/// garbage collection of the underlying shared symbol.
pub struct SymbolPool {
    /// Unique table of all function symbols
    symbols: StablePointerSet<SharedSymbol, FxBuildHasher>,

    /// A map from prefixes to counters that track the next available index for function symbols
    prefix_to_register_function_map: DashMap<String, Arc<AtomicUsize>, FxBuildHasher>,
}

impl SymbolPool {
    /// Creates a new empty symbol pool.
    pub(crate) fn new() -> Self {
        Self {
            symbols: StablePointerSet::with_hasher(FxBuildHasher),
            prefix_to_register_function_map: DashMap::with_hasher(FxBuildHasher),
        }
    }

    /// Creates or retrieves a function symbol with the given name and arity.
    pub fn create<N>(&self, name: N, arity: usize) -> StablePointer<SharedSymbol>
    where
        N: Into<String> + AsRef<str>,
    {
        // Get or create symbol index
        let (shared_symbol, inserted) = self.symbols.insert_equiv(&SharedSymbolLookup { name, arity });

        if inserted {
            // If the symbol was newly created, register its prefix.
            self.update_prefix(shared_symbol.name());
        }

        // Return cloned symbol
        shared_symbol
    }

    /// Return the symbol of the SharedTerm for the given ATermRef
    pub fn symbol_name<'a>(&self, symbol: &'a SymbolRef<'a>) -> &'a str {
        symbol.shared().name()
    }

    /// Returns the arity of the function symbol
    pub fn symbol_arity<'a, 'b>(&self, symbol: &'b impl Symb<'a, 'b>) -> usize {
        symbol.shared().arity()
    }

    /// Returns the number of symbols in the pool.
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    /// Returns true if the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.symbols.is_empty()
    }

    /// Returns the capacity of the pool.
    pub fn capacity(&self) -> usize {
        self.symbols.capacity()
    }

    /// Retain only symbols satisfying the given predicate.
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&SymbolIndex) -> bool,
    {
        self.symbols.retain(|element| f(element));
    }

    /// Creates a new prefix counter for the given prefix.
    pub fn create_prefix(&self, prefix: &str) -> Arc<AtomicUsize> {
        // Create a new counter for the prefix if it does not exist
        let result = match self.prefix_to_register_function_map.get(prefix) {
            Some(result) => result.clone(),
            None => {
                let result = Arc::new(AtomicUsize::new(0));
                assert!(
                    self.prefix_to_register_function_map
                        .insert(prefix.to_string(), result.clone())
                        .is_none(),
                    "This key should not yet exist"
                );
                result
            }
        };

        // Ensure the counter starts at a sufficiently large index
        self.get_sufficiently_large_postfix_index(prefix, &result);
        result
    }

    /// Removes a prefix counter from the pool.
    pub fn remove_prefix(&self, prefix: &str) {
        // Remove the prefix counter if it exists
        self.prefix_to_register_function_map.remove(prefix);
    }

    /// Updates the counter for a registered prefix for the newly created symbol.
    fn update_prefix(&self, name: &str) {
        // Check whether there is a registered prefix p such that name equal pn where n is a number.
        // In that case prevent that pn will be generated as a fresh function name.
        let start_of_index = name
            .rfind(|c: char| !c.is_ascii_digit())
            .map(|pos| pos + 1)
            .unwrap_or(0);

        if start_of_index < name.len() {
            let potential_number = &name[start_of_index..];
            let prefix = &name[..start_of_index];

            if let Some(counter) = self.prefix_to_register_function_map.get(prefix) {
                if let Ok(number) = potential_number.parse::<usize>() {
                    counter.fetch_max(number + 1, Ordering::Relaxed);
                }
            }
        }
    }

    /// Traverse all symbols to find the maximum numeric suffix for this prefix
    fn get_sufficiently_large_postfix_index(&self, prefix: &str, counter: &Arc<AtomicUsize>) {
        for symbol in self.symbols.iter() {
            let name = symbol.name();
            if name.starts_with(prefix) {
                // Symbol name starts with the prefix, check for numeric suffix
                let suffix_start = prefix.len();
                if suffix_start < name.len() {
                    let suffix = &name[suffix_start..];
                    if let Ok(number) = suffix.parse::<usize>() {
                        // There is a numeric suffix, update the counter if it's larger
                        counter.fetch_max(number + 1, Ordering::Relaxed);
                    }
                }
            }
        }
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

impl SharedSymbol {
    /// Creates a new function symbol.
    pub fn new(name: impl Into<String>, arity: usize) -> Self {
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
    use std::sync::atomic::Ordering;

    use crate::Symbol;
    use crate::storage::THREAD_TERM_POOL;

    #[test]
    fn test_symbol_sharing() {
        let _ = merc_utilities::test_logger();

        let f1 = Symbol::new("f", 2);
        let f2 = Symbol::new("f", 2);

        // Should be the same object
        assert_eq!(f1, f2);
    }

    #[test]
    fn test_prefix_counter() {
        let _ = merc_utilities::test_logger();

        let _symbol = Symbol::new("x69", 0);
        let _symbol2 = Symbol::new("x_y", 0);

        let value =
            THREAD_TERM_POOL.with_borrow(|tp| tp.term_pool().write().expect("Lock poisoned!").register_prefix("x"));

        assert_eq!(value.load(Ordering::Relaxed), 70);

        let _symbol3 = Symbol::new("x_no_effect", 0);
        let _symbol4 = Symbol::new("x130", 0);

        assert_eq!(value.load(Ordering::Relaxed), 131);
    }
}
