use core::fmt;
use std::borrow::Borrow;
use std::cell::Cell;
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
    data_appl: RefCell<Vec<Symbol>>,

    /// A counter used to periodically trigger a garbage collection test.
    /// This is a stupid hack, but we need to periodically test for garbage collection and this is only allowed outside of a shared
    /// lock section. Therefore, we count (arbitrarily) to reduce the amount of this is checked.
    gc_counter: Cell<usize>,

    /// Temporary storage for arguments when creating terms.
    arguments: RefCell<Vec<*const ffi::_aterm>>,

    _callback: ManuallyDrop<UniquePtr<ffi::tls_callback_container>>,
}

impl ThreadTermPool {
    pub fn new() -> ThreadTermPool {
        // Register a protection set into the global set.
        let (protection_set, container_protection_set, index) = GLOBAL_TERM_POOL.lock().register_thread_term_pool();

        ThreadTermPool {
            protection_set,
            container_protection_set,
            index,
            gc_counter: Cell::new(TEST_GC_INTERVAL),
            data_appl: RefCell::new(vec![]),
            arguments: RefCell::new(vec![]),
            _callback: ManuallyDrop::new(mcrl2_aterm_pool_register_mark_callback(
                mark_protection_sets,
                protection_set_size,
            )),
        }
        }
    

    /// Trigger a garbage collection explicitly.
    pub fn collect(&self) {
        mcrl2_aterm_pool_collect_garbage();
    }

    /// Creates an ATerm from a string.
    pub fn from_string(&self, text: &str) -> Result<ATerm, Exception> {
        match mcrl2_aterm_from_string(text) {
            Ok(term) => Ok(ATerm::from_unique_ptr(term)),
            Err(exception) => Err(exception),
        }
    }

    /// Creates an [ATerm] with the given symbol and arguments.
    pub fn create<'a, 'b>(
        &self,
        symbol: &impl Borrow<SymbolRef<'a>>,
        arguments: &[impl Borrow<ATermRef<'b>>],
    ) -> ATerm {
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
    pub fn create_data_application<'a, 'b>(
        &self,
        head: &impl Borrow<ATermRef<'a>>,
        arguments: &[impl Borrow<ATermRef<'b>>],
    ) -> ATerm {
        // Make the temp vector sufficient length.
        let mut tmp_args = self.arguments.borrow_mut();
        while tmp_args.len() < arguments.len() {
            tmp_args.push(std::ptr::null());
        }

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

    /// Creates a function symbol with the given name and arity.
    pub fn create_symbol(&self, name: &str, arity: usize) -> Symbol {
        Symbol::take(mcrl2_function_symbol_create(String::from(name), arity))
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
        unsafe {
            self.protect_with(
                self.protection_set.write_exclusive(),
                term,
            )
        }
    }

    /// Protects the given aterm address and returns the term.
    pub fn protect_container(&self, container: Arc<dyn Markable + Send + Sync>) -> ProtectionIndex {
        let root = unsafe { self.container_protection_set.write_exclusive().protect(container) };

        trace!("Protected container index {}, protection set {}", root, self.index,);

        root
    }

    /// Removes the [ATerm] from the protection set.
    pub fn drop_term(&self, term: &ATerm) {
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
    pub fn drop_container(&self, container_root: ProtectionIndex) {
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
    pub fn is_data_application(&self, term: &ATermRef<'_>) -> bool {
        let symbol = term.get_head_symbol();
        // It can be that data_applications are created without create_data_application in the mcrl2 ffi.
        let mut data_appl = self.data_appl.borrow_mut();
        while data_appl.len() <= symbol.arity() {
            let symbol = Symbol::take(mcrl2_function_symbol_create(
                String::from("DataAppl"),
                data_appl.len(),
            ));
            data_appl.push(symbol);
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

        let term = ATermRef::new(term);
        trace!("Protected term {:?}, index {}, protection set {}", term, root, self.index);

        let result = ATerm::from_ref(term, root);

        // Test for garbage collection intermediately.
        let counter = self.gc_counter.get().saturating_sub(1);
        self.gc_counter.set(counter);

        if guard.unlock() && counter == 0 {
            mcrl2_aterm_pool_test_garbage_collection();
            self.gc_counter.set(TEST_GC_INTERVAL);
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

        #[cfg(not(target_os = "macos"))]
        unsafe {
            ManuallyDrop::drop(&mut self._callback);
        }
    }
}

impl fmt::Display for ThreadTermPool {
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
                    THREAD_TERM_POOL.with_borrow(|tp|
                        tp.collect()
                    );

                    for term in &terms {
                        verify_term(term);
                    }
                });
            }
        });
    }
}
