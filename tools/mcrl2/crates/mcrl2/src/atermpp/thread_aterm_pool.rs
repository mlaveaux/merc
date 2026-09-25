use core::fmt;
use std::borrow::Borrow;
use std::cell::Cell;
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use log::debug;
use log::trace;

use mcrl2_sys::atermpp::ffi;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_create;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_create_int;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_empty_list_function_symbol;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_from_string;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_list_function_symbol;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_pool_collect_garbage;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_pool_print_metrics;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_pool_resize;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_pool_resize_is_needed;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_pool_size;
use mcrl2_sys::atermpp::ffi::mcrl2_function_symbol_create;
use mcrl2_sys::cxx::Exception;

use merc_unsafety::ProtectionIndex;
use merc_unsafety::ProtectionSet;
use merc_utilities::debug_trace;

use crate::ATerm;
use crate::ATermRef;
use crate::atermpp::BfTermPoolThreadWrite;
use crate::atermpp::Symbol;

use super::Markable;
use super::SymbolRef;
use super::global_aterm_pool::ATermPtr;
use super::global_aterm_pool::GLOBAL_TERM_POOL;
use super::global_aterm_pool::SharedContainerProtectionSet;
use super::global_aterm_pool::SharedProtectionSet;
use super::global_aterm_pool::num_thread_pools;
use super::global_aterm_pool::register_mark_callback;

/// The number of terms that the pool must contain before garbage collection is
/// considered at all, since collecting a small pool is not worth its cost.
const MIN_TERMS_UNTIL_GC: usize = 1_000_000;

/// The smallest number of terms a thread creates before consulting the shared
/// budget again, which bounds how much the threads contend on it.
const MIN_GC_CHUNK: usize = 100;

/// The largest such number. Bounded because the same interval also governs how
/// long a hash table resize can be postponed, see [`ThreadTermPool::protect_with`].
const MAX_GC_CHUNK: usize = 10_000;

/// The pool size at which the next garbage collection should be triggered.
///
/// This threshold is global rather than per thread, since a collection performed
/// by one thread reclaims the garbage of all of them. With a per thread threshold
/// the collecting thread lowers only its own, after which every other thread still
/// exceeds its stale threshold and collects the very same (already collected) pool
/// again in turn.
static SIZE_UNTIL_GC: AtomicUsize = AtomicUsize::new(MIN_TERMS_UNTIL_GC);

/// The number of terms a single thread may create before it compares the pool size
/// against [`SIZE_UNTIL_GC`] again, i.e. this thread's share of the budget that is
/// left until the next collection.
static GC_CHUNK: AtomicUsize = AtomicUsize::new(MIN_GC_CHUNK);

/// Set while some thread is performing a garbage collection, so that the other
/// threads skip theirs instead of queueing up behind it.
static GC_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Recomputes the global garbage collection budget from the current pool size,
/// which is done after every collection.
fn reset_gc_budget() {
    let size = mcrl2_aterm_pool_size();

    // Collect again once the pool has roughly doubled.
    //
    // Note that the capacity cannot be used for this, since it sums the capacities
    // of the storages for every arity while a term only occupies one of them. The
    // pool size therefore stays well below the capacity and never reaches it.
    let until = size.saturating_mul(2).max(MIN_TERMS_UNTIL_GC);
    SIZE_UNTIL_GC.store(until, Ordering::Relaxed);

    // Divide the remaining headroom over the registered threads. Every thread
    // counting down a fixed interval instead would make the threads together
    // check (and thereby contend on the pool size) a factor `threads` too often.
    let chunk = until.saturating_sub(size) / num_thread_pools();
    GC_CHUNK.store(chunk.clamp(MIN_GC_CHUNK, MAX_GC_CHUNK), Ordering::Relaxed);
}

thread_local! {
    /// This is the thread specific term pool that manages the protection sets.
    pub(crate) static THREAD_TERM_POOL: RefCell<ThreadTermPool> = RefCell::new(ThreadTermPool::new());
}

pub(crate) struct ThreadTermPool {
    protection_set: SharedProtectionSet,
    container_protection_set: SharedContainerProtectionSet,

    /// The index of the thread term pool in the list of thread pools.
    index: usize,

    /// Function symbol for non-empty list constructors.
    list_symbol: Symbol,

    /// Function symbol for the empty list.
    empty_list_symbol: Symbol,

    /// Function symbols to represent 'DataAppl' with any number of arguments.
    data_appl: RefCell<Vec<Symbol>>,

    /// Counts down this thread's share of the budget until the next garbage
    /// collection, see [`GC_CHUNK`]. Testing for garbage collection is only allowed
    /// outside of a shared lock section, and counting keeps the test itself off the
    /// hot path.
    gc_counter: Cell<usize>,

    /// Temporary storage for arguments when creating terms.
    arguments: RefCell<Vec<*const ffi::_aterm>>,
}

impl ThreadTermPool {
    pub fn new() -> ThreadTermPool {
        // Register a protection set into the global set. The lock must be released
        // again before registering the mark callback below, see
        // `register_mark_callback`.
        let (protection_set, container_protection_set, index) = GLOBAL_TERM_POOL.lock().register_thread_term_pool();

        // Only the first thread actually registers, the callback is global.
        register_mark_callback();

        ThreadTermPool {
            protection_set,
            container_protection_set,
            index,
            // SAFETY: the FFI returns the live built-in list / empty-list function symbols.
            list_symbol: unsafe { Symbol::from_ptr(mcrl2_aterm_list_function_symbol()) },
            empty_list_symbol: unsafe { Symbol::from_ptr(mcrl2_aterm_empty_list_function_symbol()) },
            gc_counter: Cell::new(GC_CHUNK.load(Ordering::Relaxed)),
            data_appl: RefCell::new(vec![]),
            arguments: RefCell::new(vec![]),
        }
    }

    /// Trigger a garbage collection explicitly.
    ///
    /// Note that `mcrl2_aterm_pool_collect_garbage` enables garbage collection for
    /// the duration of the call, since the pool ignores every collection—including
    /// explicit ones—while it is disabled (see `aterm_pool::collect_impl`). It is
    /// disabled in the global term pool to keep the pool from collecting on its own
    /// from inside a shared section.
    pub fn collect(&self) {
        debug!("Collecting mCRL2 aterm pool garbage");

        mcrl2_aterm_pool_collect_garbage();

        // Garbage collection was performed, so we can reset the budget.
        reset_gc_budget();
    }

    /// Performs a garbage collection, unless another thread is already collecting
    /// or has collected since this thread observed that the pool exceeded
    /// [`SIZE_UNTIL_GC`].
    fn collect_if_needed(&self) {
        // Claim the collection. Threads that lose this race skip their collection
        // entirely: the one that is running reclaims their garbage as well.
        if GC_IN_PROGRESS
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return;
        }

        // Acquiring the claim synchronises with the release below, so the threshold
        // read here is the one stored by the previous collection. That collection
        // may well have brought the pool back under it.
        if mcrl2_aterm_pool_size() >= SIZE_UNTIL_GC.load(Ordering::Relaxed) {
            self.collect();
        }

        GC_IN_PROGRESS.store(false, Ordering::Release);
    }

    /// Creates an ATerm from a string.
    pub fn from_string(&self, text: &str) -> Result<ATerm, Exception> {
        match mcrl2_aterm_from_string(text) {
            Ok(term) => Ok(ATerm::from_unique_ptr(term)),
            Err(exception) => Err(exception),
        }
    }

    /// Creates an [ATerm] with the given symbol and arguments.
    pub fn create<'a, 'b, S, T>(&self, symbol: &S, arguments: &[T]) -> ATerm
    where
        S: Borrow<SymbolRef<'a>>,
        T: Borrow<ATermRef<'b>>,
    {
        // Copy the arguments to make a slice.
        let mut tmp_args = self.arguments.borrow_mut();
        tmp_args.clear();
        for arg in arguments {
            tmp_args.push(arg.borrow().get());
        }

        debug_assert_eq!(
            symbol.borrow().arity(),
            tmp_args.len(),
            "Number of arguments does not match arity"
        );

        unsafe {
            // ThreadPool is not Sync, so only one has access.
            let protection_set = self.protection_set.write_exclusive();
            let term: *const ffi::_aterm = mcrl2_aterm_create(symbol.borrow().get(), &tmp_args);
            self.protect_with(protection_set, term)
        }
    }

    /// Creates an [ATerm] with the given symbol, head argument and other arguments.
    pub fn create_data_application<'a, 'b, S, T>(&self, head: &S, arguments: &[T]) -> ATerm
    where
        S: Borrow<ATermRef<'a>>,
        T: Borrow<ATermRef<'b>>,
    {
        let mut tmp_args = self.arguments.borrow_mut();
        tmp_args.clear();
        tmp_args.push(head.borrow().get());
        for arg in arguments {
            tmp_args.push(arg.borrow().get());
        }

        let mut tmp_data_appl = self.data_appl.borrow_mut();
        while tmp_data_appl.len() <= arguments.len() + 1 {
            let symbol = self.create_symbol("DataAppl", tmp_data_appl.len());
            tmp_data_appl.push(symbol);
        }

        let symbol = &tmp_data_appl[arguments.len() + 1];

        debug_assert_eq!(
            symbol.arity(),
            tmp_args.len(),
            "Number of arguments does not match arity"
        );

        unsafe {
            // ThreadPool is not Sync, so only one has access.
            let protection_set = self.protection_set.write_exclusive();
            let term: *const ffi::_aterm = mcrl2_aterm_create(symbol.get(), &tmp_args);
            self.protect_with(protection_set, term)
        }
    }

    /// Creates an aterm_int from the given value.
    pub fn create_int(&self, value: u64) -> ATerm {
        unsafe {
            // ThreadPool is not Sync, so only one has access.
            let protection_set = self.protection_set.write_exclusive();
            let term: *const ffi::_aterm = mcrl2_aterm_create_int(value);
            self.protect_with(protection_set, term)
        }
    }

    /// Creates a function symbol with the given name and arity.
    pub fn create_symbol(&self, name: &str, arity: usize) -> Symbol {
        Symbol::take(mcrl2_function_symbol_create(String::from(name), arity))
    }

    /// Returns the function symbol for non-empty list constructors.
    pub fn list_symbol(&self) -> SymbolRef<'_> {
        self.list_symbol.copy()
    }

    /// Returns the function symbol for the empty list.
    pub fn empty_list_symbol(&self) -> SymbolRef<'_> {
        self.empty_list_symbol.copy()
    }

    /// Creates a term with the FFI while taking care of the protection and garbage collection.
    pub fn create_with<F>(&self, create: F) -> ATerm
    where
        F: Fn() -> *const ffi::_aterm,
    {
        unsafe {
            // ThreadPool is not Sync, so only one has access.
            let protection_set = self.protection_set.write_exclusive();
            self.protect_with(protection_set, create())
        }
    }

    /// Protects the given aterm address and returns the term.
    pub fn protect(&self, term: *const ffi::_aterm) -> ATerm {
        unsafe { self.protect_with(self.protection_set.write_exclusive(), term) }
    }

    /// Protects the given aterm address and returns the term.
    pub fn protect_container(&self, container: Arc<dyn Markable + Send + Sync>) -> ProtectionIndex {
        let root = unsafe { self.container_protection_set.write_exclusive().protect(container) };

        debug_trace!("Protected container index {}, protection set {}", root, self.index,);

        root
    }

    /// Removes the [ATerm] from the protection set.
    pub fn drop_term(&self, term: &ATerm) {
        term.require_valid();

        unsafe {
            let mut protection_set = self.protection_set.write_exclusive();
            debug_trace!(
                "Dropped term {:?}, index {}, protection set {}",
                term.term,
                term.root,
                self.index
            );
            // SAFETY: `term.root` was returned by a matching `protect` and the
            // owning `ATerm` is dropped exactly once, so it is unprotected once.
            protection_set.unprotect(term.root);
        }
    }

    /// Removes the container from the protection set.
    pub fn drop_container(&self, container_root: ProtectionIndex) {
        unsafe {
            let mut container_protection_set = self.container_protection_set.write_exclusive();
            debug_trace!(
                "Dropped container index {}, protection set {}",
                container_root,
                self.index
            );
            // SAFETY: `container_root` was returned by a matching `protect` and
            // the owning handle is dropped exactly once, so it is unprotected once.
            container_protection_set.unprotect(container_root);
        }
    }

    /// Returns true iff the given term is a data application.
    pub fn is_data_application(&self, term: &ATermRef<'_>) -> bool {
        let symbol = term.get_head_symbol();
        // Data applications can be created without using create_data_application in the mcrl2 FFI.
        let mut data_appl = self.data_appl.borrow_mut();
        while data_appl.len() <= symbol.arity() {
            let new_symbol = self.create_symbol("DataAppl", data_appl.len());
            data_appl.push(new_symbol);
        }

        symbol == data_appl[symbol.arity()].copy()
    }

    /// Protects the given aterm address and returns the term.
    ///     - guard: An existing guard to the ThreadTermPool.protection_set.
    fn protect_with(
        &self,
        mut guard: BfTermPoolThreadWrite<'_, ProtectionSet<ATermPtr>>,
        term: *const ffi::_aterm,
    ) -> ATerm {
        debug_assert!(!term.is_null(), "Can only protect valid terms");
        let aterm = ATermPtr::new(term);
        let root = guard.protect(aterm.clone());

        // SAFETY: the term was just protected on the protection set above
        // (`root`), so it stays live as long as the resulting `ATerm` holds that
        // root, which justifies the `'static` lifetime.
        let term = unsafe { ATermRef::new(term) };
        debug_trace!(
            "Protected term {:?}, index {}, protection set {}",
            term,
            root,
            self.index
        );

        let result = ATerm::from_ref(term, root);

        // Test for garbage collection and hash table resizing intermediately.
        let counter = self.gc_counter.get().saturating_sub(1);
        self.gc_counter.set(counter);

        // `guard.unlock()` returns true only when this leaves the outermost
        // shared section, i.e. the thread is no longer busy.
        if guard.unlock() && counter == 0 {
            self.gc_counter.set(GC_CHUNK.load(Ordering::Relaxed));

            // If garbage collection is necessary according to our requirements.
            if mcrl2_aterm_pool_size() >= SIZE_UNTIL_GC.load(Ordering::Relaxed) {
                self.collect_if_needed();
            }

            // Only take the exclusive lock when a storage actually has to grow.
            // Resizing unconditionally suspends every other thread (the exclusive
            // lock waits for all of them to leave their shared sections) once per
            // chunk per thread, which stalls exploration completely.
            if mcrl2_aterm_pool_resize_is_needed() {
                mcrl2_aterm_pool_resize();
            }
        }

        result
    }
}

impl Default for ThreadTermPool {
    fn default() -> Self {
        ThreadTermPool::new()
    }
}

impl Drop for ThreadTermPool {
    fn drop(&mut self) {
        debug_assert!(
            self.protection_set.read().is_empty(),
            "The protection set should be empty"
        );

        GLOBAL_TERM_POOL.lock().drop_thread_term_pool(self.index);
    }
}

impl fmt::Display for ThreadTermPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Note: This will always print the global term pool metrics, only depending on the aterm_configuration.h.
        mcrl2_aterm_pool_print_metrics();

        write!(f, "{:?}", GLOBAL_TERM_POOL.lock())
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use rand::RngExt;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    use super::super::random_term::random_term;
    use super::THREAD_TERM_POOL;
    use super::mcrl2_aterm_pool_size;
    use crate::ATerm;
    use crate::ATermRef;

    /// Make sure that the term has the same number of arguments as its arity.
    fn verify_term(term: &ATermRef<'_>) {
        for subterm in term.iter() {
            assert_eq!(
                subterm.get_head_symbol().arity(),
                subterm.arguments().len(),
                "The arity matches the number of arguments."
            )
        }
    }

    /// Garbage collection is disabled in the global term pool, which also makes
    /// the mCRL2 pool ignore explicitly requested collections. Check that an
    /// explicit collect actually removes the unprotected terms.
    #[test]
    fn test_collect_removes_unprotected_terms() {
        let mut rng = rand::rng();

        {
            let _terms: Vec<ATerm> = (0..1000)
                .map(|_| {
                    random_term(
                        &mut rng,
                        &[("f".to_string(), 2)],
                        &["a".to_string(), "b".to_string()],
                        10,
                    )
                })
                .collect();
        }

        let before = mcrl2_aterm_pool_size();
        THREAD_TERM_POOL.with_borrow(|tp| tp.collect());
        let after = mcrl2_aterm_pool_size();

        assert!(
            after < before,
            "collecting garbage did not remove any of the {before} terms in the pool"
        );
    }

    /// `BfTermPool::write_exclusive` documents that it is sound "given that
    /// other threads use [Self::write] and [Self::read] exclusively" against
    /// this same pool, i.e. it relies on no *other* thread calling `.read()`
    /// on the very `SharedProtectionSet` a thread is mutating through
    /// `write_exclusive`. But `write_exclusive` only takes the C++
    /// *shared* (busy) lock, the same lock class `.read()` takes, so the two
    /// are mutually compatible under the busy/forbidden protocol and do not
    /// exclude each other.
    ///
    /// `ThreadTermPool`'s `Display` impl (via `GlobalTermPool`'s `Debug`)
    /// calls `.read()` on *every* thread's protection set, including ones
    /// concurrently being mutated by their owning thread through
    /// `write_exclusive` (e.g. from `protect_with` while creating terms).
    /// That is exactly the forbidden interleaving: one thread observes
    /// `&ProtectionSet<ATermPtr>` (via `.read()`) while another thread holds
    /// `&mut ProtectionSet<ATermPtr>` (via `write_exclusive`) on the very same
    /// object, which is undefined behaviour (aliased mutable/shared access)
    /// and a genuine data race on the underlying `Vec` if it reallocates
    /// mid-iteration.
    ///
    /// This test drives that interleaving directly: one thread continuously
    /// creates terms (mutating its own protection set through
    /// `write_exclusive`) while another repeatedly formats the pool (reading
    /// every thread's protection set through `.read()`), and expects it not
    /// to crash / corrupt memory. On the reviewed code this is unsound even
    /// though it is not exercised anywhere else in the crate today; run it
    /// under a thread sanitizer (`cargo +nightly xtask thread-sanitizer test
    /// -p mcrl2 -- read_races_with_concurrent_term_creation`) for conclusive
    /// evidence, since a plain debug build may not reproduce the race on
    /// every run.
    #[test]
    fn read_races_with_concurrent_term_creation() {
        use std::sync::Arc;
        use std::sync::atomic::AtomicBool;
        use std::sync::atomic::Ordering;

        let stop = Arc::new(AtomicBool::new(false));

        thread::scope(|s| {
            // Several threads continuously creating terms, which mutates
            // their own `SharedProtectionSet` through `write_exclusive`
            // (a merely-*shared* C++ lock).
            for _ in 0..4 {
                let stop = stop.clone();
                s.spawn(move || {
                    let mut rng = rand::rng();
                    while !stop.load(Ordering::Relaxed) {
                        let _term = random_term(
                            &mut rng,
                            &[("f".to_string(), 2)],
                            &["a".to_string(), "b".to_string()],
                            50,
                        );
                    }
                });
            }

            // Concurrently format the pool, which calls `.read()` on every
            // thread's protection set -- including the ones the threads
            // above are mutating right now.
            for _ in 0..2000 {
                THREAD_TERM_POOL.with_borrow(|tp| {
                    let _ = format!("{tp}");
                });
            }

            stop.store(true, Ordering::Relaxed);
        });
    }

    #[test]
    fn test_thread_aterm_pool_parallel() {
        let mut rng = rand::rng();
        let seed: u64 = rng.random();
        println!("seed: {}", seed);

        thread::scope(|s| {
            for _ in 0..2 {
                s.spawn(|| {
                    let mut rng = StdRng::seed_from_u64(seed);
                    let terms: Vec<ATerm> = (0..100)
                        .map(|_| {
                            random_term(
                                &mut rng,
                                &[("f".to_string(), 2)],
                                &["a".to_string(), "b".to_string()],
                                10,
                            )
                        })
                        .collect();

                    // Force garbage collection.
                    THREAD_TERM_POOL.with_borrow(|tp| tp.collect());

                    for term in &terms {
                        verify_term(term);
                    }
                });
            }
        });
    }
}
