use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ops::DerefMut;

use mcrl2_sys::atermpp::ffi::mcrl2_aterm_pool_lock_exclusive;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_pool_lock_shared;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_pool_unlock_exclusive;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_pool_unlock_shared;

/// Provides access to the mCRL2 busy forbidden protocol, where there
/// are thread local busy flags and one central storage for the forbidden
/// flags.
///
/// # Deadlock Warning
///
/// Care must be taken to avoid deadlocks since the FFI also uses the same flags.
/// In particular, **do not call FFI functions that may acquire busy/forbidden flags
/// while holding a lock (read or write) on a `BfTermPool`**. This can result in a
/// deadlock if the FFI function attempts to acquire a lock that is already held by
/// the current thread.
/// A `T` accessed exclusively through the mCRL2 busy/forbidden protocol.
///
/// # Safety contract
///
/// The protocol distinguishes two lock classes on the underlying C++ pool:
/// *shared* (`mcrl2_aterm_pool_lock_shared`/`unlock_shared`, a per-thread
/// "busy" flag; any number of threads may hold it concurrently) and
/// *exclusive* (`mcrl2_aterm_pool_lock_exclusive`/`unlock_exclusive`, which
/// waits for every thread's busy flag to clear). [`BfTermPool::read`] and
/// [`BfTermPool::write_exclusive`] both take the *shared* lock and hand out
/// `&T`/`&mut T` respectively; [`BfTermPool::write`] takes the *exclusive*
/// lock. Consequently `read()` and `write_exclusive()` do **not** exclude
/// each other — the invariant that makes this sound is not "the lock is
/// held" but:
///
/// **At most one thread may ever call `write_exclusive` (or hold its guard)
/// on a given `BfTermPool<T>` value at a time, and for the whole time some
/// thread's `write_exclusive` guard is live, no *other* thread may call
/// `read` or `get` on that same value.** (Two threads may freely call
/// `read`/`get` concurrently with each other, and `write` is genuinely
/// exclusive against everything, since it takes the C++ exclusive lock.)
/// It is the caller's responsibility to uphold this — the type itself does
/// not enforce it.
pub(crate) struct BfTermPool<T: ?Sized> {
    object: UnsafeCell<T>,
}

// SAFETY: the `UnsafeCell` is only ever accessed behind the busy/forbidden
// protocol, so sending the pool to another thread is sound whenever `T` itself
// is `Send`.
unsafe impl<T: Send> Send for BfTermPool<T> {}
// SAFETY: `T: Sync` lets `&T` be shared across threads, which is what `read`
// and `get` hand out; the busy/forbidden protocol above is what actually
// keeps those shared borrows from overlapping a `write`/`write_exclusive`
// `&mut T`, not this bound. Declaring `Sync` merely admits `&BfTermPool<T>`
// as shared state (e.g. behind an `Arc`); see the struct's safety contract
// for what a caller must still guarantee before calling `write_exclusive`.
unsafe impl<T: Send + Sync> Sync for BfTermPool<T> {}

impl<T> BfTermPool<T> {
    pub fn new(object: T) -> BfTermPool<T> {
        BfTermPool {
            object: UnsafeCell::new(object),
        }
    }
}

impl<'a, T: ?Sized> BfTermPool<T> {
    /// Provides read access to the underlying object.
    pub fn read(&'a self) -> BfTermPoolRead<'a, T> {
        mcrl2_aterm_pool_lock_shared();
        BfTermPoolRead {
            mutex: self,
            _marker: Default::default(),
        }
    }

    /// Provides write access to the underlying object.
    pub fn write(&'a self) -> BfTermPoolWrite<'a, T> {
        mcrl2_aterm_pool_lock_exclusive();

        BfTermPoolWrite {
            mutex: self,
            _marker: Default::default(),
        }
    }

    /// Provides read access to the underlying object without taking either
    /// C++ lock.
    ///
    /// # Safety
    ///
    /// Requires the calling thread to already be inside a section where no
    /// other thread can be concurrently reading or writing `self` — in
    /// practice, the calling thread must hold the C++ *exclusive* lock (as
    /// during a stop-the-world garbage collection callback), which per the
    /// class' safety contract guarantees no `write_exclusive` guard on this
    /// value is live on any thread, this one included. Guarantees: the
    /// returned `&'a T` is valid to dereference for `'a` and observes no
    /// concurrent mutation, provided the precondition holds.
    pub unsafe fn get(&'a self) -> &'a T {
        unsafe { &*self.object.get() }
    }

    /// Provides exclusive mutable access to the underlying object while only
    /// taking the C++ *shared* (busy) lock, not the exclusive one.
    ///
    /// # Safety
    ///
    /// Requires: for the entire lifetime of the returned guard, no thread
    /// other than the caller may call [`Self::read`], [`Self::get`] or
    /// `write_exclusive` again on this same `BfTermPool<T>` value (see the
    /// class' safety contract) — equivalently, `self` must be a protection
    /// set that only the calling thread ever touches outside of a
    /// stop-the-world collection. `ThreadTermPool` upholds this because each
    /// thread's `SharedProtectionSet`/`SharedContainerProtectionSet` is
    /// mutated (via `write_exclusive`) only from its own thread; the
    /// precondition is violated by code that calls `BfTermPool::read` (or
    /// `Debug`/`Markable::contains_term`/`Markable::len`, which call it) on
    /// *another* thread's set from outside a collection, e.g.
    /// `GlobalTermPool`'s `Debug` impl.
    ///
    /// Guarantees: the returned guard's `DerefMut` yields a `&mut T` valid
    /// until the guard is dropped or [`BfTermPoolThreadWrite::unlock`] is
    /// called, after which further mutation through it is a type error
    /// (the guard is consumed/no longer mutably borrowable), not UB.
    pub unsafe fn write_exclusive(&'a self) -> BfTermPoolThreadWrite<'a, T> {
        // This is a lock shared, but assuming that only ONE thread uses this function.
        mcrl2_aterm_pool_lock_shared();

        BfTermPoolThreadWrite {
            mutex: self,
            locked: true,
            _marker: Default::default(),
        }
    }
}

pub(crate) struct BfTermPoolRead<'a, T: ?Sized> {
    mutex: &'a BfTermPool<T>,
    _marker: PhantomData<&'a ()>,
}

impl<T: ?Sized> Deref for BfTermPoolRead<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: this guard holds the shared (read) lock, so concurrent readers
        // are allowed but no writer can be active; an immutable borrow is sound.
        unsafe { &*self.mutex.object.get() }
    }
}

impl<T: ?Sized> Drop for BfTermPoolRead<'_, T> {
    fn drop(&mut self) {
        // If we leave the shared section and the counter is zero.
        mcrl2_aterm_pool_unlock_shared();
    }
}

pub(crate) struct BfTermPoolWrite<'a, T: ?Sized> {
    mutex: &'a BfTermPool<T>,
    _marker: PhantomData<&'a ()>,
}

impl<T: ?Sized> Deref for BfTermPoolWrite<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: this guard holds the exclusive (write) lock, so it is the only
        // guard accessing the object; an immutable borrow is sound.
        unsafe { &*self.mutex.object.get() }
    }
}

impl<T: ?Sized> DerefMut for BfTermPoolWrite<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: this guard holds the exclusive (write) lock, so it is the only
        // guard accessing the object; a mutable borrow is sound.
        unsafe { &mut *self.mutex.object.get() }
    }
}

impl<T: ?Sized> Drop for BfTermPoolWrite<'_, T> {
    fn drop(&mut self) {
        mcrl2_aterm_pool_unlock_exclusive();
    }
}

pub(crate) struct BfTermPoolThreadWrite<'a, T: ?Sized> {
    mutex: &'a BfTermPool<T>,
    locked: bool,
    _marker: PhantomData<&'a ()>,
}

impl<T: ?Sized> BfTermPoolThreadWrite<'_, T> {
    /// Unlocks the guard prematurely, but returns whether the shared section was actually left.
    pub fn unlock(&mut self) -> bool {
        if self.locked {
            self.locked = false;
            mcrl2_aterm_pool_unlock_shared()
        } else {
            false
        }
    }
}

impl<T: ?Sized> Deref for BfTermPoolThreadWrite<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: this guard is created under the `write_exclusive` contract that
        // only a single thread uses it, so it is the sole accessor; an immutable
        // borrow is sound.
        unsafe { &*self.mutex.object.get() }
    }
}

impl<T: ?Sized> DerefMut for BfTermPoolThreadWrite<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: this guard is created under the `write_exclusive` contract that
        // only a single thread uses it, so it is the sole accessor; a mutable
        // borrow is sound.
        unsafe { &mut *self.mutex.object.get() }
    }
}

impl<T: ?Sized> Drop for BfTermPoolThreadWrite<'_, T> {
    fn drop(&mut self) {
        if self.locked {
            mcrl2_aterm_pool_unlock_shared();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use merc_utilities::random_test_threads;

    use crate::BfTermPool;

    #[test]
    fn test_random_busy_forbidden_threaded() {
        let pool = Arc::new(BfTermPool::new(0u64));

        random_test_threads(
            100,
            2,
            || pool.clone(),
            |_id, pool| {
                // Test read lock
                {
                    let guard = pool.read();
                    let _value = *guard;
                }

                // Test write lock and increment
                {
                    let mut guard = pool.write();
                    *guard += 1;
                }
            },
        );

        // Verify final count
        let guard = pool.read();
        let final_value = *guard;
        assert_eq!(final_value, 200); // 100 iterations * 2 threads
    }
}
