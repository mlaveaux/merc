use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::ptr::NonNull;
use std::ptr::slice_from_raw_parts_mut;

use merc_aterm::ATermIndex;
use merc_aterm::ATermRef;
use merc_aterm::Symb;
use merc_aterm::SymbolRef;
use merc_aterm::Term;
use merc_aterm::storage::SharedTerm;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_unsafety::SliceDst;
use merc_unsafety::StablePointer;

use crate::DataExpressionFFI;
use crate::DataExpressionRefFFI;
use crate::SabreRewriteVTable;

/// Wraps a raw shared term node address as a transient [`ATermIndex`].
///
/// # Safety
///
/// `node` must be the address of a live `SharedTerm` owned by the global term
/// pool. The returned index is a non-owning view: it must not outlive the node.
unsafe fn node_to_index(node: *const c_void) -> ATermIndex {
    unsafe {
        // `SharedTerm` is a slice DST, so its pointer is wide. Recover the
        // slice length from the symbol header stored inline at offset 0,
        // reading it by reference so the symbol's debug reference counter is
        // untouched (equivalent to what `Erasable::unerase` does).
        let symbol: &SymbolRef<'static> = &*(node as *const SymbolRef<'static>);
        let len = symbol.arity();

        let slice = NonNull::new_unchecked(slice_from_raw_parts_mut(node as *mut (), len));
        StablePointer::from_ptr(<SharedTerm as SliceDst>::retype(slice))
    }
}

/// Returns the shared term node address backing a term reference.
fn node_of<'a, T: Term<'a, 'a>>(term: &T) -> *const c_void {
    // SAFETY: `term: &T` is a live borrow, so the `SharedTerm` it points into is live for the
    // duration of this call.
    unsafe { term.shared().ptr() }.as_ptr() as *const c_void
}

/// Wraps an owned [`DataExpression`] as a [`DataExpressionFFI`] handle.
fn into_ffi(expr: DataExpression) -> DataExpressionFFI {
    let node = node_of(&expr);
    DataExpressionFFI {
        node,
        handle: Box::into_raw(Box::new(expr)) as *mut c_void,
    }
}

unsafe extern "C-unwind" fn host_arity(node: *const c_void) -> usize {
    unsafe {
        let index = node_to_index(node);
        let term = ATermRef::from_index(&index);
        DataExpressionRef::from(term).data_arguments().len()
    }
}

unsafe extern "C-unwind" fn host_data_arg(node: *const c_void, index: usize) -> *const c_void {
    unsafe {
        let term_index = node_to_index(node);
        let term = ATermRef::from_index(&term_index);
        node_of(&DataExpressionRef::from(term).data_arg(index))
    }
}

unsafe extern "C-unwind" fn host_data_function_symbol(node: *const c_void) -> *const c_void {
    unsafe {
        let index = node_to_index(node);
        let term = ATermRef::from_index(&index);
        node_of(&DataExpressionRef::from(term).data_function_symbol())
    }
}

unsafe extern "C-unwind" fn host_operation_id(node: *const c_void) -> usize {
    unsafe {
        let index = node_to_index(node);
        ATermRef::from_index(&index).index()
    }
}

unsafe extern "C-unwind" fn host_create(
    head: *const c_void,
    args: *const DataExpressionRefFFI<'static>,
    len: usize,
) -> DataExpressionFFI {
    unsafe {
        let head_index = node_to_index(head);
        let head_term = ATermRef::from_index(&head_index);

        if len == 0 {
            // A constant data expression is the function symbol itself.
            return into_ffi(DataExpressionRef::from(head_term).protect());
        }

        // Reconstruct argument references. The index wrappers must outlive the
        // ATermRefs derived from them, so keep them in a separate vector.
        let arg_refs = std::slice::from_raw_parts(args, len);
        let arg_indices: Vec<ATermIndex> = arg_refs.iter().map(|a| node_to_index(a.shared())).collect();
        let arg_terms: Vec<ATermRef<'_>> = arg_indices.iter().map(|i| ATermRef::from_index(i)).collect();

        let application = DataApplication::with_args(&head_term, &arg_terms);
        into_ffi(application.into())
    }
}

unsafe extern "C-unwind" fn host_protect(node: *const c_void) -> DataExpressionFFI {
    unsafe {
        let index = node_to_index(node);
        let term = ATermRef::from_index(&index);
        into_ffi(DataExpressionRef::from(term).protect())
    }
}

unsafe extern "C-unwind" fn host_destroy(handle: *mut c_void) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle as *mut DataExpression) });
    }
}

/// Builds the vtable of host operations. These functions are the only place
/// where the global term pool is touched; they run in the host's address
/// space using the host's `merc_aterm` globals regardless of which crate's
/// code invokes them through the vtable.
pub fn rewrite_vtable() -> SabreRewriteVTable {
    SabreRewriteVTable {
        arity: host_arity,
        data_arg: host_data_arg,
        data_function_symbol: host_data_function_symbol,
        operation_id: host_operation_id,
        create: host_create,
        protect: host_protect,
        destroy: host_destroy,
    }
}

/// Builds a borrowed reference handle for a term owned by the host.
pub fn data_expression_ref_from_term<'a, T: Term<'a, 'a>>(term: &T) -> DataExpressionRefFFI<'a> {
    DataExpressionRefFFI::from_raw(node_of(term))
}

/// Consumes a handle returned across the boundary and recovers the owned
/// [`DataExpression`], transferring its protection to the caller.
///
/// # Safety
///
/// `expr` must be an owned handle produced by [`rewrite_vtable`]'s `create`,
/// `protect`, or the generated `rewrite` entry point, and must not be used
/// afterwards.
pub unsafe fn into_data_expression(expr: DataExpressionFFI) -> DataExpression {
    // Avoid the `Drop` impl, which would `destroy` the handle we are taking.
    let expr = ManuallyDrop::new(expr);
    *unsafe { Box::from_raw(expr.handle as *mut DataExpression) }
}
