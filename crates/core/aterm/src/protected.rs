#[cfg(debug_assertions)]
use std::cell::RefCell;
use std::fmt::Debug;
use std::hash::Hash;
use std::marker::PhantomData;
use std::mem::transmute;
use std::ops::Deref;
use std::ops::DerefMut;
use std::sync::Arc;

use merc_unsafety::ProtectionIndex;
use merc_utilities::PhantomUnsend;

use crate::Markable;
use crate::Symb;
use crate::SymbolRef;
use crate::Term;
use crate::Transmutable;
use crate::aterm::ATermRef;
use crate::storage::GcMutex;
use crate::storage::GcMutexReadGuard;
use crate::storage::SendContainerProtectionSet;
use crate::storage::THREAD_TERM_POOL;

/// A container of objects, typically either terms or objects containing terms,
/// that implement [Markable]. These store [ATermRef]`<'static>` values that are
/// protected during garbage collection by being in the container itself.
pub struct Protected<C> {
    container: Arc<GcMutex<C>>,
    root: ProtectionIndex,

    // Protected is not Send because it uses thread-local state for its protection
    // mechanism.
    _unsend: PhantomUnsend,
}

impl<C: Markable + Send + Sync + Transmutable + 'static> Protected<C> {
    /// Creates a new Protected container from a given container.
    pub fn new(container: C) -> Protected<C> {
        let shared = Arc::new(GcMutex::new(container));

        let root = THREAD_TERM_POOL.with(|tp| tp.protect_container(shared.clone()));

        Protected {
            container: shared,
            root,
            _unsend: Default::default(),
        }
    }

    /// Provides mutable access to the underlying container through a [ProtectedWriteGuard] that
    /// only exists during `f`.
    ///
    /// A term removed from the container is no longer protected once the guard is dropped, so the
    /// guard's lifetime `'g` is universally quantified to keep such terms from escaping `f`. As
    /// with [std::thread::scope], terms that outlive `'env` can still be inserted directly.
    pub fn write<'env, F, R>(&mut self, f: F) -> R
    where
        F: for<'g> FnOnce(&'g mut ProtectedWriteGuard<'g, 'env, C::Target<'g>>) -> R,
    {
        // SAFETY: Protected is `!Send` so it is only ever used from one thread,
        // and `write` takes `&mut self`, so no other guard from this handle
        // overlaps. The only other access is the global garbage collector,
        // which only accesses the container when this handle is dropped, so it
        // cannot overlap either.
        let mutex = unsafe { &mut *(Arc::as_ptr(&self.container) as *mut GcMutex<C>) };
        write_scoped(mutex, f)
    }

    /// Provides immutable access to the underlying container, returning a [ProtectedReadGuard].
    pub fn read(&self) -> ProtectedReadGuard<'_, C> {
        ProtectedReadGuard::new(self.container.lock())
    }
}

impl<C: Default + Markable + Send + Sync + Transmutable + 'static> Default for Protected<C> {
    fn default() -> Self {
        Protected::new(Default::default())
    }
}

impl<C: Clone + Markable + Send + Sync + Transmutable + 'static> Clone for Protected<C> {
    fn clone(&self) -> Self {
        Protected::new(self.container.lock().clone())
    }
}

impl<C: Hash + Markable> Hash for Protected<C> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.container.lock().hash(state)
    }
}

impl<C: PartialEq + Markable> PartialEq for Protected<C> {
    fn eq(&self, other: &Self) -> bool {
        self.container.lock().eq(&other.container.lock())
    }
}

impl<C: PartialOrd + Markable> PartialOrd for Protected<C> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        let c: &C = &other.container.lock();
        self.container.lock().partial_cmp(c)
    }
}

impl<C: Debug + Markable> Debug for Protected<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let c: &C = &self.container.lock();
        write!(f, "{c:?}")
    }
}

impl<C: Eq + PartialEq + Markable> Eq for Protected<C> {}
impl<C: Ord + PartialEq + PartialOrd + Markable> Ord for Protected<C> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let c: &C = &other.container.lock();
        self.container.lock().partial_cmp(c).expect("Terms are totally ordered")
    }
}

impl<C> Drop for Protected<C> {
    fn drop(&mut self) {
        THREAD_TERM_POOL.with(|tp| {
            tp.drop_container(self.root);
        });
    }
}

/// A [`Protected`]-like container that can be sent to, and used from, a
/// different thread than the one that created it.
///
/// # Details
///
/// [`Protected`] registers its container in the *calling* thread's own
/// protection set and later unregisters it through that same thread-local
/// lookup, so it can only ever be dropped on the thread that created it.
/// Instead, this type registers its container in a *globally-scanned*
/// protection set, so it can implement `Send`.
pub struct ProtectedSend<C> {
    container: Arc<GcMutex<C>>,
    root: ProtectionIndex,

    /// A shared handle to the protection set this container was registered in.
    protection_set: SendContainerProtectionSet,
}

impl<C: Markable + Send + Sync + Transmutable + 'static> ProtectedSend<C> {
    /// Creates a new protected container from a given container.
    pub fn new(container: C) -> ProtectedSend<C> {
        let shared = Arc::new(GcMutex::new(container));

        let protection_set = THREAD_TERM_POOL.with(|tp| tp.send_container_protection_set().clone());
        // Inserting the clone into the (globally-scanned) send-container protection set makes it
        // a GC root until this `GlobalProtected` is dropped.
        let root = protection_set.lock().expect("Lock poisoned!").protect(shared.clone());

        ProtectedSend {
            container: shared,
            root,
            protection_set,
        }
    }

    /// Provides mutable access to the underlying container, see [Protected::write].
    pub fn write<'env, F, R>(&mut self, f: F) -> R
    where
        F: for<'g> FnOnce(&'g mut ProtectedWriteGuard<'g, 'env, C::Target<'g>>) -> R,
    {
        // SAFETY: `write` takes `&mut self`, so no other guard from this handle overlaps, and
        // ownership (hence any access) is confined to a single thread at a time. The only other
        // access is the global garbage collector, which -- as for `Protected` -- only reaches
        // the container's contents through `GcMutex::lock`, so it cannot overlap either.
        let mutex = unsafe { &mut *(Arc::as_ptr(&self.container) as *mut GcMutex<C>) };
        write_scoped(mutex, f)
    }

    /// Provides immutable access to the underlying container, returning a [ProtectedReadGuard].
    pub fn read(&self) -> ProtectedReadGuard<'_, C> {
        ProtectedReadGuard::new(self.container.lock())
    }
}

impl<C: Default + Markable + Send + Sync + Transmutable + 'static> Default for ProtectedSend<C> {
    fn default() -> Self {
        ProtectedSend::new(Default::default())
    }
}

impl<C> Drop for ProtectedSend<C> {
    fn drop(&mut self) {
        let mut guard = match self.protection_set.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        // SAFETY: `self.root` was protected when this instance was created and
        // `Drop` runs exactly once, so the root is unprotected exactly once.
        unsafe {
            guard.unprotect(self.root);
        }
    }
}

/// Runs `f` with a [ProtectedWriteGuard] on `mutex`, which prevents garbage collection meanwhile.
fn write_scoped<'env, C, F, R>(mutex: &mut GcMutex<C>, f: F) -> R
where
    C: Markable + Transmutable,
    F: for<'g> FnOnce(&'g mut ProtectedWriteGuard<'g, 'env, C::Target<'g>>) -> R,
{
    // The guard is borrowed for its own lifetime, so it cannot own anything with drop glue that
    // uses that lifetime. The lock and the debug bookkeeping therefore live here instead.
    let mut lock = mutex.lock_mut();

    #[cfg(debug_assertions)]
    let protected = RefCell::new(Vec::new());
    #[cfg(debug_assertions)]
    let protected_symbols = RefCell::new(Vec::new());

    // SAFETY: `f` is generic over the lifetime of the container, so nothing that it takes from the
    // container outlives this call, during which the lock prevents garbage collection.
    let container = unsafe { lock.transmute_lifetime_mut() };
    let result = f(&mut ProtectedWriteGuard {
        container,
        _env: PhantomData,
        #[cfg(debug_assertions)]
        protected: &protected,
        #[cfg(debug_assertions)]
        protected_symbols: &protected_symbols,
    });

    #[cfg(debug_assertions)]
    {
        for term in protected.borrow().iter() {
            debug_assert!(lock.contains_term(term), "Term was protected but not actually inserted");
        }

        for symbol in protected_symbols.borrow().iter() {
            debug_assert!(
                lock.contains_symbol(symbol),
                "Symbol was protected but not actually inserted"
            );
        }
    }

    result
}

/// Mutable access to the container of a [Protected] or [ProtectedSend], see [Protected::write].
pub struct ProtectedWriteGuard<'a, 'env, T: ?Sized> {
    container: &'a mut T,

    /// Within the closure passed to `write`, the type `&'a mut ProtectedWriteGuard<'a, 'env, T>`
    /// of its argument implies `'env: 'a`, so terms that outlive `'env` can be inserted. The guard
    /// is generic over the container with lifetime `'a`, rather than the stored container with
    /// lifetime `'static`, since the latter would also imply `'static: 'a`, and the compiler then
    /// requires inserted terms to be `'static`.
    _env: PhantomData<&'a mut &'env ()>,

    /// Terms that have been protected during the lifetime of this guard.
    #[cfg(debug_assertions)]
    protected: &'a RefCell<Vec<ATermRef<'static>>>,

    /// Symbols that have been protected during the lifetime of this guard.
    #[cfg(debug_assertions)]
    protected_symbols: &'a RefCell<Vec<SymbolRef<'static>>>,
}

impl<'a, C: ?Sized> ProtectedWriteGuard<'a, '_, C> {
    /// Yields a term to insert into the container.
    ///
    /// # Safety
    ///
    /// The invariant to uphold is that the resulting term MUST be inserted into
    /// the container. This is checked in debug mode, but not in release mode.
    /// If this invariant is violated, undefined behaviour may occur during
    /// garbage collection.
    pub unsafe fn protect<'b, T: Term<'a, 'b>>(&self, term: &'b T) -> ATermRef<'static> {
        unsafe {
            // Store terms that are marked as protected to check if they are
            // actually in the container when the protection is dropped.
            #[cfg(debug_assertions)]
            self.protected
                .borrow_mut()
                .push(transmute::<ATermRef<'_>, ATermRef<'static>>(term.copy()));

            transmute::<ATermRef<'_>, ATermRef<'static>>(term.copy())
        }
    }

    /// Yields a symbol to insert into the container.
    ///
    /// # Safety
    ///
    /// The invariant to uphold is that the resulting symbol MUST be inserted
    /// into the container.
    pub unsafe fn protect_symbol<'b, S: Symb<'a, 'b>>(&self, symbol: &'b S) -> SymbolRef<'static> {
        unsafe {
            // Store symbols that are marked as protected to check if they are
            // actually in the container when the protection is dropped.
            #[cfg(debug_assertions)]
            self.protected_symbols
                .borrow_mut()
                .push(transmute::<SymbolRef<'_>, SymbolRef<'static>>(symbol.copy()));

            transmute::<SymbolRef<'_>, SymbolRef<'static>>(symbol.copy())
        }
    }
}

impl<T: ?Sized> Deref for ProtectedWriteGuard<'_, '_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.container
    }
}

impl<T: ?Sized> DerefMut for ProtectedWriteGuard<'_, '_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.container
    }
}

pub struct ProtectedReadGuard<'a, C> {
    reference: GcMutexReadGuard<'a, C>,
}

impl<'a, C> ProtectedReadGuard<'a, C> {
    fn new(reference: GcMutexReadGuard<'a, C>) -> Self {
        Self { reference }
    }
}

impl<'a, C: Transmutable> Deref for ProtectedReadGuard<'a, C> {
    type Target = C::Target<'a>;

    fn deref(&self) -> &Self::Target {
        // SAFETY: 'a is a shared borrow of the container, which therefore cannot be modified or
        // dropped while terms copied from it with lifetime 'a exist, so they remain protected.
        unsafe { self.reference.transmute_lifetime() }
    }
}
