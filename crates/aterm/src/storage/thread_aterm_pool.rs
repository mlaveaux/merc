use std::cell::Cell;
use std::cell::RefCell;
use std::cell::UnsafeCell;
use std::ops::Deref;
use std::ops::DerefMut;
use std::sync::Arc;

use log::debug;

use merc_collections::ProtectionIndex;
use merc_pest_consume::Parser;
use merc_sharedmutex::RecursiveLock;
use merc_sharedmutex::RecursiveLockReadGuard;
use merc_unsafety::StablePointer;
use merc_utilities::MercError;
use merc_utilities::debug_trace;

use crate::Markable;
use crate::Return;
use crate::Rule;
use crate::Symb;
use crate::Symbol;
use crate::SymbolRef;
use crate::Term;
use crate::TermParser;
use crate::aterm::ATerm;
use crate::aterm::ATermRef;
use crate::storage::AGGRESSIVE_GC;
use crate::storage::GlobalTermPool;
use crate::storage::SharedTerm;
use crate::storage::SharedTermProtection;
use crate::storage::global_aterm_pool::GLOBAL_TERM_POOL;

thread_local! {
    /// Thread-specific term pool that manages protection sets.
    pub static THREAD_TERM_POOL: RefCell<ThreadTermPool> = RefCell::new(ThreadTermPool::new());
}

/// Per-thread term pool managing local protection sets.
pub struct ThreadTermPool {
    /// A reference to the protection set of this thread pool.
    protection_set: Arc<UnsafeCell<SharedTermProtection>>,

    /// The number of times termms have been created before garbage collection is triggered.
    garbage_collection_counter: Cell<usize>,

    /// A vector of terms that are used to store the arguments of a term for loopup.
    tmp_arguments: RefCell<Vec<ATermRef<'static>>>,

    /// A local view for the global term pool.
    term_pool: RecursiveLock<GlobalTermPool>,

    /// Copy of the default terms since thread local access is cheaper.
    int_symbol: SymbolRef<'static>,
    empty_list_symbol: SymbolRef<'static>,
    list_symbol: SymbolRef<'static>,
}

impl ThreadTermPool {
    /// Creates a new thread-local term pool.
    fn new() -> Self {
        // Register protection sets with global pool
        let term_pool: RecursiveLock<GlobalTermPool> = RecursiveLock::from_mutex(GLOBAL_TERM_POOL.share());

        let mut pool = term_pool.write().expect("Lock poisoned!");

        let protection_set = pool.register_thread_term_pool();
        let int_symbol = pool.get_int_symbol().copy();
        let empty_list_symbol = pool.get_empty_list_symbol().copy();
        let list_symbol = pool.get_list_symbol().copy();
        drop(pool);

        // Arbitrary value to trigger garbage collection
        Self {
            protection_set,
            garbage_collection_counter: Cell::new(if AGGRESSIVE_GC { 1 } else { 1000 }),
            tmp_arguments: RefCell::new(Vec::new()),
            int_symbol,
            empty_list_symbol,
            list_symbol,
            term_pool,
        }
    }

    /// Creates a term without arguments.
    pub fn create_constant(&self, symbol: &SymbolRef<'_>) -> ATerm {
        assert!(symbol.arity() == 0, "A constant should not have arity > 0");

        let empty_args: [ATermRef<'_>; 0] = [];
        let guard = self.term_pool.read_recursive().expect("Lock poisoned!");

        let (index, inserted) = guard.create_term_array(symbol, &empty_args);

        if inserted {
            self.trigger_garbage_collection();
        }

        self.protect_guard(guard, &unsafe { ATermRef::from_index(&index) })
    }

    /// Create a term with the given arguments
    pub fn create_term<'a, 'b>(
        &self,
        symbol: &'b impl Symb<'a, 'b>,
        args: &'b [impl Term<'a, 'b>],
    ) -> Return<ATermRef<'static>> {
        let mut arguments = self.tmp_arguments.borrow_mut();

        arguments.clear();
        for arg in args {
            unsafe {
                arguments.push(ATermRef::from_index(arg.shared()));
            }
        }

        let guard = self.term_pool.read_recursive().expect("Lock poisoned!");

        let (index, inserted) = guard.create_term_array(symbol, &arguments);

        if inserted {
            self.trigger_garbage_collection();
        }

        unsafe {
            // SAFETY: The guard is guaranteed to live as long as the returned term, since it is thread local and Return cannot be sended to other threads.
            Return::new(
                std::mem::transmute::<RecursiveLockReadGuard<'_, _>, RecursiveLockReadGuard<'static, _>>(guard),
                ATermRef::from_index(&index),
            )
        }
    }

    /// Create a term with the given index.
    pub fn create_int(&self, value: usize) -> ATerm {
        let guard = self.term_pool.read_recursive().expect("Lock poisoned!");

        let (index, inserted) = guard.create_int(value);

        if inserted {
            self.trigger_garbage_collection();
        }

        self.protect_guard(guard, &unsafe { ATermRef::from_index(&index) })
    }

    /// Create a term with the given arguments given by the iterator.
    pub fn create_term_iter<'a, 'b, 'c, 'd, I, T>(&self, symbol: &'b impl Symb<'a, 'b>, args: I) -> ATerm
    where
        I: IntoIterator<Item = T>,
        T: Term<'c, 'd>,
    {
        let mut arguments = self.tmp_arguments.borrow_mut();
        arguments.clear();
        for arg in args {
            unsafe {
                arguments.push(ATermRef::from_index(arg.shared()));
            }
        }

        let guard = self.term_pool.read_recursive().expect("Lock poisoned!");

        let (index, inserted) = guard.create_term_array(symbol, &arguments);

        if inserted {
            self.trigger_garbage_collection();
        }

        self.protect_guard(guard, &unsafe { ATermRef::from_index(&index) })
    }

    /// Create a term with the given arguments given by the iterator that is failable.
    pub fn try_create_term_iter<'a, 'b, 'c, 'd, I, T>(
        &self,
        symbol: &'b impl Symb<'a, 'b>,
        args: I,
    ) -> Result<ATerm, MercError>
    where
        I: IntoIterator<Item = Result<T, MercError>>,
        T: Term<'c, 'd>,
    {
        let mut arguments = self.tmp_arguments.borrow_mut();
        arguments.clear();
        for arg in args {
            unsafe {
                arguments.push(ATermRef::from_index(arg?.shared()));
            }
        }

        let guard = self.term_pool.read_recursive().expect("Lock poisoned!");

        let (index, inserted) = guard.create_term_array(symbol, &arguments);

        if inserted {
            self.trigger_garbage_collection();
        }

        Ok(self.protect_guard(guard, &unsafe { ATermRef::from_index(&index) }))
    }

    /// Create a term with the given arguments given by the iterator.
    pub fn create_term_iter_head<'a, 'b, 'c, 'd, 'e, 'f, I, T>(
        &self,
        symbol: &'b impl Symb<'a, 'b>,
        head: &'d impl Term<'c, 'd>,
        args: I,
    ) -> ATerm
    where
        I: IntoIterator<Item = T>,
        T: Term<'e, 'f>,
    {
        let mut arguments = self.tmp_arguments.borrow_mut();
        arguments.clear();
        unsafe {
            arguments.push(ATermRef::from_index(head.shared()));
        }
        for arg in args {
            unsafe {
                arguments.push(ATermRef::from_index(arg.shared()));
            }
        }

        let guard = self.term_pool.read_recursive().expect("Lock poisoned!");

        let (index, inserted) = guard.create_term_array(symbol, &arguments);

        if inserted {
            self.trigger_garbage_collection();
        }

        self.protect_guard(guard, &unsafe { ATermRef::from_index(&index) })
    }

    /// Create a function symbol
    pub fn create_symbol(&self, name: impl Into<String> + AsRef<str>, arity: usize) -> Symbol {
        self.term_pool
            .read_recursive()
            .expect("Lock poisoned!")
            .create_symbol(name, arity, |index| unsafe {
                self.protect_symbol(&SymbolRef::from_index(&index))
            })
    }

    /// Protect the term by adding its index to the protection set
    pub fn protect(&self, term: &ATermRef<'_>) -> ATerm {
        // Protect the term by adding its index to the protection set
        let root = self.lock_protection_set().protection_set.protect(term.shared().copy());

        // Return the protected term
        let result = ATerm::from_index(term.shared(), root);

        debug_trace!(
            "Protected term {:?}, root {}, protection set {}",
            term,
            root,
            self.index()
        );

        result
    }

    /// Protect the term by adding its index to the protection set
    pub fn protect_guard(&self, _guard: RecursiveLockReadGuard<'_, GlobalTermPool>, term: &ATermRef<'_>) -> ATerm {
        // Protect the term by adding its index to the protection set
        // SAFETY: If the global term pool is locked, so we can safely access the protection set.
        let root = unsafe { &mut *self.protection_set.get() }
            .protection_set
            .protect(term.shared().copy());

        // Return the protected term
        let result = ATerm::from_index(term.shared(), root);

        debug_trace!(
            "Protected term {:?}, root {}, protection set {}",
            term,
            root,
            self.index()
        );

        result
    }

    /// Unprotects a term from this thread's protection set.
    pub fn drop(&self, term: &ATerm) {
        self.lock_protection_set().protection_set.unprotect(term.root());

        debug_trace!(
            "Unprotected term {:?}, root {}, protection set {}",
            term,
            term.root(),
            self.index()
        );
    }

    /// Protects a container in this thread's container protection set.
    pub fn protect_container(&self, container: Arc<dyn Markable + Send + Sync>) -> ProtectionIndex {
        let root = self.lock_protection_set().container_protection_set.protect(container);

        debug_trace!("Protected container index {}, protection set {}", root, self.index());

        root
    }

    /// Unprotects a container from this thread's container protection set.
    pub fn drop_container(&self, root: ProtectionIndex) {
        self.lock_protection_set().container_protection_set.unprotect(root);

        debug_trace!("Unprotected container index {}, protection set {}", root, self.index());
    }

    /// Parse the given string and returns the Term representation.
    pub fn from_string(&self, text: &str) -> Result<ATerm, MercError> {
        let mut result = TermParser::parse(Rule::TermSpec, text)?;
        let root = result.next().unwrap();

        Ok(TermParser::TermSpec(root).unwrap())
    }

    /// Protects a symbol from garbage collection.
    pub fn protect_symbol(&self, symbol: &SymbolRef<'_>) -> Symbol {
        let result = unsafe {
            Symbol::from_index(
                symbol.shared(),
                self.lock_protection_set()
                    .symbol_protection_set
                    .protect(symbol.shared().copy()),
            )
        };

        debug_trace!(
            "Protected symbol {}, root {}, protection set {}",
            symbol,
            result.root(),
            lock.index,
        );

        result
    }

    /// Unprotects a symbol, allowing it to be garbage collected.
    pub fn drop_symbol(&self, symbol: &mut Symbol) {
        self.lock_protection_set()
            .symbol_protection_set
            .unprotect(symbol.root());
    }

    /// Returns the symbol for ATermInt
    pub fn int_symbol(&self) -> &SymbolRef<'_> {
        &self.int_symbol
    }

    /// Returns the symbol for ATermList
    pub fn list_symbol(&self) -> &SymbolRef<'_> {
        &self.list_symbol
    }

    /// Returns the symbol for the empty ATermInt
    pub fn empty_list_symbol(&self) -> &SymbolRef<'_> {
        &self.empty_list_symbol
    }

    /// Enables or disables automatic garbage collection.
    pub fn automatic_garbage_collection(&self, enabled: bool) {
        let mut guard = self.term_pool.write().expect("Lock poisoned!");
        guard.automatic_garbage_collection(enabled);
    }

    /// Returns access to the shared protection set.
    pub(crate) fn get_protection_set(&self) -> &Arc<UnsafeCell<SharedTermProtection>> {
        &self.protection_set
    }

    /// Returns a reference to the global term pool.
    pub(crate) fn term_pool(&self) -> &RecursiveLock<GlobalTermPool> {
        &self.term_pool
    }

    /// Replace the entry in the protection set with the given term.
    pub(crate) fn replace(
        &self,
        _guard: RecursiveLockReadGuard<'_, GlobalTermPool>,
        root: ProtectionIndex,
        term: StablePointer<SharedTerm>,
    ) {
        // Protect the term by adding its index to the protection set
        // SAFETY: If the global term pool is locked, so we can safely access the protection set.
        unsafe { &mut *self.protection_set.get() }
            .protection_set
            .replace(root, term);
    }

    /// This triggers the global garbage collection based on heuristics.
    fn trigger_garbage_collection(&self) {
        // If the term was newly inserted, decrease the garbage collection counter and trigger garbage collection if necessary
        let mut value = self.garbage_collection_counter.get();
        value = value.saturating_sub(1);

        if value == 0 && !self.term_pool.is_locked() {
            // Trigger garbage collection and acquire a new counter value.
            value = self
                .term_pool
                .write()
                .expect("Lock poisoned!")
                .trigger_garbage_collection();
        }

        self.garbage_collection_counter.set(value);
    }

    /// Returns the index of the protection set.
    fn index(&self) -> usize {
        self.lock_protection_set().index
    }

    /// The protection set is locked by the global read-write lock
    fn lock_protection_set(&self) -> ProtectionSetGuard<'_> {
        let guard = self.term_pool.read_recursive().expect("Lock poisoned!");
        let protection_set = unsafe { &mut *self.protection_set.get() };

        ProtectionSetGuard::new(guard, protection_set)
    }
}

impl Drop for ThreadTermPool {
    fn drop(&mut self) {
        let mut write = self.term_pool.write().expect("Lock poisoned!");

        debug!("{}", write.metrics());
        write.deregister_thread_pool(self.index());

        debug!("{}", unsafe { &mut *self.protection_set.get() }.metrics());
        debug!(
            "Acquired {} read locks and {} write locks",
            self.term_pool.read_recursive_call_count(),
            self.term_pool.write_call_count()
        )
    }
}

struct ProtectionSetGuard<'a> {
    _guard: RecursiveLockReadGuard<'a, GlobalTermPool>,
    object: &'a mut SharedTermProtection,
}

impl ProtectionSetGuard<'_> {
    fn new<'a>(
        guard: RecursiveLockReadGuard<'a, GlobalTermPool>,
        object: &'a mut SharedTermProtection,
    ) -> ProtectionSetGuard<'a> {
        ProtectionSetGuard { _guard: guard, object }
    }
}

impl Deref for ProtectionSetGuard<'_> {
    type Target = SharedTermProtection;

    fn deref(&self) -> &Self::Target {
        self.object
    }
}

impl DerefMut for ProtectionSetGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.object
    }
}

#[cfg(test)]
mod tests {
    use crate::Term;

    use super::*;
    use std::thread;

    #[test]
    fn test_thread_local_protection() {
        let _ = merc_utilities::test_logger();

        thread::scope(|scope| {
            for _ in 0..3 {
                scope.spawn(|| {
                    // Create and protect some terms
                    let symbol = Symbol::new("test", 0);
                    let term = ATerm::constant(&symbol);
                    let protected = term.protect();

                    // Verify protection
                    THREAD_TERM_POOL.with_borrow(|tp| {
                        assert!(tp.lock_protection_set().protection_set.contains_root(protected.root()));
                    });

                    // Unprotect
                    let root = protected.root();
                    drop(protected);

                    THREAD_TERM_POOL.with_borrow(|tp| {
                        assert!(!tp.lock_protection_set().protection_set.contains_root(root));
                    });
                });
            }
        });
    }

    #[test]
    fn test_parsing() {
        let _ = merc_utilities::test_logger();

        let t = ATerm::from_string("f(g(a),b)").unwrap();

        assert!(t.get_head_symbol().name() == "f");
        assert!(t.arg(0).get_head_symbol().name() == "g");
        assert!(t.arg(1).get_head_symbol().name() == "b");
    }

    #[test]
    fn test_create_term() {
        let _ = merc_utilities::test_logger();

        let f = Symbol::new("f", 2);
        let g = Symbol::new("g", 1);

        let t = THREAD_TERM_POOL.with_borrow(|tp| {
            tp.create_term(
                &f,
                &[
                    tp.create_term(&g, &[tp.create_constant(&Symbol::new("a", 0))])
                        .protect(),
                    tp.create_constant(&Symbol::new("b", 0)),
                ],
            )
            .protect()
        });

        assert!(t.get_head_symbol().name() == "f");
        assert!(t.arg(0).get_head_symbol().name() == "g");
        assert!(t.arg(1).get_head_symbol().name() == "b");
    }
}
