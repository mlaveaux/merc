use std::cell::Cell;
use std::cell::RefCell;
use std::cell::UnsafeCell;
use std::iter;
use std::mem::ManuallyDrop;
use std::ops::Deref;
use std::ops::DerefMut;
use std::sync::Arc;
use std::sync::Mutex;

use log::debug;

use merc_pest_consume::Parser;
use merc_sharedmutex::RecursiveLock;
use merc_sharedmutex::RecursiveLockReadGuard;
use merc_unsafety::ProtectionIndex;
use merc_unsafety::ProtectionSet;
use merc_unsafety::StablePointer;
use merc_utilities::MercError;
use merc_utilities::debug_trace;

use crate::ATermIndex;
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
use crate::storage::GlobalTermPool;
use crate::storage::GlobalTermPoolGuard;
use crate::storage::MAX_FIXED_ARITY;
use crate::storage::SendContainerProtectionSet;
use crate::storage::SharedTerm;
use crate::storage::SharedTermProtection;
use crate::storage::global_aterm_pool::GC_BUDGET_CHUNK;
use crate::storage::global_aterm_pool::GLOBAL_TERM_POOL;

thread_local! {
    /// Thread-specific [ThreadTermPool] that manages protection sets for the
    /// current thread.
    ///
    /// Deliberately not wrapped in a `RefCell`: term construction hands out
    /// `Return` values whose recursive read guard points into this pool, so it
    /// cannot be moved or replaced while a term is alive.
    pub static THREAD_TERM_POOL: ThreadTermPool = ThreadTermPool::new();
}

/// Per-thread term pool managing local protection sets for interaction with the global term pool.
pub struct ThreadTermPool {
    /// Contains all the protection sets for this thread.
    protection_sets: Arc<UnsafeCell<SharedTermProtection>>,

    /// A separate protection set for sendable terms, see [crate::ATermSend].
    send_term_protection_set: Arc<Mutex<ProtectionSet<ATermIndex>>>,

    /// A separate protection set for sendable containers, see [crate::ProtectedSend].
    send_container_protection_set: SendContainerProtectionSet,

    /// Counts down the number of terms this thread may still create before it must consume the
    /// next [GC_BUDGET_CHUNK] from the global budget (see [GlobalTermPool::reset_gc_budget]).
    ///
    /// Zero means this thread holds no reservation, so the next term creation has to claim one.
    garbage_collection_counter: Cell<usize>,

    /// A vector of terms that are used to store the arguments of a term for lookup.
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

        let (protection_sets, send_term_protection_set, send_container_protection_set) =
            pool.with_mut(|pool| pool.register_thread_term_pool());
        let int_symbol = pool.get_int_symbol().copy();
        let empty_list_symbol = pool.get_empty_list_symbol().copy();
        let list_symbol = pool.get_list_symbol().copy();

        drop(pool);

        Self {
            protection_sets,
            send_term_protection_set,
            send_container_protection_set,
            // Start without a reservation so the first created term claims (and charges) one.
            garbage_collection_counter: Cell::new(0),
            tmp_arguments: RefCell::new(Vec::new()),
            int_symbol,
            empty_list_symbol,
            list_symbol,
            term_pool,
        }
    }

    /// Creates a constant [ATerm] (arity 0) for the given symbol.
    pub fn create_constant<'a, 'b, S: Symb<'a, 'b>>(&self, symbol: &'b S) -> ATerm {
        assert!(symbol.arity() == 0, "A constant should not have arity > 0");

        let empty_args: [ATermRef<'_>; 0] = [];
        let guard = self.term_pool.read_recursive().expect("Lock poisoned!");

        let (index, inserted) = guard.create_term_array(symbol, &empty_args);
        let result = self.protect_guard(guard, &unsafe { ATermRef::from_index(&index) });

        if inserted {
            // Intentially called after the guard is dropped.
            self.decrement_garbage_collection_counter();
        }

        result
    }

    /// Create a term with the given arguments
    pub fn create_term<'a, 'b, S: Symb<'a, 'b>, T: Term<'a, 'b>>(
        &self,
        symbol: &'b S,
        args: &'b [T],
    ) -> Return<ATermRef<'static>> {
        // We cannot perform garbage collection afterwards since the guard is alive in the return.
        self.trigger_garbage_collection();

        let guard = self.term_pool.read_recursive().expect("Lock poisoned!");

        let (index, inserted) = if symbol.arity() <= MAX_FIXED_ARITY {
            // Fast path: build the fixed-arity key straight from the input slice, skipping
            // the `tmp_arguments` buffer round-trip.
            guard.create_term_fixed(symbol, args)
        } else {
            let mut arguments = self.tmp_arguments.borrow_mut();

            arguments.clear();
            for arg in args {
                unsafe {
                    arguments.push(ATermRef::from_index(arg.shared()));
                }
            }

            guard.create_term_array(symbol, &arguments)
        };

        let result = self.make_return(index, guard);

        if inserted {
            self.decrement_garbage_collection_counter();
        }

        result
    }

    /// Create a term with the given index.
    pub fn create_int(&self, value: usize) -> ATerm {
        let guard = self.term_pool.read_recursive().expect("Lock poisoned!");
        let (index, inserted) = guard.create_int(value);
        let result = self.protect_guard(guard, &unsafe { ATermRef::from_index(&index) });

        if inserted {
            // Intentially called after the guard is dropped.
            self.decrement_garbage_collection_counter();
        }

        result
    }

    /// Create a term with the given arguments given by the iterator.
    ///
    /// # Panics
    ///
    /// For symbols with arity above the fixed-arity limit the iterator is driven while an
    /// internal argument buffer is borrowed, so an iterator that itself constructs terms
    /// (e.g. through [ATerm::with_args] or [crate::ATerm::with_iter]) panics with a
    /// `RefCell` double borrow.
    pub fn create_term_iter<'a, 'b, 'c, 'd, S, I, T>(&self, symbol: &'b S, args: I) -> ATerm
    where
        S: Symb<'a, 'b>,
        I: IntoIterator<Item = T>,
        T: Term<'c, 'd>,
    {
        let guard = self.term_pool.read_recursive().expect("Lock poisoned!");

        let (index, inserted) = if symbol.arity() <= MAX_FIXED_ARITY {
            // Fast path: feed the argument indices straight into the fixed-arity storage,
            // skipping the `tmp_arguments` buffer round-trip.
            // SAFETY: the read guard blocks garbage collection, so every copied index stays
            // valid until the inserted term stores it; afterwards the GC marks the arguments
            // of live terms.
            guard.create_term_fixed_iter(symbol, args.into_iter().map(|arg| unsafe { arg.shared().copy() }))
        } else {
            let mut arguments = self.tmp_arguments.borrow_mut();
            arguments.clear();
            for arg in args {
                unsafe {
                    arguments.push(ATermRef::from_index(arg.shared()));
                }
            }

            guard.create_term_array(symbol, &arguments)
        };

        let result = self.protect_guard(guard, &unsafe { ATermRef::from_index(&index) });

        if inserted {
            // Intentially called after the guard is dropped.
            self.decrement_garbage_collection_counter();
        }

        result
    }

    /// Create a term with the given arguments given by the iterator that is fallible.
    ///
    /// # Panics
    ///
    /// The iterator is driven while an internal argument buffer is borrowed, so an
    /// iterator that itself constructs terms panics with a `RefCell` double borrow;
    /// see [Self::create_term_iter].
    pub fn try_create_term_iter<'a, 'b, 'c, 'd, S, I, T>(&self, symbol: &'b S, args: I) -> Result<ATerm, MercError>
    where
        S: Symb<'a, 'b>,
        I: IntoIterator<Item = Result<T, MercError>>,
        T: Term<'c, 'd>,
    {
        let guard = self.term_pool.read_recursive().expect("Lock poisoned!");
        let mut arguments = self.tmp_arguments.borrow_mut();
        arguments.clear();
        for arg in args {
            unsafe {
                arguments.push(ATermRef::from_index(arg?.shared()));
            }
        }

        let (index, inserted) = guard.create_term_array(symbol, &arguments);
        let result = Ok(self.protect_guard(guard, &unsafe { ATermRef::from_index(&index) }));

        if inserted {
            // Intentially called after the guard is dropped.
            self.decrement_garbage_collection_counter();
        }

        result
    }

    /// Create a term with the given arguments given by the iterator.
    ///
    /// # Panics
    ///
    /// For symbols with arity above the fixed-arity limit the iterator is driven while an
    /// internal argument buffer is borrowed, so an iterator that itself constructs terms
    /// panics with a `RefCell` double borrow; see [Self::create_term_iter].
    pub fn create_term_iter_head<'a, 'b, 'c, 'd, 'e, 'f, S, H, I, T>(
        &self,
        symbol: &'b S,
        head: &'d H,
        args: I,
    ) -> ATerm
    where
        S: Symb<'a, 'b>,
        H: Term<'c, 'd>,
        I: IntoIterator<Item = T>,
        T: Term<'e, 'f>,
    {
        let guard = self.term_pool.read_recursive().expect("Lock poisoned!");

        let (index, inserted) = if symbol.arity() <= MAX_FIXED_ARITY {
            // Fast path: feed the head and argument indices straight into the fixed-arity
            // storage, skipping the `tmp_arguments` buffer round-trip.
            // SAFETY: the read guard blocks garbage collection, so every copied index stays
            // valid until the inserted term stores it; afterwards the GC marks the arguments
            // of live terms.
            let head_index = unsafe { head.shared().copy() };
            guard.create_term_fixed_iter(
                symbol,
                iter::once(head_index).chain(args.into_iter().map(|arg| unsafe { arg.shared().copy() })),
            )
        } else {
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

            guard.create_term_array(symbol, &arguments)
        };

        let result = self.protect_guard(guard, &unsafe { ATermRef::from_index(&index) });

        if inserted {
            // Intentially called after the guard is dropped.
            self.decrement_garbage_collection_counter();
        }

        result
    }

    /// Create a function symbol
    pub fn create_symbol<N: Into<String> + AsRef<str>>(&self, name: N, arity: usize) -> Symbol {
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
        let root = self
            .lock_protection_set()
            .term_protection_set
            .protect(unsafe { term.shared().copy() });

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
    pub(crate) fn protect_guard(
        &self,
        _guard: RecursiveLockReadGuard<'_, GlobalTermPool>,
        term: &ATermRef<'_>,
    ) -> ATerm {
        // Protect the term by adding its index to the protection set
        // SAFETY: If the global term pool is locked, so we can safely access the protection set.
        // Copying the index is justified as in `protect`: `term` is alive for this call and
        // the protection set keeps it a GC root afterwards.
        let root = unsafe {
            (*self.protection_sets.get())
                .term_protection_set
                .protect(term.shared().copy())
        };

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
        // SAFETY: `term.root()` was returned by a matching `protect` and the
        // owning `ATerm` is dropped exactly once, so it is unprotected once.
        unsafe {
            self.lock_protection_set().term_protection_set.unprotect(term.root());
        }

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
        // SAFETY: `root` was returned by a matching `protect_container` and the
        // owning handle is dropped exactly once, so it is unprotected once.
        unsafe {
            self.lock_protection_set().container_protection_set.unprotect(root);
        }

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
        let mut lock = self.lock_protection_set();
        // Once inserted the protection set makes it a GC root until the
        // returned `Symbol` unprotects it.
        let root = lock.symbol_protection_set.protect(unsafe { symbol.shared().copy() });
        let result = unsafe { Symbol::from_index(symbol.shared(), root) };

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
        // SAFETY: `symbol.root()` was returned by a matching `protect_symbol`
        // and the owning `Symbol` is dropped exactly once, so it is
        // unprotected once.
        unsafe {
            self.lock_protection_set()
                .symbol_protection_set
                .unprotect(symbol.root());
        }
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
        guard.with_mut(|guard| guard.automatic_garbage_collection(enabled));
    }

    /// Forces a garbage collection to occur regardless of the current GC budget.
    pub fn force_collect_garbage(&self) {
        let mut guard = self.term_pool.write().expect("Lock poisoned!");
        guard.with_mut(|guard| {
            guard.collect_garbage();
            guard.reset_gc_budget();
        });

        // Reset the counter.
        self.garbage_collection_counter.set(GC_BUDGET_CHUNK);
    }

    /// Perform a garbage collection if the global aterm pool is not locked. Returns whether a
    /// collection actually ran; it is skipped when another thread already holds the write lock.
    pub fn collect_garbage(&self) {
        if let Some(mut guard) = self.term_pool.try_write().expect("Lock poisoned!") {
            guard.with_mut(|guard| guard.trigger_garbage_collection());

            // Reset the counter.
            self.garbage_collection_counter.set(GC_BUDGET_CHUNK);
        }
    }

    /// Triggers delayed garbage collection if the counter has reached zero.
    ///
    /// # Safety
    ///
    /// This function drops the passed guard.
    pub(crate) unsafe fn trigger_delayed_garbage_collection(&self, guard: &mut ManuallyDrop<GlobalTermPoolGuard<'_>>) {
        // Read the depth before dropping the guard; using `guard` after `ManuallyDrop::drop`
        // would violate its contract. The guard itself accounts for one level.
        debug_assert!(
            guard.read_depth() == 1,
            "Cannot trigger garbage collection while holding another read lock"
        );

        unsafe {
            ManuallyDrop::drop(guard);
        }
        if self.garbage_collection_counter.get() == 0 {
            self.trigger_garbage_collection();
        }
    }

    /// Counts a newly inserted term off the local counter, and triggers garbage collection if is exhausted.
    fn decrement_garbage_collection_counter(&self) {
        self.garbage_collection_counter
            .set(self.garbage_collection_counter.get().saturating_sub(1));

        self.trigger_garbage_collection();
    }

    /// Trigger garbage collection if the counter is exhausted and the global pool is not locked.
    fn trigger_garbage_collection(&self) {
        if self.garbage_collection_counter.get() != 0 || self.term_pool.is_locked() {
            return;
        }

        // Subtract a chunk from the shared budget.
        let previous = self
            .term_pool
            .read_recursive()
            .expect("Lock poisoned!")
            .consume_gc_budget(GC_BUDGET_CHUNK);

        if previous <= GC_BUDGET_CHUNK {
            // The global budget is exhausted, so collect
            self.collect_garbage();
        }

        // Count another chunk down locally.
        self.garbage_collection_counter.set(GC_BUDGET_CHUNK);
    }

    /// Returns a reference to the send term protection set.
    pub fn send_term_protection_set(&self) -> &Arc<Mutex<ProtectionSet<ATermIndex>>> {
        &self.send_term_protection_set
    }

    /// Returns a reference to the send container protection set, see [crate::ProtectedSend].
    pub fn send_container_protection_set(&self) -> &SendContainerProtectionSet {
        &self.send_container_protection_set
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
        unsafe { &mut *self.protection_sets.get() }
            .term_protection_set
            .replace(root, term);
    }

    /// Creates a Return for the given index and guard.
    fn make_return(
        &self,
        index: ATermIndex,
        guard: RecursiveLockReadGuard<'_, GlobalTermPool>,
    ) -> Return<ATermRef<'static>> {
        // SAFETY: The guard borrows from `term_pool`. Its implicit `Drop` never runs because
        // `Return` stores it in a `ManuallyDrop` and releases it through `THREAD_TERM_POOL`,
        // which panics if the thread-local pool is gone instead of dereferencing a dangling
        // lock. `Return` is also `!Send`, so it never crosses to another thread.
        unsafe {
            Return::new(
                std::mem::transmute::<RecursiveLockReadGuard<'_, _>, RecursiveLockReadGuard<'static, _>>(guard),
                ATermRef::from_index(&index),
            )
        }
    }

    /// The protection set is locked by the global read-write lock
    fn lock_protection_set(&self) -> ProtectionSetGuard<'_> {
        let guard = self.term_pool.read_recursive().expect("Lock poisoned!");
        let protection_set = unsafe { &mut *self.protection_sets.get() };

        ProtectionSetGuard::new(guard, protection_set)
    }
}

impl Drop for ThreadTermPool {
    fn drop(&mut self) {
        let mut write = self.term_pool.write().expect("Lock poisoned!");
        // SAFETY: `ThreadTermPool` is being dropped and we hold the global write lock,
        // so this protection set cannot be mutated concurrently.
        let protection = unsafe { &*self.protection_sets.get() };

        // In the normal case all thread-local terms, symbols, and containers have been dropped
        // by the time the thread exits, so these sets are empty. Any that remain are reachable
        // from state that outlives this thread (for example a `static` or another thread-local,
        // such as the default data symbols in `merc_data`). Adopt them as global orphan roots so
        // that later read-only inspection cannot dereference reclaimed memory. The orphan sets
        // deduplicate, so repeatedly leaking the same shared roots does not grow memory unboundedly.
        write.with_mut(|write| write.protect_orphan_roots(protection));

        debug!("{}", write.metrics());
        write.with_mut(|write| write.deregister_thread_pool(protection.index));

        debug!("{}", unsafe { &mut *self.protection_sets.get() }.metrics());
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
    use std::sync::mpsc;

    use crate::ATerm;
    use crate::ATermRef;
    use crate::Symb;
    use crate::Symbol;
    use crate::Term;
    use crate::storage::THREAD_TERM_POOL;

    use std::thread;

    #[test]
    fn test_thread_local_protection() {
        merc_utilities::test_logger();

        thread::scope(|scope| {
            for _ in 0..3 {
                scope.spawn(|| {
                    // Create and protect some terms
                    let symbol = Symbol::new("test", 0);
                    let term = ATerm::constant(&symbol);
                    let protected = term.protect();

                    // Verify protection
                    THREAD_TERM_POOL.with(|tp| {
                        assert!(
                            tp.lock_protection_set()
                                .term_protection_set
                                .contains_root(protected.root())
                        );
                    });

                    // Unprotect
                    let root = protected.root();
                    drop(protected);

                    THREAD_TERM_POOL.with(|tp| {
                        assert!(!tp.lock_protection_set().term_protection_set.contains_root(root));
                    });
                });
            }
        });
    }

    #[test]
    fn test_parsing() {
        merc_utilities::test_logger();

        let t = ATerm::from_string("f(g(a),b)").unwrap();

        assert!(t.get_head_symbol().name() == "f");
        assert!(t.arg(0).get_head_symbol().name() == "g");
        assert!(t.arg(1).get_head_symbol().name() == "b");
    }

    #[test]
    fn test_create_term() {
        merc_utilities::test_logger();

        let f = Symbol::new("f", 2);
        let g = Symbol::new("g", 1);

        let t = THREAD_TERM_POOL.with(|tp| {
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

    #[test]
    fn test_orphaned_thread_term_remains_readable_after_gc() {
        merc_utilities::test_logger();

        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let term =
                ATerm::with_args(&Symbol::new("f_tls", 1), &[ATerm::constant(&Symbol::new("a_tls", 0))]).protect();

            // SAFETY: the term index is copied while protected and only used for read-only
            // inspection in this regression test.
            tx.send(unsafe { term.shared().copy() })
                .expect("Channel send should succeed");

            // Leak the term so it is still protected at thread teardown, exercising the orphan
            // adoption path. The bug we guard against is post-teardown read UB.
            std::mem::forget(term);
        });

        handle.join().expect("Thread should join without panic");
        let orphan_index = rx.recv().expect("Channel receive should succeed");

        for _ in 0..10 {
            THREAD_TERM_POOL.with(|tp| tp.force_collect_garbage());
        }

        // SAFETY: the index was adopted into the global orphan set during `ThreadTermPool::drop`,
        // so read-only inspection remains valid.
        let orphan = unsafe { ATermRef::from_index(&orphan_index) };
        assert_eq!(orphan.get_head_symbol().name(), "f_tls");
        assert_eq!(orphan.arg(0).get_head_symbol().name(), "a_tls");
    }
}
