#![allow(non_camel_case_types)]

use std::ffi::CStr;
use std::ffi::c_char;
use std::mem;
use std::ops::Deref;
use std::ptr;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use merc_aterm::ATermIndex;
use merc_aterm::ATermInt;
use merc_aterm::ATermList;
use merc_aterm::ATermRef;
use merc_aterm::SharedSymbol;
use merc_aterm::SharedTerm;
use merc_aterm::Symb;
use merc_aterm::Symbol;
use merc_aterm::SymbolIndex;
use merc_aterm::SymbolRef;
use merc_aterm::THREAD_TERM_POOL;
use merc_aterm::Term;
use merc_aterm::TermOrAnnotation;
use merc_aterm::is_empty_list_term;
use merc_aterm::is_int_term;
use merc_aterm::is_list_term;

/// The is the underlying shared aterm that is pointed to by the term.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct unprotected_aterm_t {
    ptr: *const std::ffi::c_void,
}

/// This keeps track of the root index for a term.
#[repr(C)]
pub struct root_index_t {
    index: usize,
}

/// This is a pair that is used as return value for some functions.
#[repr(C)]
pub struct aterm_t {
    term: unprotected_aterm_t,
    root: root_index_t,
}

/// The pointer to a shared function symbol, and the root index for its protection.
#[repr(C)]
pub struct function_symbol_t {
    ptr: *const std::ffi::c_void,
    root: root_index_t,
}

/// Returns true iff the term is an integer term.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn term_is_int(term: unprotected_aterm_t) -> bool {
    unsafe { is_int_term(&term_to_aterm_ref(term, true)) }
}

/// Returns true iff the term is a list term.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn term_is_list(term: unprotected_aterm_t) -> bool {
    unsafe { is_list_term(&term_to_aterm_ref(term, false)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn term_empty_list() -> unprotected_aterm_t {
    let empty_list = ATermList::<()>::empty();

    unprotected_aterm_t {
        ptr: empty_list.shared().deref() as *const SharedTerm as *const std::ffi::c_void,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn term_is_empty_list(term: unprotected_aterm_t) -> bool {
    unsafe { is_empty_list_term(&term_to_aterm_ref(term, false)) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn term_is_defined(term: unprotected_aterm_t) -> bool {
    !term.ptr.is_null()
}

/// Creates a new integer term with the given value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn term_create_int(value: usize) -> aterm_t {
    let term = ATermInt::new(value);

    let term_ptr = term.shared().deref() as *const SharedTerm as *const std::ffi::c_void;
    let root = *term.root().deref();

    std::mem::forget(term); // Prevent the term from being dropped

    aterm_t {
        term: unprotected_aterm_t { ptr: term_ptr },
        root: root_index_t { index: root },
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn term_get_int_value(term: unprotected_aterm_t) -> usize {
    unsafe {
        let shared_term = term_to_aterm_ref(term, true);
        debug_assert!(shared_term.annotation().is_some(), "Term is not an integer term");
        shared_term.annotation().unwrap_unchecked()
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn term_protect(term: unprotected_aterm_t) -> root_index_t {
    THREAD_TERM_POOL.with_borrow(|tp| {
        let term = unsafe { tp.protect(&term_to_aterm_ref(term, false)) };
        let root = term.root();
        std::mem::forget(term); // Prevent the term from being dropped
        root_index_t { index: *root.deref() }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn term_unprotect(_root: root_index_t) {
    unimplemented!();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn term_get_argument(_term: unprotected_aterm_t, _index: usize) -> unprotected_aterm_t {
    unimplemented!();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn term_create_appl(
    _symbol: function_symbol_t,
    _arguments: *const unprotected_aterm_t,
    _num_arguments: usize,
) -> aterm_t {
    unimplemented!();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn function_symbol_register_prefix(
    _prefix: *const c_char,
    _length: usize,
) -> prefix_shared_counter_t {
    unimplemented!();
    // let result = GLOBAL_TERM_POOL.write().expect("Lock poisoned!").register_prefix(
    //     unsafe { CStr::from_ptr(prefix).to_str().expect("Invalid UTF-8 in prefix") },
    // );

    // prefix_shared_counter_t {
    //     ptr: Arc::into_raw(result) as *const std::ffi::c_void,
    // }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn function_symbol_deregister_prefix(_prefix: *const std::ffi::c_char, _length: usize) {
    unimplemented!();
    // GLOBAL_TERM_POOL.write().expect("Lock poisoned!").remove_prefix(
    //     unsafe { CStr::from_ptr(prefix).to_str().expect("Invalid UTF-8 in prefix") },
    // );
}

/// Returns true iff the given function symbol is an integer symbol.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn function_symbol_is_int(symbol: function_symbol_t) -> bool {
    unsafe {
        THREAD_TERM_POOL.with_borrow(|tp| {
            let symbol_ref = function_to_symbol_ref(symbol);
            *tp.int_symbol() == symbol_ref
        })
    }
}

/// This is a counter that is used to keep track of the number of references to
/// a prefix.
///
/// This is used because Arc is not available in the FFI, so we use a raw
/// pointer to the counter (which is stable because it is an Arc).
#[repr(C)]
pub struct prefix_shared_counter_t {
    ptr: *const std::ffi::c_void,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn shared_counter_value(counter: prefix_shared_counter_t) -> usize {
    unsafe {
        counter
            .ptr
            .cast::<AtomicUsize>()
            .as_ref()
            .expect("Counter pointer is not null")
            .load(Ordering::Relaxed)
    }
}

/// Increases the reference count of the shared counter.
///
/// # Safety
///
/// The given counter must be a valid pointer that has not been released.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn shared_counter_add_ref(counter: prefix_shared_counter_t) {
    unsafe {
        // Clone the Arc to increment the reference count, but forgot it to avoid dropping it.
        let result = Arc::from_raw(counter.ptr.cast::<AtomicUsize>());
        mem::forget(result.clone());
        mem::forget(result);
    }
}

/// Decreases the reference count of the shared counter.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn shared_counter_unref(counter: prefix_shared_counter_t) {
    unsafe {
        // Construct the Arc and drop it to decrement the reference count.
        Arc::from_raw(counter.ptr.cast::<AtomicUsize>());
    }
}

/// Returns the function symbol of the given term.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn term_get_function_symbol(term: unprotected_aterm_t) -> function_symbol_t {
    unsafe {
        function_symbol_t {
            ptr: term_to_aterm_ref(term, false).shared().symbol().shared().deref() as *const SharedSymbol
                as *const std::ffi::c_void,
            root: root_index_t { index: 0 },
        }
    }
}

/// Creates a new function symbol with the given name and arity.
///
/// If check_for_registered_functions is true, it will check if the function symbol is already registered.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn function_symbol_create(
    name: *const std::ffi::c_char,
    _length: usize,
    arity: usize,
    _check_for_registered_functions: bool,
) -> function_symbol_t {
    let symbol = Symbol::new(
        unsafe { CStr::from_ptr(name).to_str().expect("Invalid UTF-8 in symbol name") },
        arity,
    );

    let symbol_ref = symbol.shared().deref() as *const SharedSymbol as *const std::ffi::c_void;
    let index = *symbol.root();
    std::mem::forget(symbol); // Prevent the symbol from being dropped
    function_symbol_t {
        ptr: symbol_ref,
        root: root_index_t { index },
    }
}

/// Protects a function symbol, returning a root index that can be used to unprotect it later.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn function_symbol_protect(symbol: function_symbol_t) -> root_index_t {
    THREAD_TERM_POOL.with_borrow(|tp| {
        let symbol_ref = unsafe { function_to_symbol_ref(symbol) };
        let protected_symbol = tp.protect_symbol(&symbol_ref);
        let root = protected_symbol.root();
        std::mem::forget(protected_symbol); // Prevent the symbol from being dropped
        root_index_t { index: *root.deref() }
    })
}

/// Removes the protection of a function symbol.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn function_symbol_unprotect(_root: root_index_t) {
    unimplemented!();
    // THREAD_TERM_POOL.with_borrow(|tp| {
    //     tp.unprotect_symbol(&SymbolRef::from_index(&SymbolIndex::from_ptr(NonNull::new_unchecked(root.index as *mut SharedSymbol))));
    // });
}

type term_deletion_hook_t = extern "C" fn(symbol: unprotected_aterm_t);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn register_deletion_hook(_symbol: &function_symbol_t, _deletion_hook: term_deletion_hook_t) {
    unimplemented!();
    // GLOBAL_TERM_POOL.write().register_deletion_hook(|term| {
    //     deletion_hook(&unprotected_aterm_t {
    //         ptr: term.shared().deref() as *const SharedTerm as *const std::ffi::c_void,
    //     });
    // });
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn function_symbol_get_arity(symbol: function_symbol_t) -> usize {
    unsafe {
        let symbol = function_to_symbol_ref(symbol);
        symbol.arity()
    }
}

#[repr(C)]
pub struct string_view_t {
    ptr: *const std::ffi::c_char,
    length: usize,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn function_symbol_get_name(symbol: function_symbol_t) -> string_view_t {
    unsafe {
        let symbol = function_to_symbol_ref(symbol);
        string_view_t {
            ptr: symbol.name().as_ptr() as *const std::ffi::c_char,
            length: symbol.name().len(),
        }
    }
}

// A dummy protection set that is used to protect a FFI container.
// struct ProtectedContainer {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn container_protect() -> root_index_t {
    unimplemented!();
    // THREAD_TERM_POOL.with_borrow(|tp| {
    //     let root = tp.protect_container();
    //     root_index_t { index: *root.deref() }
    // })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn container_unprotect(_root: root_index_t) {
    unimplemented!();
    // THREAD_TERM_POOL.with_borrow(|tp| {
    //     let root = tp.protect_container();
    //     root_index_t { index: *root.deref() }
    // })
}

/// Locks the global term pool for shared access.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn global_lock_shared() {
    // Forget the guard to prevent it from being dropped.
    unimplemented!();
    // mem::forget(GLOBAL_TERM_POOL.read_recursive());
}

/// Unlocks the global term pool after shared access.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn global_unlock_shared() {
    unimplemented!();
    // unsafe { GLOBAL_TERM_POOL.force_unlock_read() };
}

/// Locks the global term pool for exclusive access.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn global_lock_exclusive() {
    // Forget the guard to prevent it from being dropped.
    unimplemented!();
    // mem::forget(GLOBAL_TERM_POOL.write());
}

/// Unlocks the global term pool after exclusive access.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn global_unlock_exclusive() {
    unimplemented!();
    // unsafe { GLOBAL_TERM_POOL.force_unlock_write() };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn term_pool_is_busy_set() -> bool {
    unimplemented!();
    // GLOBAL_TERM_POOL.is_locked()
}

/// Can be used during garbage collection to mark a term (and all of its subterms) as being reachable.
///
/// # Safety
///
/// This function should only be called during garbage collection when the global term pool is locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn term_mark(_term: unprotected_aterm_t) {
    unimplemented!();
    // unsafe {
    //     GLOBAL_TERM_POOL
    //         .make_write_guard_unchecked()
    //         .mark_term(&term_to_aterm_ref(term));
    // }
}

/// Returns the number of arguments in the term.
unsafe fn term_len(term: unprotected_aterm_t) -> usize {
    // Assuming the pointer is to a SharedTerm, we can get the length from the SharedTerm.
    unsafe {
        let symbol: SymbolRef = ptr::read(term.ptr.cast());
        symbol.arity() // Assuming arity gives the length of the term
    }
}

/// Converts a raw pointer to an `ATermRef`, must ensure that the raw ptr is valid.
///
/// Safety: The unprotected_aterm_t must point to a valid term.
unsafe fn term_to_aterm_ref(term: unprotected_aterm_t, annotated: bool) -> ATermRef<'static> {
    unsafe {
        let wide_ptr =
            ptr::slice_from_raw_parts(term.ptr as *const TermOrAnnotation, term_len(term) + annotated as usize);
        ATermRef::from_index(&ATermIndex::from_ptr(NonNull::new_unchecked(
            wide_ptr as *mut SharedTerm,
        )))
    }
}

/// Converts a raw pointer to an `SymbolRef`, must ensure that the raw ptr is valid.
///
/// Safety: The unprotected_aterm_t must point to a valid term.
unsafe fn function_to_symbol_ref(symbol: function_symbol_t) -> SymbolRef<'static> {
    unsafe {
        SymbolRef::from_index(&SymbolIndex::from_ptr(NonNull::new_unchecked(
            symbol.ptr as *mut SharedSymbol,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_term_create_int() {
        unsafe {
            let aterm = term_create_int(42);
            assert!(term_is_int(aterm.term));
            let value = term_get_int_value(aterm.term);
            assert_eq!(value, 42);
        }
    }
}
