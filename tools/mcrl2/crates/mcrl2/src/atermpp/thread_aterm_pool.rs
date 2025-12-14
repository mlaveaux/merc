use core::fmt;
use std::borrow::Borrow;
use std::cell::RefCell;
use std::mem::ManuallyDrop;
use std::sync::Arc;

use log::trace;

use mcrl2_sys::atermpp::ffi;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_create;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_from_string;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_pool_collect_garbage;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_pool_print_metrics;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_pool_register_mark_callback;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_pool_test_garbage_collection;
use mcrl2_sys::atermpp::ffi::mcrl2_function_symbol_create;
use mcrl2_sys::cxx::Exception;
use mcrl2_sys::cxx::UniquePtr;
use merc_utilities::ProtectionIndex;
use merc_utilities::ProtectionSet;

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
use super::global_aterm_pool::mark_protection_sets;
use super::global_aterm_pool::protection_set_size;

/// The number of times before garbage collection is tested again.
const TEST_GC_INTERVAL: usize = 100;

thread_local! {
    /// This is the thread specific term pool that manages the protection sets.
    pub(crate) static THREAD_TERM_POOL: RefCell<ThreadTermPool> = RefCell::new(ThreadTermPool::new());
}

pub struct ThreadTermPool {
    protection_set: SharedProtectionSet,
    container_protection_set: SharedContainerProtectionSet,

    /// The index of the thread term pool in the list of thread pools.
    index: usize,

    /// Function symbols to represent 'DataAppl' with any number of arguments.
    data_appl: Vec<Symbol>,

    /// A counter used to periodically trigger a garbage collection test.
    /// This is a stupid hack, but we need to periodically test for garbage collection and this is only allowed outside of a shared
    /// lock section. Therefore, we count (arbitrarily) to reduce the amount of this is checked.
    gc_counter: usize,

    _callback: ManuallyDrop<UniquePtr<ffi::tls_callback_container>>,
}

/// Protects the given aterm address and returns the term.
///     - guard: An existing guard to the ThreadTermPool.protection_set.
///     - index: The index of the ThreadTermPool
fn protect_with(
    mut guard: BfTermPoolThreadWrite<'_, ProtectionSet<ATermPtr>>,
    gc_counter: &mut usize,
    index: usize,
    term: *const ffi::_aterm,
) -> ATerm {
    debug_assert!(!term.is_null(), "Can only protect valid terms");
    let aterm = ATermPtr::new(term);
    let root = guard.protect(aterm.clone());

    let term = ATermRef::new(term);
    trace!("Protected term {:?}, index {}, protection set {}", term, root, index,);

    let result = ATerm::new(term, root);

    // Test for garbage collection intermediately.
    *gc_counter = gc_counter.saturating_sub(1);

    if guard.unlock() && *gc_counter == 0 {
        mcrl2_aterm_pool_test_garbage_collection();
        *gc_counter = TEST_GC_INTERVAL;
    }

    result
}

impl ThreadTermPool {
    pub fn new() -> ThreadTermPool {
        // Register a protection set into the global set.
        let (protection_set, container_protection_set, index) = GLOBAL_TERM_POOL.lock().register_thread_term_pool();

        ThreadTermPool {
            protection_set,
            container_protection_set,
            index,
            gc_counter: TEST_GC_INTERVAL,
            data_appl: vec![],
            _callback: ManuallyDrop::new(mcrl2_aterm_pool_register_mark_callback(
                mark_protection_sets,
                protection_set_size,
            )),
        }
    }

    /// Protects the given aterm address and returns the term.
    pub fn protect(&mut self, term: *const ffi::_aterm) -> ATerm {
        unsafe {
            protect_with(
                self.protection_set.write_exclusive(),
                &mut self.gc_counter,
                self.index,
                term,
            )
        }
    }

    /// Protects the given aterm address and returns the term.
    pub fn protect_container(&mut self, container: Arc<dyn Markable + Send + Sync>) -> ProtectionIndex {
        let root = unsafe { self.container_protection_set.write_exclusive().protect(container) };

        trace!("Protected container index {}, protection set {}", root, self.index,);

        root
    }

    /// Removes the [ATerm] from the protection set.
    pub fn drop(&mut self, term: &ATerm) {
        term.require_valid();

        unsafe {
            let mut protection_set = self.protection_set.write_exclusive();
            trace!(
                "Dropped term {:?}, index {}, protection set {}",
                term.term, term.root, self.index
            );
            protection_set.unprotect(term.root);
        }
    }

    /// Removes the container from the protection set.
    pub fn drop_container(&mut self, container_root: ProtectionIndex) {
        unsafe {
            let mut container_protection_set = self.container_protection_set.write_exclusive();
            trace!(
                "Dropped container index {}, protection set {}",
                container_root, self.index
            );
            container_protection_set.unprotect(container_root);
        }
    }

    /// Returns true iff the given term is a data application.
    pub fn is_data_application(&mut self, term: &ATermRef<'_>) -> bool {
        let symbol = term.get_head_symbol();
        // It can be that data_applications are created without create_data_application in the mcrl2 ffi.
        while self.data_appl.len() <= symbol.arity() {
            let symbol = Symbol::take(mcrl2_function_symbol_create(
                String::from("DataAppl"),
                self.data_appl.len(),
            ));
            self.data_appl.push(symbol);
        }

        symbol == self.data_appl[symbol.arity()].copy()
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
            self.protection_set.read().len() == 0,
            "The protection set should be empty"
        );

        GLOBAL_TERM_POOL.lock().drop_thread_term_pool(self.index);

        #[cfg(not(target_os = "macos"))]
        unsafe {
            ManuallyDrop::drop(&mut self._callback);
        }
    }
}

/// This is the thread local term pool.
pub struct TermPool {
    arguments: Vec<*const ffi::_aterm>,
}

impl TermPool {
    pub fn new() -> TermPool {
        TermPool { arguments: vec![] }
    }

    /// Trigger a garbage collection explicitly.
    pub fn collect(&mut self) {
        mcrl2_aterm_pool_collect_garbage();
    }

    /// Creates an ATerm from a string.
    pub fn from_string(&mut self, text: &str) -> Result<ATerm, Exception> {
        match mcrl2_aterm_from_string(String::from(text)) {
            Ok(term) => Ok(ATerm::from_unique_ptr(term)),
            Err(exception) => Err(exception),
        }
    }

    /// Creates an [ATerm] with the given symbol and arguments.
    pub fn create<'a, 'b>(
        &mut self,
        symbol: &impl Borrow<SymbolRef<'a>>,
        arguments: &[impl Borrow<ATermRef<'b>>],
    ) -> ATerm {
        // Copy the arguments to make a slice.
        self.arguments.clear();
        for arg in arguments {
            self.arguments.push(arg.borrow().get());
        }

        debug_assert_eq!(
            symbol.borrow().arity(),
            self.arguments.len(),
            "Number of arguments does not match arity"
        );

        let result = THREAD_TERM_POOL.with_borrow_mut(|tp| {
            unsafe {
                // ThreadPool is not Sync, so only one has access.
                let protection_set = tp.protection_set.write_exclusive();
                let term: *const ffi::_aterm = mcrl2_aterm_create(symbol.borrow().get(), &self.arguments);
                protect_with(protection_set, &mut tp.gc_counter, tp.index, term)
            }
        });

        result
    }

    /// Creates an [ATerm] with the given symbol, head argument and other arguments.
    pub fn create_data_application<'a, 'b>(
        &mut self,
        head: &impl Borrow<ATermRef<'a>>,
        arguments: &[impl Borrow<ATermRef<'b>>],
    ) -> ATerm {
        // Make the temp vector sufficient length.
        while self.arguments.len() < arguments.len() {
            self.arguments.push(std::ptr::null());
        }

        self.arguments.clear();
        self.arguments.push(head.borrow().get());
        for arg in arguments {
            self.arguments.push(arg.borrow().get());
        }

        THREAD_TERM_POOL.with_borrow_mut(|tp| {
            while tp.data_appl.len() <= arguments.len() + 1 {
                let symbol = self.create_symbol("DataAppl", tp.data_appl.len());
                tp.data_appl.push(symbol);
            }

            let symbol = &tp.data_appl[arguments.len() + 1];

            debug_assert_eq!(
                symbol.arity(),
                self.arguments.len(),
                "Number of arguments does not match arity"
            );

            let result = unsafe {
                // ThreadPool is not Sync, so only one has access.
                let protection_set = tp.protection_set.write_exclusive();
                let term: *const ffi::_aterm = mcrl2_aterm_create(symbol.get(), &self.arguments);
                protect_with(protection_set, &mut tp.gc_counter, tp.index, term)
            };

            result
        })
    }

    /// Creates a function symbol with the given name and arity.
    pub fn create_symbol(&mut self, name: &str, arity: usize) -> Symbol {
        Symbol::take(mcrl2_function_symbol_create(String::from(name), arity))
    }

    /// Creates a term with the FFI while taking care of the protection and garbage collection.
    pub fn create_with<F>(&mut self, create: F) -> ATerm
    where
        F: Fn() -> *const ffi::_aterm,
    {
        THREAD_TERM_POOL.with_borrow_mut(|tp| {
            unsafe {
                // ThreadPool is not Sync, so only one has access.
                let protection_set = tp.protection_set.write_exclusive();
                protect_with(protection_set, &mut tp.gc_counter, tp.index, create())
            }
        })
    }
}

impl Default for TermPool {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TermPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO: This will always print, but only depends on aterm_configuration.h
        mcrl2_aterm_pool_print_metrics();

        write!(f, "{:?}", GLOBAL_TERM_POOL.lock())
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use rand::Rng;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    use crate::random_term;

    use super::*;

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

    #[test]
    fn test_thread_aterm_pool_parallel() {
        let seed: u64 = rand::rng().random();
        println!("seed: {}", seed);

        thread::scope(|s| {
            for _ in 0..2 {
                s.spawn(|| {
                    let mut tp = TermPool::new();

                    let mut rng = StdRng::seed_from_u64(seed);
                    let terms: Vec<ATerm> = (0..100)
                        .map(|_| {
                            random_term(
                                &mut tp,
                                &mut rng,
                                &[("f".to_string(), 2)],
                                &["a".to_string(), "b".to_string()],
                                10,
                            )
                        })
                        .collect();

                    tp.collect();

                    for term in &terms {
                        verify_term(term);
                    }
                });
            }
        });
    }
}
