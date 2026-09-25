use std::borrow::Borrow;
use std::cmp::Ordering;
use std::collections::VecDeque;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::marker::PhantomData;
use std::ops::Deref;

use mcrl2_sys::atermpp::ffi;
use mcrl2_sys::atermpp::ffi::_aterm;
use mcrl2_sys::atermpp::ffi::aterm;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_get_address;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_get_argument;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_get_function_symbol;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_is_empty_list;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_is_int;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_is_list;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_print;
use mcrl2_sys::cxx::Exception;
use mcrl2_sys::cxx::UniquePtr;
use merc_unsafety::ProtectionIndex;
use merc_utilities::PhantomUnsend;

use super::THREAD_TERM_POOL;
use crate::atermpp::SymbolRef;

use super::global_aterm_pool::ATermPtr;
use super::global_aterm_pool::SEND_PROTECTION_SET;

/// This represents a lifetime bound reference to an existing ATerm that is
/// protected somewhere statically.
///
/// Can be 'static if the term is protected in a container or ATerm. That means
/// we either return &'a ATermRef<'static> or with a concrete lifetime
/// ATermRef<'a>. However, this means that the functions for ATermRef cannot use
/// the associated lifetime for the results parameters, as that would allow us
/// to acquire the 'static lifetime. This occasionally gives rise to issues
/// where we look at the argument of a term and want to return it's name, but
/// this is not allowed since the temporary returned by the argument is dropped.
///
/// Note that since terms are stored in thread local storage, we can not store
/// any [ATermRef] or [ATerm] in a thread local storage ourselves, as that would
/// lead to unsoundness. The destruction order of thread local storage is not
/// defined, so we might drop a term pool before dropping the terms stored in
/// it.
#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ATermRef<'a> {
    term: *const ffi::_aterm,
    marker: PhantomData<&'a ()>,
}

// SAFETY: `ATermRef<'a>` is `Copy` and carries only a raw `*const _aterm`
// plus a `PhantomData<&'a ()>` lifetime witness; it owns no protection by
// itself (that is held elsewhere, by whichever `ATerm`/container the
// lifetime `'a` is borrowed from). The pointee is immutable from Rust's
// perspective — mutation only happens as relaxed-atomic bookkeeping during
// garbage collection, which every thread observes consistently because GC
// requires the C++ exclusive lock (no thread may be "busy" while it runs).
// Reading the same `*const _aterm` concurrently from multiple threads (via
// `Send`) or through a shared `&ATermRef` (via `Sync`) therefore performs no
// racing writes. Moving/sharing an `ATermRef<'a>` across threads cannot
// extend its validity past `'a`: `'a` is a compile-time bound the type
// carries with it (e.g. via `std::thread::scope`), independent of which
// thread evaluates it.
unsafe impl Send for ATermRef<'_> {}
unsafe impl Sync for ATermRef<'_> {}

impl Default for ATermRef<'_> {
    fn default() -> Self {
        ATermRef {
            term: std::ptr::null(),
            marker: PhantomData,
        }
    }
}

impl<'a> ATermRef<'a> {
    /// Protects the reference on the thread local protection pool.
    pub fn protect(&self) -> ATerm {
        if self.is_default() {
            ATerm::default()
        } else {
            THREAD_TERM_POOL.with_borrow(|tp| tp.protect(self.term))
        }
    }

    /// This allows us to extend our borrowed lifetime from 'a to 'b based on
    /// existing parent term which has lifetime 'b.
    ///
    /// The main usecase is to establish transitive lifetimes. For example given
    /// a term t from which we borrow `u = t.arg(0)` then we cannot have
    /// u.arg(0) live as long as t since the intermediate temporary u is
    /// dropped. However, since we know that u.arg(0) is a subterm of `t` we can
    /// upgrade its lifetime to the lifetime of `t` using this function.
    ///
    /// # Safety
    ///
    /// Requires: `*self` and `*parent` name the same maximally-shared node,
    /// or `*self` is reachable from `*parent` by following zero or more
    /// `arg(i)` steps (i.e. `*self` is `*parent` or one of its transitive
    /// subterms) — checked by the `debug_assert!` above in debug builds,
    /// trusted in release builds. Since garbage collection marks a live
    /// term's whole subtree from its root, this means: whatever keeps
    /// `*parent` alive for `'b` (a protected `ATerm`'s root, a `Markable`
    /// container, or a transitively-live parent up the chain) also keeps
    /// `*self` alive for `'b`. Guarantees: the returned `ATermRef<'b>` is
    /// valid to hold and dereference for the whole of `'b` under that same
    /// condition.
    pub unsafe fn upgrade<'b: 'a>(&'a self, parent: &ATermRef<'b>) -> ATermRef<'b> {
        debug_assert!(
            parent.iter().any(|t| t.copy() == *self),
            "Upgrade has been used on a witness that is not a parent term"
        );

        // SAFETY: the caller guarantees the current term is a subterm of
        // `parent`, which is live for `'b`, so the term is live for `'b` too.
        unsafe { ATermRef::new(self.term) }
    }

    /// A private unchecked version of [`ATermRef::upgrade`] to use in iterators.
    ///
    /// # Safety
    ///
    /// Same precondition as [`ATermRef::upgrade`] (`*self` reachable from
    /// `*_parent` via zero or more `arg(i)` steps), but unchecked even in
    /// debug builds — every call site must justify it inline rather than
    /// relying on a `debug_assert!` here.
    unsafe fn upgrade_unchecked<'b: 'a>(&'a self, _parent: &ATermRef<'b>) -> ATermRef<'b> {
        // SAFETY: callers guarantee `_parent` is a parent term, see `upgrade`.
        unsafe { ATermRef::new(self.term) }
    }

    /// Obtains the underlying pointer
    pub(crate) fn get(&self) -> &ffi::_aterm {
        self.require_valid();
        // SAFETY: holding an `ATermRef` witnesses that the underlying term is
        // live (protected somewhere), so the pointer is valid to dereference.
        unsafe { self.term.as_ref().expect("The pointer should be defined") }
    }
}

impl<'a> ATermRef<'a> {
    /// Creates a reference to the maximally shared term at `term` with an
    /// arbitrary, caller-chosen lifetime `'a`.
    ///
    /// # Safety
    ///
    /// Requires: `term` is non-null (or the caller only ever calls
    /// `is_default`/nothing else on the result) and, if non-null, names a
    /// maximally-shared node that is reachable from some GC root — a
    /// protected `ATerm`'s address, the global send set, or a `Markable`
    /// container's contents — for the *entire* span of the caller-chosen
    /// `'a`. Choosing `'a` longer than that root's actual protected lifetime
    /// is unsound: a later garbage collection may free `term` while the
    /// `ATermRef<'a>` is still considered live by the type system, giving a
    /// dangling pointer that `get()`/`arg()`/... will dereference.
    /// Guarantees: on success, `ATermRef { term, .. }` is safe to copy,
    /// compare and dereference (via `get()`) for all of `'a`, provided the
    /// precondition holds.
    pub(crate) unsafe fn new(term: *const ffi::_aterm) -> ATermRef<'a> {
        ATermRef {
            term,
            marker: PhantomData,
        }
    }

    /// Returns the raw maximally shared term address underlying this reference.
    pub fn address(&self) -> *const ffi::_aterm {
        self.term
    }
}

impl ATermRef<'_> {
    /// Returns the indexed argument of the term.
    pub fn arg(&self, index: usize) -> ATermRef<'_> {
        self.require_valid();
        assert!(
            index < self.get_head_symbol().arity(),
            "arg({index}) is not defined for term {:?}",
            self
        );

        ATermRef {
            term: mcrl2_aterm_get_argument(self.get(), index),
            marker: PhantomData,
        }
    }

    /// Returns the list of arguments as a collection
    pub fn arguments(&self) -> ATermArgs<'_> {
        self.require_valid();

        ATermArgs::new(self.copy())
    }

    /// Makes a copy of the term with the same lifetime as itself.
    pub fn copy(&self) -> ATermRef<'_> {
        // SAFETY: the returned reference is bounded by the borrow of `self`,
        // which already witnesses that the term is live, so the lifetime cannot
        // outlive the term.
        unsafe { ATermRef::new(self.term) }
    }

    /// Returns whether the term is the default term (not initialised)
    pub fn is_default(&self) -> bool {
        self.term.is_null()
    }

    /// Returns true iff this is an aterm_list
    pub fn is_list(&self) -> bool {
        mcrl2_aterm_is_list(self.get())
    }

    /// Returns true iff this is the empty aterm_list
    pub fn is_empty_list(&self) -> bool {
        mcrl2_aterm_is_empty_list(self.get())
    }

    /// Returns true iff this is an aterm_int
    pub fn is_int(&self) -> bool {
        mcrl2_aterm_is_int(self.get())
    }

    /// Returns the head function symbol of the term.
    pub fn get_head_symbol(&self) -> SymbolRef<'_> {
        mcrl2_aterm_get_function_symbol(self.get()).into()
    }

    /// Returns an iterator over all arguments of the term that runs in pre order traversal of the term trees.
    pub fn iter(&self) -> TermIterator<'_> {
        TermIterator::new(self.copy())
    }

    /// Panics if the term is default
    pub fn require_valid(&self) {
        debug_assert!(
            !self.is_default(),
            "This function can only be called on valid terms, i.e., not default terms"
        );
    }
}

impl fmt::Display for ATermRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.require_valid();
        write!(f, "{:?}", self)
    }
}

impl fmt::Debug for ATermRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_default() {
            write!(f, "None")?;
        } else {
            write!(f, "{}", mcrl2_aterm_print(self.get()))?;
        }

        Ok(())
    }
}

/// The protected version of [ATermRef], mostly derived from it.
#[derive(Default)]
pub struct ATerm {
    pub(crate) term: ATermRef<'static>,
    pub(crate) root: ProtectionIndex,

    // ATerm is not Send because it uses thread-local state for its protection
    // mechanism.
    _marker: PhantomUnsend,
}

impl ATerm {
    /// Creates a new ATerm with the given symbol and arguments.
    pub fn with_args<'a, 'b, S, T>(symbol: &S, arguments: &[T]) -> ATerm
    where
        S: Borrow<SymbolRef<'a>>,
        T: Borrow<ATermRef<'b>>,
    {
        THREAD_TERM_POOL.with_borrow(|tp| tp.create(symbol, arguments))
    }

    /// Creates a constant ATerm with the given symbol.
    pub fn constant<'a, S: Borrow<SymbolRef<'a>>>(symbol: &S) -> ATerm {
        let tmp: &[ATermRef<'a>] = &[];
        THREAD_TERM_POOL.with_borrow(|tp| tp.create(symbol, tmp))
    }

    /// Constructs an ATerm from a string by parsing it.
    pub fn from_string(s: &str) -> Result<ATerm, Exception> {
        THREAD_TERM_POOL.with_borrow(|tp| tp.from_string(s))
    }

    /// Constructs an ATerm from a `UniquePtr<aterm>`. Note that we still do the
    /// protection here, so the term is copied into the thread local term pool.
    pub(crate) fn from_unique_ptr(term: UniquePtr<aterm>) -> Self {
        debug_assert!(!term.is_null(), "Cannot create ATerm from null unique ptr");
        THREAD_TERM_POOL.with_borrow(|tp| tp.protect(mcrl2_aterm_get_address(term.as_ref().expect("Pointer is valid"))))
    }

    /// Creates an ATerm from a raw pointer. It will be protected on creation.
    ///
    /// # Safety
    ///
    /// Requires: `term` is non-null and, at the point of the call, names a
    /// maximally-shared node that is currently reachable from some live GC
    /// root (so a garbage collection triggered by *this* call cannot free it
    /// first). No requirement on `term` staying live afterwards: this
    /// function immediately registers it in the calling thread's own
    /// protection set (`ThreadTermPool::protect`) before returning, and a
    /// term in a thread's own protection set is always marked live by
    /// `GlobalTermPool::mark_protection_sets`, regardless of what else
    /// referenced it. Guarantees: the returned `ATerm` keeps `term` alive
    /// (via that protection-set root) until the `ATerm` is dropped.
    pub unsafe fn from_ptr(term: *const ffi::_aterm) -> Self {
        debug_assert!(!term.is_null(), "Cannot create ATerm from null ptr");
        THREAD_TERM_POOL.with_borrow(|tp| tp.protect(term))
    }

    /// Obtains the underlying pointer
    pub fn get(&self) -> &_aterm {
        self.term.get()
    }

    /// Creates a new term from the given reference and protection set root
    /// entry.
    pub(crate) fn from_ref(term: ATermRef<'static>, root: ProtectionIndex) -> ATerm {
        ATerm {
            term,
            root,
            _marker: PhantomData,
        }
    }

    /// Returns the address of the underlying aterm
    pub fn address(&self) -> *const ffi::_aterm {
        self.term.term
    }
}

impl Drop for ATerm {
    fn drop(&mut self) {
        if !self.is_default() {
            THREAD_TERM_POOL.with_borrow(|tp| {
                tp.drop_term(self);
            })
        }
    }
}

impl Clone for ATerm {
    fn clone(&self) -> Self {
        self.copy().protect()
    }
}

impl Deref for ATerm {
    type Target = ATermRef<'static>;

    fn deref(&self) -> &Self::Target {
        &self.term
    }
}

impl<'a> Borrow<ATermRef<'a>> for ATerm {
    fn borrow(&self) -> &ATermRef<'a> {
        &self.term
    }
}

impl fmt::Display for ATerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.copy())
    }
}

impl fmt::Debug for ATerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.copy())
    }
}

impl Hash for ATerm {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.term.hash(state)
    }
}

impl PartialEq for ATerm {
    fn eq(&self, other: &Self) -> bool {
        self.term.eq(&other.term)
    }
}

impl PartialOrd for ATerm {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ATerm {
    fn cmp(&self, other: &Self) -> Ordering {
        self.term.cmp(&other.term)
    }
}

impl Eq for ATerm {}

/// A garbage-collection-protected term that is [`Send`] and [`Sync`].
///
/// Unlike [`ATerm`], whose protection root lives in a *thread-local* set, an
/// `ATermSend` registers its term in the single global protection set.
pub struct ATermSend {
    term: ATermPtr,
    root: ProtectionIndex,
}

impl ATermSend {
    /// Protects `term` in the global send protection set.
    fn protect(term: *const ffi::_aterm) -> ATermSend {
        debug_assert!(!term.is_null(), "Can only protect valid terms");
        let root = SEND_PROTECTION_SET.lock().protect(ATermPtr::new(term));
        ATermSend {
            term: ATermPtr::new(term),
            root,
        }
    }

    /// Creates an `ATermSend` protecting the maximally shared term at `term`.
    ///
    /// # Safety
    ///
    /// Requires: `term` is non-null and, at the point of the call, names a
    /// maximally-shared node currently reachable from some live GC root
    /// (mirrors [`ATerm::from_ptr`]'s precondition; the same reasoning
    /// applies — this immediately protects `term` in
    /// [`SEND_PROTECTION_SET`], so no further liveness is required from the
    /// caller). Guarantees: the returned `ATermSend` keeps `term` alive,
    /// markable and dereferenceable from *any* thread, until it is dropped.
    pub unsafe fn from_ptr(term: *const ffi::_aterm) -> ATermSend {
        ATermSend::protect(term)
    }

    /// Returns a borrowed view of the protected term.
    pub fn copy(&self) -> ATermRef<'_> {
        // SAFETY: the term stays protected in the global send set for as long as
        // `self` lives, so the borrow cannot outlive the term's liveness.
        unsafe { ATermRef::new(self.term.ptr) }
    }

    /// Protects the term on the *current* thread, returning an owning [`ATerm`].
    pub fn protect_local(&self) -> ATerm {
        self.copy().protect()
    }

    /// Returns the raw maximally shared term address underlying this term.
    pub fn address(&self) -> *const ffi::_aterm {
        self.term.ptr
    }
}

impl From<&ATerm> for ATermSend {
    fn from(term: &ATerm) -> Self {
        ATermSend::protect(term.address())
    }
}

impl From<&ATermSend> for ATerm {
    fn from(term: &ATermSend) -> Self {
        term.protect_local()
    }
}

impl Clone for ATermSend {
    fn clone(&self) -> Self {
        ATermSend::protect(self.term.ptr)
    }
}

impl Drop for ATermSend {
    fn drop(&mut self) {
        if !self.term.ptr.is_null() {
            // SAFETY: `self.root` was protected when this `ATermSend` was created
            // and `Drop` runs exactly once, so the root is unprotected once.
            unsafe {
                SEND_PROTECTION_SET.lock().unprotect(self.root);
            }
        }
    }
}

impl fmt::Display for ATermSend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.copy())
    }
}

impl fmt::Debug for ATermSend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.copy())
    }
}

impl Hash for ATermSend {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.copy().hash(state)
    }
}

impl PartialEq for ATermSend {
    fn eq(&self, other: &Self) -> bool {
        self.term.ptr == other.term.ptr
    }
}

impl Eq for ATermSend {}

impl PartialOrd for ATermSend {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ATermSend {
    fn cmp(&self, other: &Self) -> Ordering {
        self.copy().cmp(&other.copy())
    }
}

/// An iterator over the arguments of a term.
#[derive(Default)]
pub struct ATermArgs<'a> {
    term: ATermRef<'a>,
    arity: usize,
    index: usize,
}

impl<'a> ATermArgs<'a> {
    fn new(term: ATermRef<'a>) -> ATermArgs<'a> {
        let arity = term.get_head_symbol().arity();
        ATermArgs { term, arity, index: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.arity == 0
    }
}

impl<'a> Iterator for ATermArgs<'a> {
    type Item = ATermRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.arity {
            // SAFETY: `arg(self.index)` is a direct subterm of `self.term`, so
            // `self.term` is a valid parent witness for the lifetime upgrade.
            let res = unsafe { Some(self.term.arg(self.index).upgrade_unchecked(&self.term)) };

            self.index += 1;
            res
        } else {
            None
        }
    }
}

impl DoubleEndedIterator for ATermArgs<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.index < self.arity {
            // SAFETY: `arg(self.arity - 1)` is a direct subterm of `self.term`,
            // so `self.term` is a valid parent witness for the lifetime upgrade.
            let res = unsafe { Some(self.term.arg(self.arity - 1).upgrade_unchecked(&self.term)) };

            self.arity -= 1;
            res
        } else {
            None
        }
    }
}

impl ExactSizeIterator for ATermArgs<'_> {
    fn len(&self) -> usize {
        self.arity - self.index
    }
}

/// An iterator over all subterms of the given [ATerm] in preorder traversal, i.e.,
/// for f(g(a), b) we visit f(g(a), b), g(a), a, b.
pub struct TermIterator<'a> {
    queue: VecDeque<ATermRef<'a>>,
}

impl TermIterator<'_> {
    pub fn new(t: ATermRef) -> TermIterator {
        TermIterator {
            queue: VecDeque::from([t]),
        }
    }
}

impl<'a> Iterator for TermIterator<'a> {
    type Item = ATermRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.queue.pop_back() {
            Some(term) => {
                // Put subterms in the queue
                for argument in term.arguments().rev() {
                    // SAFETY: `argument` is a subterm of `term`, so `term` is a
                    // valid parent witness for the lifetime upgrade.
                    unsafe {
                        self.queue.push_back(argument.upgrade_unchecked(&term));
                    }
                }

                Some(term)
            }
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use crate::ATerm;
    use crate::ATermSend;
    use crate::atermpp::THREAD_TERM_POOL;

    /// `ThreadTermPool::create` (reached from the fully safe `ATerm::with_args`)
    /// only checks "argument count matches the symbol's arity" via
    /// `debug_assert_eq!`, which is compiled out in release builds. The
    /// vendored C++ side it feeds into does not re-check either: for
    /// arities 0-7 (the common case, dispatched in
    /// `aterm_pool::create_appl_dynamic`), `_aterm_appl<N>`'s iterator
    /// constructor (`detail/aterm.h`) loops exactly `N` times over the given
    /// iterator regardless of where the caller's real `end` is, guarded only
    /// by `assert(it != end)` — itself compiled out under the `-DNDEBUG=1`
    /// this workspace's release C++ build uses. So a too-short `arguments`
    /// slice makes the C++ side read past the end of the backing `Vec` and
    /// store the resulting garbage `usize`s as `*const _aterm` term-argument
    /// pointers, which are later dereferenced as live terms by anything that
    /// walks the term (`iter()`, `Debug`, hashing, marking, ...).
    ///
    /// This is a debug-vs-release-only bug: in a debug build the
    /// `debug_assert_eq!` panics before the FFI call is ever made, so this
    /// test is `#[ignore]`d there. Run it with
    /// `cargo test --release -p mcrl2 --lib
    /// with_args_arity_mismatch_reads_out_of_bounds_in_release -- --ignored
    /// --test-threads=1` (isolated to its own process: on the reviewed code
    /// this is expected to crash, abort, or print a garbage argument address
    /// rather than return normally).
    #[test]
    #[cfg_attr(
        debug_assertions,
        ignore = "the arity mismatch is caught by a debug_assert before reaching the FFI; run --release"
    )]
    fn with_args_arity_mismatch_reads_out_of_bounds_in_release() {
        use crate::Symbol;

        // A symbol of arity 4 backed by only 1 real argument: the C++ side
        // will read 3 elements past the end of the 1-element `Vec` that
        // backs this slice.
        let f = Symbol::new("with_args_arity_mismatch_reads_out_of_bounds_in_release_f", 4);
        let a = ATerm::constant(&Symbol::new(
            "with_args_arity_mismatch_reads_out_of_bounds_in_release_a",
            0,
        ));

        let bogus = ATerm::with_args(&f, &[a]);

        // Walk every argument, including the 3 out-of-range ones, which
        // dereferences whatever garbage pointer was read from adjacent heap
        // memory as though it were a live `_aterm`.
        for arg in bogus.arguments() {
            let _ = format!("{arg:?}");
        }
    }

    #[test]
    fn test_term_iterator() {
        let t = ATerm::from_string("f(g(a),b)").unwrap();

        let mut result = t.iter();
        assert_eq!(result.next().unwrap(), ATerm::from_string("f(g(a),b)").unwrap().copy());
        assert_eq!(result.next().unwrap(), ATerm::from_string("g(a)").unwrap().copy());
        assert_eq!(result.next().unwrap(), ATerm::from_string("a").unwrap().copy());
        assert_eq!(result.next().unwrap(), ATerm::from_string("b").unwrap().copy());
    }

    /// Creates `ATermSend`s on one thread, moves them to another thread where
    /// they survive a garbage collection and are finally dropped.
    #[test]
    fn test_aterm_send_cross_thread() {
        let terms: Vec<ATermSend> = (0..50)
            .map(|i| ATermSend::from(&ATerm::from_string(&format!("f(a{i}, b)")).unwrap()))
            .collect();

        let joined = thread::spawn(move || {
            // Allocate unrelated terms and force a collection; the moved-in
            // `ATermSend`s must remain valid because the global send set keeps
            // them marked.
            for i in 0..50 {
                let _ = ATerm::from_string(&format!("g(c{i})")).unwrap();
            }
            THREAD_TERM_POOL.with_borrow(|tp| tp.collect());

            for (i, term) in terms.iter().enumerate() {
                assert_eq!(term.copy(), ATerm::from_string(&format!("f(a{i}, b)")).unwrap().copy());
            }

            // Cloning on this thread and dropping the clones here, then returning
            // the originals to be dropped on the main thread, covers both
            // same-thread and cross-thread unprotect.
            let _clones: Vec<ATermSend> = terms.clone();
            terms
        })
        .join()
        .expect("worker thread panicked");

        // Drop the originals here, on the main thread (different from where the
        // clones above were dropped), and collect once more for good measure.
        drop(joined);
        THREAD_TERM_POOL.with_borrow(|tp| tp.collect());
    }
}
