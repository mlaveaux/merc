use std::cell::UnsafeCell;
use std::mem::ManuallyDrop;
use std::ops::Deref;
use std::ops::DerefMut;

use crate::storage::GlobalTermPoolGuard;
use crate::storage::THREAD_TERM_POOL;

/// A mutex that prevents garbage collection by holding a shared read lock on
/// the [super::GlobalTermPool] for the duration of the guard's lifetime.
/// Returns a [GcMutexGuard] on access.
///
/// # Panics
///
/// The `GcMutex` returns guards that are tied to the thread-local storage of
/// [crate::storage::THREAD_TERM_POOL]. This means that the guard must be
/// dropped before this thread-local storage is dropped, or it will panic.
pub(crate) struct GcMutex<T> {
    inner: UnsafeCell<T>,
}

// SAFETY:
//
// `GcMutex<T> { inner: UnsafeCell<T> }`. `UnsafeCell<T>` is `!Sync` unconditionally (regardless
// of `T`) -- that is precisely why a type providing its own synchronization around an
// `UnsafeCell` must reassert `Sync` explicitly. Moving a `GcMutex<T>` moves the one `T` it owns,
// so `Send` needs only `T: Send`.
//
// - `Send for GcMutex<T> where T: Send`: there is exactly one owner of the `T` inside at a time;
//   transferring that ownership to another thread is sound whenever `T` itself is `Send`.
// - `Sync for GcMutex<T> where T: Send + Sync`: `&GcMutex<T>` lets any thread that reaches it
//   obtain a `&T` via `GcMutex::lock` (any number of concurrent readers, requiring `T: Sync`) or
//   a `&mut T` via `GcMutex::lock_mut` (requiring `T: Send`, since the thread that ends up
//   mutating it may differ from the one that created the value). `lock_mut` takes `&mut self`,
//   so the borrow checker already guarantees at most one live `GcMutexGuard` per `GcMutex` and
//   excludes it from overlapping with any `GcMutexReadGuard` obtained from the same handle. The
//   two guard types between them enforce the same shared-xor-exclusive discipline a
//   `Sync + RwLock`-style type normally provides, which is what makes reasserting `Sync` here
//   sound.
unsafe impl<T: Send> Send for GcMutex<T> {}
unsafe impl<T: Send + Sync> Sync for GcMutex<T> {}

impl<T> GcMutex<T> {
    pub fn new(value: T) -> GcMutex<T> {
        GcMutex {
            inner: UnsafeCell::new(value),
        }
    }

    /// Provides shared access to the underlying value, returning a [GcMutexReadGuard].
    ///
    /// The returned guard holds a read lock on the global term pool, preventing
    /// garbage collection for its lifetime. It only provides immutable access.
    pub fn lock(&self) -> GcMutexReadGuard<'_, T> {
        GcMutexReadGuard {
            mutex: self,
            guard: ManuallyDrop::new(THREAD_TERM_POOL.with(|tp| unsafe {
                std::mem::transmute::<_, GlobalTermPoolGuard<'_>>(
                    tp.term_pool().read_recursive().expect("Lock poisoned!"),
                )
            })),
        }
    }

    /// Provides exclusive mutable access to the underlying value, returning a [GcMutexGuard].
    ///
    /// Takes `&mut self` so only one mutable guard can exist at a time; the borrow
    /// checker enforces that no other guard (read or write) coexists.
    pub fn lock_mut(&mut self) -> GcMutexGuard<'_, T> {
        GcMutexGuard {
            mutex: self,
            guard: ManuallyDrop::new(THREAD_TERM_POOL.with(|tp| unsafe {
                std::mem::transmute::<_, GlobalTermPoolGuard<'_>>(
                    tp.term_pool().read_recursive().expect("Lock poisoned!"),
                )
            })),
        }
    }
}

/// A read-only guard produced by [GcMutex::lock].  Holds a shared read lock on
/// the global term pool for its lifetime, preventing garbage collection.
pub(crate) struct GcMutexReadGuard<'a, T> {
    mutex: &'a GcMutex<T>,

    /// Only used to avoid garbage collection, will be released on drop.
    guard: ManuallyDrop<GlobalTermPoolGuard<'a>>,
}

impl<T> Deref for GcMutexReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.inner.get() }
    }
}

impl<T> Drop for GcMutexReadGuard<'_, T> {
    fn drop(&mut self) {
        // Access the guard only through `THREAD_TERM_POOL`. If the thread-local pool is already
        // destroyed, `with` panics before we dereference the guard (which borrows the
        // thread-local term pool), turning a would-be use-after-free into a deterministic panic.
        THREAD_TERM_POOL.with(|tp| {
            if self.guard.read_depth() == 1 {
                unsafe { tp.trigger_delayed_garbage_collection(&mut self.guard) }
            } else {
                unsafe { ManuallyDrop::drop(&mut self.guard) };
            }
        });
    }
}

/// A read-write guard produced by [GcMutex::lock_mut].  Provides both
/// [Deref] and [DerefMut].  Because [GcMutex::lock_mut] takes `&mut self`,
/// the borrow checker guarantees this is the only live guard for its lifetime.
pub(crate) struct GcMutexGuard<'a, T> {
    mutex: &'a GcMutex<T>,

    /// Only used to avoid garbage collection, will be released on drop.
    guard: ManuallyDrop<GlobalTermPoolGuard<'a>>,
}

impl<T> Deref for GcMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.mutex.inner.get() }
    }
}

impl<T> DerefMut for GcMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.mutex.inner.get() }
    }
}

impl<T> Drop for GcMutexGuard<'_, T> {
    fn drop(&mut self) {
        // Access the guard only through `THREAD_TERM_POOL`. If the thread-local pool is already
        // destroyed, `with` panics before we dereference the guard (which borrows the
        // thread-local term pool), turning a would-be use-after-free into a deterministic panic.
        THREAD_TERM_POOL.with(|tp| {
            if self.guard.read_depth() == 1 {
                // If this is the last guard, we can trigger garbage collection when it was delayed earlier.
                unsafe { tp.trigger_delayed_garbage_collection(&mut self.guard) }
            } else {
                // Just drop the guard
                unsafe { ManuallyDrop::drop(&mut self.guard) };
            }
        });
    }
}
