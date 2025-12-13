use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ops::DerefMut;

use mcrl2_sys::atermpp::ffi::mcrl2_lock_exclusive;
use mcrl2_sys::atermpp::ffi::mcrl2_lock_shared;
use mcrl2_sys::atermpp::ffi::mcrl2_unlock_exclusive;
use mcrl2_sys::atermpp::ffi::mcrl2_unlock_shared;

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
pub struct BfTermPool<T: ?Sized> {
    object: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for BfTermPool<T> {}
unsafe impl<T: Send> Sync for BfTermPool<T> {}

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
        mcrl2_lock_shared();
        BfTermPoolRead {
            mutex: self,
            _marker: Default::default(),
        }
    }

    /// Provides write access to the underlying object.
    pub fn write(&'a self) -> BfTermPoolWrite<'a, T> {
        mcrl2_lock_exclusive();

        BfTermPoolWrite {
            mutex: self,
            _marker: Default::default(),
        }
    }

    /// Provides read access to the underlying object.
    ///
    /// # Safety
    ///
    /// Assumes that we are in an exclusive section.
    pub unsafe fn get(&'a self) -> &'a T {
        unsafe { &*self.object.get() }
    }

    /// Provides write access to the underlying object
    ///
    /// # Safety
    ///
    /// Provides mutable access given that other threads use [write] and [read]
    /// exclusively. If we are already in an exclusive context then lock can be
    /// set to false.
    pub unsafe fn write_exclusive(&'a self) -> BfTermPoolThreadWrite<'a, T> {
        // This is a lock shared, but assuming that only ONE thread uses this function.
        mcrl2_lock_shared();

        BfTermPoolThreadWrite {
            mutex: self,
            locked: true,
            _marker: Default::default(),
        }
    }
}

pub struct BfTermPoolRead<'a, T: ?Sized> {
    mutex: &'a BfTermPool<T>,
    _marker: PhantomData<&'a ()>,
}

impl<T: ?Sized> Deref for BfTermPoolRead<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // There can only be read guards.
        unsafe { &*self.mutex.object.get() }
    }
}

impl<T: ?Sized> Drop for BfTermPoolRead<'_, T> {
    fn drop(&mut self) {
        // If we leave the shared section and the counter is zero.
        mcrl2_unlock_shared();
    }
}

pub struct BfTermPoolWrite<'a, T: ?Sized> {
    mutex: &'a BfTermPool<T>,
    _marker: PhantomData<&'a ()>,
}

impl<T: ?Sized> Deref for BfTermPoolWrite<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // There can only be read guards.
        unsafe { &*self.mutex.object.get() }
    }
}

impl<T: ?Sized> DerefMut for BfTermPoolWrite<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // We are the only guard after `write()`, so we can provide mutable access to the underlying object.
        unsafe { &mut *self.mutex.object.get() }
    }
}

impl<T: ?Sized> Drop for BfTermPoolWrite<'_, T> {
    fn drop(&mut self) {
        mcrl2_unlock_exclusive();
    }
}

pub struct BfTermPoolThreadWrite<'a, T: ?Sized> {
    mutex: &'a BfTermPool<T>,
    locked: bool,
    _marker: PhantomData<&'a ()>,
}

impl<T: ?Sized> BfTermPoolThreadWrite<'_, T> {
    /// Unlocks the guard prematurely, but returns whether the shared section was actually left.
    pub fn unlock(&mut self) -> bool {
        if self.locked {
            self.locked = false;
            mcrl2_unlock_shared()
        } else {
            false
        }
    }
}

impl<T: ?Sized> Deref for BfTermPoolThreadWrite<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // There can only be read guards.
        unsafe { &*self.mutex.object.get() }
    }
}

impl<T: ?Sized> DerefMut for BfTermPoolThreadWrite<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // We are the only guard after `write()`, so we can provide mutable access to the underlying object.
        unsafe { &mut *self.mutex.object.get() }
    }
}

impl<T: ?Sized> Drop for BfTermPoolThreadWrite<'_, T> {
    fn drop(&mut self) {
        if self.locked {
            mcrl2_unlock_shared();
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
                    let value = *guard;
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
