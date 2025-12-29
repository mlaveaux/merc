use std::cell::UnsafeCell;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::atomic::AtomicUsize;
use std::time::Instant;

use log::debug;

use merc_collections::ProtectionSet;
use merc_io::LargeFormatter;
use merc_sharedmutex::GlobalBfSharedMutex;
use merc_sharedmutex::RecursiveLockReadGuard;
use merc_unsafety::StablePointer;
use merc_utilities::debug_trace;

use crate::ATermIndex;
use crate::ATermRef;
use crate::Markable;
use crate::Symb;
use crate::Symbol;
use crate::SymbolIndex;
use crate::SymbolRef;
use crate::Term;
use crate::storage::ATermStorage;
use crate::storage::SharedTerm;
use crate::storage::SharedTermLookup;
use crate::storage::SymbolPool;

/// This is the global set of protection sets that are managed by the ThreadTermPool
pub static GLOBAL_TERM_POOL: LazyLock<GlobalBfSharedMutex<GlobalTermPool>> =
    LazyLock::new(|| GlobalBfSharedMutex::new(GlobalTermPool::new()));

/// Enables aggressive garbage collection, which is used for testing.
pub(crate) const AGGRESSIVE_GC: bool = false;

/// A type alias for the global term pool guard
pub(crate) type GlobalTermPoolGuard<'a> = RecursiveLockReadGuard<'a, GlobalTermPool>;

/// A type alias for deletion hooks
type DeletionHook = Box<dyn Fn(&ATermIndex) + Sync + Send>;

/// The single global (singleton) term pool.
pub struct GlobalTermPool {
    /// Unique table of all terms with stable pointers for references
    terms: ATermStorage,
    /// The symbol pool for managing function symbols.
    symbol_pool: SymbolPool,
    /// The thread-specific protection sets.
    thread_pools: Vec<Option<Arc<UnsafeCell<SharedTermProtection>>>>,

    // Data structures used for garbage collection
    /// Used to avoid reallocations for the markings of all terms - uses pointers as keys
    marked_terms: HashSet<ATermIndex>,
    /// Used to avoid reallocations for the markings of all symbols
    marked_symbols: HashSet<SymbolIndex>,
    /// A stack used to mark terms recursively.
    stack: Vec<ATermIndex>,

    /// Deletion hooks called whenever a term with the given head symbol is deleted.
    deletion_hooks: Vec<(Symbol, DeletionHook)>,

    /// Indicates whether automatic garbage collection is enabled.
    garbage_collection: bool,

    /// Default terms
    int_symbol: SymbolRef<'static>,
    empty_list_symbol: SymbolRef<'static>,
    list_symbol: SymbolRef<'static>,
}

unsafe impl Send for GlobalTermPool {}
unsafe impl Sync for GlobalTermPool {}

impl GlobalTermPool {
    fn new() -> GlobalTermPool {
        // Insert the default symbols.
        let symbol_pool = SymbolPool::new();
        let int_symbol = unsafe { SymbolRef::from_index(&symbol_pool.create("<aterm_int>", 0)) };
        let list_symbol = unsafe { SymbolRef::from_index(&symbol_pool.create("<list_constructor>", 2)) };
        let empty_list_symbol = unsafe { SymbolRef::from_index(&symbol_pool.create("<empty_list>", 0)) };

        GlobalTermPool {
            terms: ATermStorage::new(),
            symbol_pool,
            thread_pools: Vec::new(),
            marked_terms: HashSet::new(),
            marked_symbols: HashSet::new(),
            stack: Vec::new(),
            deletion_hooks: Vec::new(),
            garbage_collection: true,
            int_symbol,
            list_symbol,
            empty_list_symbol,
        }
    }

    /// Returns the number of terms in the pool.
    pub fn len(&self) -> usize {
        self.terms.len()
    }

    /// Returns whether the term pool is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Creates a term storing a single integer value.
    pub fn create_int(&self, value: usize) -> (StablePointer<SharedTerm>, bool) {
        let shared_term = SharedTermLookup {
            symbol: unsafe { SymbolRef::from_index(self.int_symbol.shared()) },
            arguments: &[],
            annotation: Some(value),
        };

        let (index, inserted) = unsafe {
            self.terms
                .insert_equiv_dst(&shared_term, SharedTerm::length_for(&shared_term), |ptr, key| {
                    SharedTerm::construct(ptr, key)
                })
        };

        (index, inserted)
    }

    /// Create a term from a head symbol and an iterator over its arguments
    pub fn create_term_array<'a, 'b, 'c>(
        &'c self,
        symbol: &'b impl Symb<'a, 'b>,
        args: &'c [ATermRef<'c>],
    ) -> (StablePointer<SharedTerm>, bool) {
        let shared_term = SharedTermLookup {
            symbol: SymbolRef::from_symbol(symbol),
            arguments: args,
            annotation: None,
        };

        debug_assert_eq!(
            symbol.shared().arity(),
            shared_term.arguments.len(),
            "The number of arguments does not match the arity of the symbol"
        );

        let (index, inserted) = unsafe {
            self.terms
                .insert_equiv_dst(&shared_term, SharedTerm::length_for(&shared_term), |ptr, key| {
                    SharedTerm::construct(ptr, key)
                })
        };

        (index, inserted)
    }

    /// Create a function symbol
    pub fn create_symbol<P>(&self, name: impl Into<String> + AsRef<str>, arity: usize, protect: P) -> Symbol
    where
        P: FnOnce(SymbolIndex) -> Symbol,
    {
        protect(self.symbol_pool.create(name, arity))
    }

    /// Registers a new thread term pool.
    ///
    /// # Safety
    ///
    /// Note that the returned `Arc<UnsafeCell<...>>` is not Send or Sync, so it
    /// *must* be protected through other means.
    #[allow(clippy::arc_with_non_send_sync)]
    pub(crate) fn register_thread_term_pool(&mut self) -> Arc<UnsafeCell<SharedTermProtection>> {
        let protection = Arc::new(UnsafeCell::new(SharedTermProtection {
            protection_set: ProtectionSet::new(),
            symbol_protection_set: ProtectionSet::new(),
            container_protection_set: ProtectionSet::new(),
            index: self.thread_pools.len(),
        }));

        debug!("Registered thread_local protection set(s) {}", self.thread_pools.len());
        self.thread_pools.push(Some(protection.clone()));

        protection
    }

    /// Deregisters a thread pool.
    pub(crate) fn deregister_thread_pool(&mut self, index: usize) {
        debug!("Removed thread_local protection set(s) {index}");
        if let Some(entry) = self.thread_pools.get_mut(index) {
            *entry = None;
        }
    }

    /// Triggers garbage collection if necessary and returns an updated counter for the thread local pool.
    pub(crate) fn trigger_garbage_collection(&mut self) -> usize {
        self.collect_garbage();

        if AGGRESSIVE_GC {
            return 1;
        }

        self.len()
    }

    /// Returns a counter for the unique numeric suffix of the given prefix.
    pub fn register_prefix(&self, prefix: &str) -> Arc<AtomicUsize> {
        self.symbol_pool.create_prefix(prefix)
    }

    /// Removes the registration of a prefix from the symbol pool.
    pub fn remove_prefix(&self, prefix: &str) {
        self.symbol_pool.remove_prefix(prefix)
    }

    /// Register a deletion hook that is called whenever a term is deleted with the given symbol.
    pub fn register_deletion_hook<F>(&mut self, symbol: SymbolRef<'static>, hook: F)
    where
        F: Fn(&ATermIndex) + Sync + Send + 'static,
    {
        self.deletion_hooks.push((symbol.protect(), Box::new(hook)));
    }

    /// Enables or disables automatic garbage collection.
    pub fn automatic_garbage_collection(&mut self, enabled: bool) {
        self.garbage_collection = enabled;
    }

    /// Collects garbage terms.
    fn collect_garbage(&mut self) {
        if !self.garbage_collection {
            // Garbage collection is disabled.
            return;
        }

        // Clear marking data structures
        self.marked_terms.clear();
        self.marked_symbols.clear();
        self.stack.clear();

        // Mark the default symbols
        self.marked_symbols.insert(self.int_symbol.shared().copy());
        self.marked_symbols.insert(self.list_symbol.shared().copy());
        self.marked_symbols.insert(self.empty_list_symbol.shared().copy());

        let mut marker = Marker {
            marked_terms: &mut self.marked_terms,
            marked_symbols: &mut self.marked_symbols,
            stack: &mut self.stack,
        };

        let mark_time = Instant::now();

        // Loop through all protection sets and mark the terms.
        for pool in self.thread_pools.iter().flatten() {
            // SAFETY: We have exclusive access to the global term pool, so no other thread can modify the protection sets.
            let pool = unsafe { &mut *pool.get() };

            for (_root, symbol) in pool.symbol_protection_set.iter() {
                debug_trace!("Marking root {_root} symbol {symbol:?}");
                // Remove all symbols that are not protected
                marker.marked_symbols.insert(symbol.copy());
            }

            for (_root, term) in pool.protection_set.iter() {
                debug_trace!("Marking root {_root} term {term:?}");
                unsafe {
                    ATermRef::from_index(term).mark(&mut marker);
                }
            }

            for (_, container) in pool.container_protection_set.iter() {
                container.mark(&mut marker);
            }
        }

        let mark_time_elapsed = mark_time.elapsed();
        let collect_time = Instant::now();

        let num_of_terms = self.len();
        let num_of_symbols = self.symbol_pool.len();

        // Delete all terms that are not marked
        self.terms.retain(|term| {
            if !self.marked_terms.contains(term) {
                debug_trace!("Dropping term: {:?}", term);

                // Call the deletion hooks for the term
                for (symbol, hook) in &self.deletion_hooks {
                    if symbol == term.symbol() {
                        debug_trace!("Calling deletion hook for term: {:?}", term);
                        hook(term);
                    }
                }

                return false;
            }

            true
        });

        // We ensure that every removed symbol is not used anymore.
        self.symbol_pool.retain(|symbol| {
            if !self.marked_symbols.contains(symbol) {
                debug_trace!("Dropping symbol: {:?}", symbol);
                return false;
            }

            true
        });

        debug!(
            "Garbage collection: marking took {}ms, collection took {}ms, {} terms and {} symbols removed",
            mark_time_elapsed.as_millis(),
            collect_time.elapsed().as_millis(),
            num_of_terms - self.len(),
            num_of_symbols - self.symbol_pool.len()
        );

        debug!("{}", self.metrics());

        // Print information from the protection sets.
        for pool in self.thread_pools.iter().flatten() {
            // SAFETY: We have exclusive access to the global term pool, so no other thread can modify the protection sets.
            let pool = unsafe { &mut *pool.get() };
            debug!("{}", pool.metrics());
        }
    }

    /// Returns the metrics of the term pool, can be formatted and written to output.
    pub fn metrics(&self) -> TermPoolMetrics<'_> {
        TermPoolMetrics(self)
    }

    /// Marks the given term as being reachable.
    ///
    /// # Safety
    ///
    /// Should only be called during garbage collection.
    pub unsafe fn mark_term(&mut self, term: &ATermRef<'_>) {
        // Ensure that the global term pool is locked for writing.
        let mut marker = Marker {
            marked_terms: &mut self.marked_terms,
            marked_symbols: &mut self.marked_symbols,
            stack: &mut self.stack,
        };
        term.mark(&mut marker);
    }

    /// Returns integer function symbol.
    pub(crate) fn get_int_symbol(&self) -> &SymbolRef<'static> {
        &self.int_symbol
    }

    /// Returns integer function symbol.
    pub(crate) fn get_list_symbol(&self) -> &SymbolRef<'static> {
        &self.list_symbol
    }

    /// Returns integer function symbol.
    pub(crate) fn get_empty_list_symbol(&self) -> &SymbolRef<'static> {
        &self.empty_list_symbol
    }
}

pub struct TermPoolMetrics<'a>(&'a GlobalTermPool);

impl fmt::Display for TermPoolMetrics<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "There are {} terms, and {} symbols",
            self.0.terms.len(),
            self.0.symbol_pool.len()
        )
    }
}

pub struct SharedTermProtection {
    /// Protection set for terms
    pub protection_set: ProtectionSet<ATermIndex>,
    /// Protection set to prevent garbage collection of symbols
    pub symbol_protection_set: ProtectionSet<SymbolIndex>,
    /// Protection set for containers
    pub container_protection_set: ProtectionSet<Arc<dyn Markable + Sync + Send>>,
    /// Index in global pool's thread pools list
    pub index: usize,
}

impl SharedTermProtection {
    /// Returns the metrics of the term pool, can be formatted and written to output.
    pub fn metrics(&self) -> ProtectionMetrics<'_> {
        ProtectionMetrics(self)
    }
}

/// A struct that can be used to print the performance of the protection sets.
pub struct ProtectionMetrics<'a>(&'a SharedTermProtection);

impl fmt::Display for ProtectionMetrics<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "Protection set {} has {} roots, max {} and {} insertions",
            self.0.index,
            LargeFormatter(self.0.protection_set.len()),
            LargeFormatter(self.0.protection_set.maximum_size()),
            LargeFormatter(self.0.protection_set.number_of_insertions())
        )?;

        writeln!(
            f,
            "Containers: {} roots, max {} and {} insertions",
            LargeFormatter(self.0.container_protection_set.len()),
            LargeFormatter(self.0.container_protection_set.maximum_size()),
            LargeFormatter(self.0.container_protection_set.number_of_insertions()),
        )?;

        write!(
            f,
            "Symbols: {} roots, max {} and {} insertions",
            LargeFormatter(self.0.symbol_protection_set.len()),
            LargeFormatter(self.0.symbol_protection_set.maximum_size()),
            LargeFormatter(self.0.symbol_protection_set.number_of_insertions()),
        )
    }
}

/// Helper struct to pass private data required to mark term recursively.
pub struct Marker<'a> {
    marked_terms: &'a mut HashSet<ATermIndex>,
    marked_symbols: &'a mut HashSet<SymbolIndex>,
    stack: &'a mut Vec<ATermIndex>,
}

impl Marker<'_> {
    // Marks the given term as being reachable.
    pub fn mark(&mut self, term: &ATermRef<'_>) {
        if !self.marked_terms.contains(term.shared()) {
            self.stack.push(term.shared().copy());

            while let Some(term) = self.stack.pop() {
                // Each term should be marked.
                self.marked_terms.insert(term.copy());

                // Mark the function symbol.
                self.marked_symbols.insert(term.symbol().shared().copy());

                // For some terms, such as ATermInt, we must ONLY consider the valid arguments (indicated by the arity)
                for arg in term.arguments()[0..term.symbol().arity()].iter() {
                    // Skip if unnecessary, otherwise mark before pushing to stack since it can be shared.
                    if !self.marked_terms.contains(arg.shared()) {
                        self.marked_terms.insert(arg.shared().copy());
                        self.marked_symbols.insert(arg.get_head_symbol().shared().copy());
                        self.stack.push(arg.shared().copy());
                    }
                }
            }
        }
    }

    /// Marks the given symbol as being reachable.
    pub fn mark_symbol(&mut self, symbol: &SymbolRef<'_>) {
        self.marked_symbols.insert(symbol.shared().copy());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use merc_utilities::random_test;

    use crate::random_term;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_maximal_sharing() {
        random_test(100, |rng| {
            let mut terms = HashMap::new();

            for _ in 0..1000 {
                let term = random_term(rng, &[("f".into(), 2), ("g".into(), 1)], &["a".to_string()], 10);

                let representation = format!("{}", term);
                if let Some(entry) = terms.get(&representation) {
                    assert_eq!(term, *entry, "There is another term with the same representation");
                } else {
                    terms.insert(representation, term);
                }
            }
        });
    }
}
