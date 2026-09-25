use std::cmp::Ordering;
use std::collections::VecDeque;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::marker::PhantomData;
use std::mem::ManuallyDrop;
use std::sync::Arc;
use std::sync::Mutex;

use delegate::delegate;

use merc_sharedmutex::RecursiveLockReadGuard;
use merc_unsafety::ProtectionIndex;
use merc_unsafety::ProtectionSet;
use merc_unsafety::StablePointer;
use merc_utilities::MercError;
use merc_utilities::PhantomUnsend;

use crate::ATermIntRef;
use crate::ATermList;
use crate::Markable;
use crate::Symb;
use crate::SymbolRef;
use crate::Transmutable;
use crate::is_empty_list_term;
use crate::is_int_term;
use crate::is_list_term;
use crate::storage::GlobalTermPool;
use crate::storage::Marker;
use crate::storage::SharedTerm;
use crate::storage::THREAD_TERM_POOL;

/// Represents a first-order term, implemented for both the owned `ATerm` (no lifetime) and the
/// borrowed `ATermRef<'a>`.
///
/// The two lifetimes let an implementation with `Self: 'b` require `'b: 'a`, so its methods can
/// safely return `ATermRef<'a>` borrowed from `Self`.
pub trait Term<'a, 'b> {
    /// Protects the term from garbage collection, returning an owned [ATerm].
    fn protect(&self) -> ATerm;

    /// Returns the indexed argument of the term as an [ATermRef].
    fn arg(&'b self, index: usize) -> ATermRef<'a>;

    /// Returns the list of arguments as an [ATermArgs] collection.
    fn arguments(&'b self) -> ATermArgs<'a>;

    /// Makes a copy of the term, returning an [ATermRef] with the same lifetime as itself.
    fn copy(&'b self) -> ATermRef<'a>;

    /// Returns the head symbol of the term as a [SymbolRef].
    fn get_head_symbol(&'b self) -> SymbolRef<'a>;

    /// Returns a [TermIterator] over all arguments of the term in pre-order traversal.
    fn iter(&'b self) -> TermIterator<'a>;

    /// Returns a unique index of the term in the term pool.
    fn index(&self) -> usize;

    /// Returns the [ATermIndex] of the term in the term pool.
    fn shared(&self) -> &ATermIndex;
}

/// Type alias for [ATerm] indices, representing a stable pointer to a [SharedTerm] in the term pool.
pub type ATermIndex = StablePointer<SharedTerm>;

/// This represents a lifetime bound reference to an existing [ATerm].
#[derive(Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ATermRef<'a> {
    shared: ATermIndex,
    marker: PhantomData<&'a ()>,
}

/// Check that the ATermRef is the same size as a usize, now that the underlying
/// [ATermIndex] stores a thin (type-erased) pointer instead of a wide slice pointer.
#[cfg(not(debug_assertions))]
const _: () = assert!(std::mem::size_of::<ATermRef>() == std::mem::size_of::<usize>());

/// Since we have NonZero we can use a niche value optimisation for option.
#[cfg(not(debug_assertions))]
const _: () = assert!(std::mem::size_of::<Option<ATermRef>>() == std::mem::size_of::<usize>());

/// # Safety
///
/// `ATermRef<'a> { shared: StablePointer<SharedTerm>, marker: PhantomData<&'a ()> }`. `shared`
/// is a non-owning, type-erased raw pointer (`merc_unsafety::StablePointer` wraps a `NonNull`),
/// which is `!Send`/`!Sync` unconditionally; these two impls are what let an `ATermRef` cross a
/// `Send`/`Sync` boundary at all. `PhantomData<&'a ()>` carries no runtime state and does not
/// itself affect either trait.
///
/// Moving or sharing a live `ATermRef<'a>` across threads is sound because, for the whole
/// duration any thread holds one:
/// 1. **No data race on the pointee.** Every `SharedTerm` in the global pool is immutable once
///    inserted: `ATermStorage::insert` only ever writes a fresh allocation
///    (`SharedTerm::construct`), and the only other writer, the sweep in
///    `GlobalTermPool::sweep_terms_and_symbols`, removes whole entries rather than mutating a
///    live one. So two threads reading the same `SharedTerm` concurrently through two
///    `ATermRef`s that alias the same address never race.
/// 2. **No use-after-free.** The address a `StablePointer<SharedTerm>` refers to is reclaimed
///    only by `GlobalTermPool::collect_garbage`'s sweep, which runs exclusively under the pool's
///    write lock (`GlobalBfSharedMutex::write`, via `RecursiveLock::write`). That write lock
///    cannot be granted while any thread holds a `read_recursive()` guard, and holding a live,
///    in-bounds `ATermRef<'a>` requires the term to be reachable from a GC root for at least
///    `'a` (the caller-side contract of [`ATermRef::from_index`]) -- every safe path that
///    produces one either holds such a read guard for the term's whole `'a` (`Return`) or has
///    already registered it in a protection set that `mark_roots` visits before any sweep can
///    run. So no thread, at any point while it holds an `ATermRef`, can observe a sweep freeing
///    the memory a concurrently-held `ATermRef` (on any thread, including its own) points at.
unsafe impl Send for ATermRef<'_> {}
unsafe impl Sync for ATermRef<'_> {}

impl ATermRef<'_> {
    /// Creates a new term reference from the given [ATermIndex].
    ///
    /// # Safety
    ///
    /// This function is unsafe because it does not check if the index is valid for the given lifetime.
    pub unsafe fn from_index(shared: &ATermIndex) -> Self {
        ATermRef {
            // SAFETY: the caller guarantees the index remains valid for the
            // lifetime of the returned reference.
            shared: unsafe { shared.copy() },
            marker: PhantomData,
        }
    }
}

impl<'a, 'b> Term<'a, 'b> for ATermRef<'a> {
    fn protect(&self) -> ATerm {
        THREAD_TERM_POOL.with(|tp| tp.protect(&self.copy()))
    }

    fn arg(&self, index: usize) -> ATermRef<'a> {
        debug_assert!(
            index < self.get_head_symbol().arity(),
            "arg({index}) is not defined for term {self:?}"
        );

        // Safety: self is ATermRef<'a>, so the GC keeps all its arguments
        // protected for 'a. We copy the stable pointer rather than borrowing
        // through the short-lived slice reference.
        unsafe { ATermRef::from_index(self.shared().deref().arguments()[index].shared()) }
    }

    fn arguments(&self) -> ATermArgs<'a> {
        ATermArgs::new(self.copy())
    }

    fn copy(&self) -> ATermRef<'a> {
        unsafe { ATermRef::from_index(self.shared()) }
    }

    fn get_head_symbol(&'b self) -> SymbolRef<'a> {
        unsafe { std::mem::transmute::<SymbolRef<'b>, SymbolRef<'a>>(self.shared().deref().symbol().copy()) }
    }

    fn iter(&self) -> TermIterator<'a> {
        TermIterator::new(self.copy())
    }

    fn index(&self) -> usize {
        // SAFETY: `self` is an `ATermRef<'a>`, so the GC keeps the term alive
        // for `'a` and the pointee is valid to read.
        unsafe { self.shared.deref() }.index()
    }

    fn shared(&self) -> &ATermIndex {
        &self.shared
    }
}

impl Markable for ATermRef<'_> {
    fn mark(&self, marker: &mut Marker) {
        marker.mark(self);
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        term == self
    }

    fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool {
        self.get_head_symbol() == *symbol
    }

    fn len(&self) -> usize {
        1
    }
}

impl fmt::Display for ATermRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

/// A pending unit of work for the iterative [ATermRef] formatter: either a
/// subterm that still needs to be formatted, or a literal separator that was
/// deferred until its subterms are printed.
enum FormatFrame<'a> {
    Term(ATermRef<'a>),
    Str(&'static str),
}

impl fmt::Debug for ATermRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Formatting recurses once per nesting level in the naive
        // implementation, which overflows the stack on deeply nested terms
        // (e.g. Peano numerals). Walk the term with an explicit stack instead.
        let mut stack = vec![FormatFrame::Term(self.copy())];

        while let Some(frame) = stack.pop() {
            match frame {
                FormatFrame::Str(s) => write!(f, "{s}")?,
                FormatFrame::Term(t) => {
                    if is_int_term(&t) {
                        write!(f, "{}", Into::<ATermIntRef>::into(t.copy()))?;
                    } else if is_list_term(&t) || is_empty_list_term(&t) {
                        write!(f, "{}", Into::<ATermList<ATerm>>::into(t.copy()))?;
                    } else if t.arguments().is_empty() {
                        write!(f, "{}", t.get_head_symbol().name())?;
                    } else {
                        // Format the term with its head symbol and arguments, avoiding trailing comma.
                        write!(f, "{:?}(", t.get_head_symbol())?;

                        let args = t.arguments().rev();
                        let num_args = args.len();
                        stack.push(FormatFrame::Str(")"));
                        for (i, arg) in args.enumerate() {
                            stack.push(FormatFrame::Term(arg));
                            if i + 1 < num_args {
                                stack.push(FormatFrame::Str(", "));
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

/// The protected counterpart of `ATermRef`: a term reference registered in the
/// current thread's protection set, keeping it alive across garbage collection.
///
/// # Safety
///
/// Terms use thread-local state for their protection mechanism, so `ATerm` is
/// not `Send`, and must not be stored in thread-local storage itself: the order
/// in which thread-local destructors run is undefined, so a term could be
/// dropped after its owning term pool. Wrap such a term in `ManuallyDrop`
/// instead, because exiting the thread cleans up the protection sets
/// regardless.
pub struct ATerm {
    term: ATermRef<'static>,

    /// The root of the term in the protection set
    root: ProtectionIndex,

    // ATerm is not Send because it uses thread-local state for its protection
    // mechanism. However, it can be Sync since terms are immutable, and unlike
    // `Rc` cloning results in a local protected copy.
    _marker: PhantomUnsend,
}

impl ATerm {
    /// Creates a new term with the given symbol and arguments.
    pub fn with_args<'a, 'b, S: Symb<'a, 'b>, T: Term<'a, 'b>>(
        symbol: &'b S,
        args: &'b [T],
    ) -> Return<ATermRef<'static>> {
        THREAD_TERM_POOL.with(|tp| tp.create_term(symbol, args))
    }

    /// Creates a new term with the given symbol and an iterator over the arguments.
    pub fn with_iter<'a, 'b, 'c, 'd, S, I, T>(symbol: &'b S, iter: I) -> ATerm
    where
        S: Symb<'a, 'b>,
        I: IntoIterator<Item = T>,
        T: Term<'c, 'd>,
    {
        THREAD_TERM_POOL.with(|tp| tp.create_term_iter(symbol, iter))
    }

    /// Creates a new term with the given symbol and an iterator over the arguments.
    pub fn try_with_iter<'a, 'b, 'c, 'd, S, I, T>(symbol: &'b S, iter: I) -> Result<ATerm, MercError>
    where
        S: Symb<'a, 'b>,
        I: IntoIterator<Item = Result<T, MercError>>,
        T: Term<'c, 'd>,
    {
        THREAD_TERM_POOL.with(|tp| tp.try_create_term_iter(symbol, iter))
    }

    /// Creates a new term with the given symbol and a head term, along with a list of arguments.
    pub fn with_iter_head<'a, 'b, 'c, 'd, 'e, 'f, I, S, T, H>(symbol: &'b S, head: &'d H, iter: I) -> ATerm
    where
        S: Symb<'a, 'b>,
        H: Term<'c, 'd>,
        I: IntoIterator<Item = T>,
        T: Term<'e, 'f>,
    {
        THREAD_TERM_POOL.with(|tp| tp.create_term_iter_head(symbol, head, iter))
    }

    /// Creates a new constant term (arity 0) for the given symbol.
    pub fn constant<'a, 'b, S: Symb<'a, 'b>>(symbol: &'b S) -> ATerm {
        THREAD_TERM_POOL.with(|tp| tp.create_constant(symbol))
    }

    /// Constructs a term from the given string.
    pub fn from_string(text: &str) -> Result<ATerm, MercError> {
        THREAD_TERM_POOL.with(|tp| tp.from_string(text))
    }

    /// Returns the term as a borrowed [ATermRef].
    pub fn get(&self) -> ATermRef<'_> {
        self.term.copy()
    }

    /// Returns the root of the term
    pub fn root(&self) -> ProtectionIndex {
        self.root
    }

    /// Replace this term by the given term in place.
    pub fn replace<'a, 'b, T>(&mut self, value: Return<T>)
    where
        T: Term<'a, 'b>,
        'b: 'a,
    {
        // Replace the current term in the protection set by the value.
        // SAFETY: `value` still holds the recursive read guard, so no garbage
        // collection can run before `tp.replace` registers this index under
        // `self.root`; from then on the protection set keeps the term alive
        // until this `ATerm` unprotects it.
        let index = unsafe { value.shared().copy() };
        // Move the guard out of `value` without running `Return::drop`, since we release the
        // guard ourselves inside the `THREAD_TERM_POOL.with` closure below.
        let mut value = ManuallyDrop::new(value);
        // SAFETY: `value` is wrapped in `ManuallyDrop`, so `guard` (and `term`) are not dropped
        // or read again after this move.
        let guard = unsafe { ManuallyDrop::take(&mut value.guard) };
        THREAD_TERM_POOL.with(|tp| tp.replace(guard, self.root, unsafe { index.copy() }));

        // Set the term itself.
        self.term = unsafe { ATermRef::from_index(&index) };
    }

    /// Creates a new term from the given reference and protection set root
    /// entry.
    pub(crate) fn from_index(term: &ATermIndex, root: ProtectionIndex) -> ATerm {
        unsafe {
            ATerm {
                term: ATermRef::from_index(term),
                root,
                _marker: PhantomData,
            }
        }
    }
}

impl<'a, 'b> Term<'a, 'b> for ATerm
where
    'b: 'a,
{
    delegate! {
        to self.term {
            fn protect(&self) -> ATerm;
            fn arg(&self, index: usize) -> ATermRef<'a>;
            fn arguments(&self) -> ATermArgs<'a>;
            fn copy(&self) -> ATermRef<'a>;
            fn get_head_symbol(&self) -> SymbolRef<'a>;
            fn iter(&self) -> TermIterator<'a>;
            fn index(&self) -> usize;
            fn shared(&self) -> &ATermIndex;
        }
    }
}

impl Markable for ATerm {
    fn mark(&self, marker: &mut Marker) {
        marker.mark(&self.term);
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        *term == self.term
    }

    fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool {
        self.get_head_symbol() == *symbol
    }

    fn len(&self) -> usize {
        1
    }
}

impl Drop for ATerm {
    fn drop(&mut self) {
        THREAD_TERM_POOL.with(|tp| tp.drop(self))
    }
}

impl Clone for ATerm {
    fn clone(&self) -> Self {
        self.copy().protect()
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

/// A sendable variant of an `ATerm`, usable from a different thread than the one that created it.
///
/// Keeps a reference to the protection set it registered with, so it can unregister itself on drop.
pub struct ATermSend {
    term: ATermRef<'static>,

    /// The root of the term in the protection set
    root: ProtectionIndex,

    /// A shared reference to the protection set that this term was created in.
    protection_set: Arc<Mutex<ProtectionSet<ATermIndex>>>,
}

/// # Safety
///
/// `ATermSend { term: ATermRef<'static>, root: ProtectionIndex, protection_set:
/// Arc<Mutex<ProtectionSet<ATermIndex>>> }`.
/// - `term` is `Send`/`Sync` by the contract on `ATermRef`'s own impls above.
/// - `root` (`ProtectionIndex`, a `Copy` newtype over a plain generational `usize` index) has no
///   interior mutability, so it is trivially both.
/// - `protection_set` is `Arc<Mutex<ProtectionSet<ATermIndex>>>`, which is `Send`/`Sync`
///   automatically as soon as `ATermIndex = StablePointer<SharedTerm>` is -- exactly the
///   property the `ATermRef` impls above establish (`Mutex<X>` needs `X: Send` to be `Sync`,
///   `Arc<X>` needs `X: Send + Sync` to be `Send`, and `ProtectionSet<ATermIndex>`'s only
///   `ATermIndex`-typed storage is a plain `Vec`, adding no further raw-pointer field of its
///   own).
///
/// So nothing here needs re-deriving from scratch: this impl exists only because `SharedTerm`
/// (embedded in `ATermIndex` via `ATermRef`) is a self-referential type that already required
/// one manual `Send`/`Sync` assertion, and a struct that contains an unasserted `!Send`/`!Sync`
/// field anywhere in its transitive closure cannot have those traits auto-derived for it either
/// -- so the compiler cannot discharge this on `ATermSend`'s behalf even though every field's
/// own requirement is already met once the `ATermRef` argument above is accepted.
///
/// The invariant `ATermSend` itself must additionally uphold across the send is that `root`
/// stays protected in exactly `protection_set` until `Drop for ATermSend` unprotects it exactly
/// once (see `ATermSend::from`); that keeps the term reachable by the GC regardless of which
/// thread is now holding it, so point 2 of the `ATermRef` argument above (no use-after-free)
/// still holds after the move, on the receiving thread.
unsafe impl Send for ATermSend {}
unsafe impl Sync for ATermSend {}

impl ATermSend {
    /// Takes ownership of an `ATerm` and makes it send.
    pub fn from(term: ATerm) -> Self {
        // We need to insert the term into the protection set of the current
        // thread, and keep track of the root index to properly unprotect it on
        // drop.
        let protection_set = THREAD_TERM_POOL.with(|tp| tp.send_term_protection_set().clone());
        let term_ref: ATermRef<'static> = unsafe { ATermRef::from_index(&term.term.shared) };
        // SAFETY: `term` keeps the index alive during this call, and inserting
        // the copy into the send-term protection set makes it a GC root until
        // the `ATermSend` is dropped.
        let root = protection_set
            .lock()
            .expect("Lock poisoned!")
            .protect(unsafe { term.shared().copy() });

        Self {
            term: term_ref,
            root,
            protection_set,
        }
    }
}

impl Drop for ATermSend {
    fn drop(&mut self) {
        let mut guard = match self.protection_set.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        // SAFETY: `self.root` was protected when this `ATermSend` was created and
        // `Drop` runs exactly once, so the root is unprotected exactly once.
        unsafe {
            guard.unprotect(self.root);
        }
    }
}

impl<'a, 'b> Term<'a, 'b> for ATermSend
where
    'b: 'a,
{
    delegate! {
        to self.term {
            fn protect(&self) -> ATerm;
            fn arg(&self, index: usize) -> ATermRef<'a>;
            fn arguments(&self) -> ATermArgs<'a>;
            fn copy(&self) -> ATermRef<'a>;
            fn get_head_symbol(&self) -> SymbolRef<'a>;
            fn iter(&self) -> TermIterator<'a>;
            fn index(&self) -> usize;
            fn shared(&self) -> &ATermIndex;
        }
    }
}

/// This is a wrapper around a term that indicates it is being returned from a
/// function.
///
/// The resulting term can have a lifetime tied to the thread-local term pool.
pub struct Return<T> {
    term: T,

    /// The recursive read guard borrows from the [`RecursiveLock`](merc_sharedmutex::RecursiveLock) owned by this thread's
    /// [crate::storage::ThreadTermPool]. It is wrapped in [ManuallyDrop] so that its own
    /// [Drop] (which dereferences that lock) never runs implicitly; instead [Return::drop]
    /// releases it through [THREAD_TERM_POOL], which panics if the thread-local pool is already
    /// gone rather than dereferencing a dangling lock.
    guard: ManuallyDrop<RecursiveLockReadGuard<'static, GlobalTermPool>>,
}

impl<T> Return<T> {
    /// Creates a new return value wrapping the given term.
    pub(crate) fn new(guard: RecursiveLockReadGuard<'static, GlobalTermPool>, term: T) -> Self {
        Return {
            term,
            guard: ManuallyDrop::new(guard),
        }
    }

    /// Casts the inner term to another type, while keeping the same guard.
    pub fn cast<U>(self) -> Return<U>
    where
        T: Into<U>,
    {
        // Move the guard out without running `Self::drop`, then hand it to the new `Return`.
        let mut this = ManuallyDrop::new(self);
        // SAFETY: `this` is wrapped in `ManuallyDrop`, so `guard` is not read or dropped again.
        let guard = unsafe { ManuallyDrop::take(&mut this.guard) };
        // SAFETY: `term` is likewise not dropped or read again after this move.
        let term = unsafe { std::ptr::read(&this.term) };
        Return {
            term: term.into(),
            guard: ManuallyDrop::new(guard),
        }
    }
}

impl<T> Drop for Return<T> {
    fn drop(&mut self) {
        // Release the guard through the thread-local pool. If the thread-local pool has already
        // been destroyed, `with` panics before we ever touch the guard, so we never dereference
        // the dangling `RecursiveLock` it borrows from. This turns a would-be use-after-free
        // into a deterministic panic.
        THREAD_TERM_POOL.with(|_| {
            // SAFETY: `guard` is never used again after this `Return` is dropped.
            unsafe { ManuallyDrop::drop(&mut self.guard) };
        });
    }
}

impl<T: Transmutable> Return<T> {
    /// Maps the inner term to another type, while keeping the same guard.
    pub fn inner(&self) -> &T::Target<'_> {
        // SAFETY: The returned lifetime is bound to the borrow of `self` by the signature.
        unsafe { self.term.transmute_lifetime() }
    }
}

impl<'a, 'b, T: Term<'a, 'b>> Term<'a, 'b> for Return<T>
where
    'b: 'a,
{
    delegate! {
        to self.term {
            fn protect(&self) -> ATerm;
            fn arg(&'b self, index: usize) -> ATermRef<'a>;
            fn arguments(&'b self) -> ATermArgs<'a>;
            fn copy(&'b self) -> ATermRef<'a>;
            fn get_head_symbol(&'b self) -> SymbolRef<'a>;
            fn iter(&'b self) -> TermIterator<'a>;
            fn index(&self) -> usize;
            fn shared(&self) -> &ATermIndex;
        }
    }
}

/// An iterator over the arguments of a term.
pub struct ATermArgs<'a> {
    term: Option<ATermRef<'a>>,
    arity: usize,
    index: usize,
}

impl<'a> ATermArgs<'a> {
    pub fn empty() -> ATermArgs<'static> {
        ATermArgs {
            term: None,
            arity: 0,
            index: 0,
        }
    }

    fn new(term: ATermRef<'a>) -> ATermArgs<'a> {
        let arity = term.get_head_symbol().arity();
        ATermArgs {
            term: Some(term),
            arity,
            index: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.arity == 0
    }
}

impl<'a> Iterator for ATermArgs<'a> {
    type Item = ATermRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.arity {
            let res = Some(self.term.as_ref().unwrap().arg(self.index));

            self.index += 1;
            res
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        // Report the exact remaining length so adapters such as `Skip` and `Map` keep
        // satisfying the `ExactSizeIterator` invariant (upper bound == lower bound).
        let remaining = self.arity - self.index;
        (remaining, Some(remaining))
    }
}

impl DoubleEndedIterator for ATermArgs<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.index < self.arity {
            let res = Some(self.term.as_ref().unwrap().arg(self.arity - 1));

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
                    self.queue.push_back(argument);
                }

                Some(term)
            }
            None => None,
        }
    }
}

/// Blanket implementation so a `&T` can be passed anywhere a `T: Term` is expected, letting
/// callers pass borrowed terms without an explicit `.copy()`.
impl<'a, 'b, T: Term<'a, 'b>> Term<'a, 'b> for &'b T {
    fn protect(&self) -> ATerm {
        (*self).protect()
    }

    fn arg(&self, index: usize) -> ATermRef<'a> {
        (*self).arg(index)
    }

    fn arguments(&self) -> ATermArgs<'a> {
        (*self).arguments()
    }

    fn copy(&self) -> ATermRef<'a> {
        (*self).copy()
    }

    fn get_head_symbol(&self) -> SymbolRef<'a> {
        (*self).get_head_symbol()
    }

    fn iter(&self) -> TermIterator<'a> {
        (*self).iter()
    }

    fn index(&self) -> usize {
        (*self).index()
    }

    fn shared(&self) -> &ATermIndex {
        (*self).shared()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use parking_lot::Mutex;

    use crate::ATerm;
    use crate::ATermSend;
    use crate::Symbol;
    use crate::Term;
    use crate::storage::THREAD_TERM_POOL;

    #[test]
    fn test_debug_multi_arg_term_argument_order() {
        // Pins the exact left-to-right, comma-separated output of the iterative
        // formatter for a multi-argument, multi-level term.
        let f = Symbol::new("f_debug_order_test", 3);
        let g = Symbol::new("g_debug_order_test", 1);
        let a = ATerm::constant(&Symbol::new("a_debug_order_test", 0));
        let b = ATerm::constant(&Symbol::new("b_debug_order_test", 0));
        let c = ATerm::with_args(&g, &[b.copy()]).protect();
        let t = ATerm::with_args(&f, &[a.copy(), b.copy(), c.copy()]).protect();

        assert_eq!(
            format!("{t:?}"),
            "f_debug_order_test(a_debug_order_test, b_debug_order_test, g_debug_order_test(b_debug_order_test))"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Building 300k terms is too slow under miri.
    fn test_debug_deeply_nested_term() {
        // Debug formatting recurses once per nesting level, so a deeply nested
        // term must not overflow the stack.
        let f = Symbol::new("f", 1);
        let mut t = ATerm::constant(&Symbol::new("a", 0));
        for _ in 0..300_000 {
            t = ATerm::with_args(&f, &[t]).protect();
        }

        let text = format!("{t:?}");
        assert!(text.starts_with("f(f("));
        assert!(text.ends_with("))"));
    }

    #[test]
    fn test_send_term_outlives_creating_thread() {
        // An ATermSend created on a thread must keep its term alive after that thread exits,
        // even across garbage collection. This pins the invariant relied upon when GC reclaims
        // the send-term protection set slots of exited threads: a slot must not be released while
        // a live ATermSend still references it.
        let symbol = Symbol::new("send_outlives", 0);
        let term = std::thread::spawn(|| ATermSend::from(ATerm::constant(&Symbol::new("send_outlives", 0))))
            .join()
            .unwrap();

        // The creating thread has exited; force collections from this thread.
        THREAD_TERM_POOL.with(|tp| {
            tp.force_collect_garbage();
            tp.force_collect_garbage();
        });

        // The term must still be alive and structurally intact.
        assert_eq!(term.get_head_symbol(), symbol.copy());
    }

    #[test]
    #[cfg_attr(miri, ignore)] // This test runs too slow under miri.
    fn test_send_terms() {
        // Run two threads that create and drop sendable terms, and check that the protection set is properly cleaned up.
        let symbol = Symbol::new("a", 0);
        let term = Arc::new(Mutex::new(ATermSend::from(ATerm::constant(&symbol))));

        let thread_a = {
            let term = term.clone();

            std::thread::spawn(move || {
                let symbol = Symbol::new("a", 0);

                for _ in 0..100000 {
                    *term.lock() = ATermSend::from(ATerm::constant(&symbol));
                }
            })
        };

        for _ in 0..100000 {
            *term.lock() = ATermSend::from(ATerm::constant(&symbol));
        }

        thread_a.join().unwrap();
    }
}
