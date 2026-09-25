// Authors: Maurice Laveaux, Flip van Spaendonck and Jan Friso Groote

use std::cell::Cell;
use std::error::Error;
use std::ops::Deref;
use std::ops::DerefMut;

use crate::BfSharedMutex;
use crate::BfSharedMutexReadGuard;
use crate::BfSharedMutexWriteGuard;

/// An extension of the [BfSharedMutex] that allows recursive read locking without deadlocks.
///
/// The recursion depth and call counters are stored in [`Cell`]s, so a `RecursiveLock` is
/// `!Sync` and has no `Clone`. To share the underlying data across threads, give each thread
/// its own `RecursiveLock` over a clone of the same mutex via
/// [`RecursiveLock::from_mutex`]`(shared.clone())`; the depth tracking is then per thread, as
/// the protocol requires. The call counters are likewise per instance, not global.
pub struct RecursiveLock<T> {
    inner: BfSharedMutex<T>,

    /// The number of times the current thread has read locked the mutex.
    recursive_depth: Cell<usize>,

    /// The number of calls to the write() method.
    write_calls: Cell<usize>,

    /// The number of calls to the read_recursive() method.
    read_recursive_calls: Cell<usize>,
}

impl<T> RecursiveLock<T> {
    /// Creates a new `RecursiveLock` with the given data.
    pub fn new(data: T) -> Self {
        RecursiveLock {
            inner: BfSharedMutex::new(data),
            recursive_depth: Cell::new(0),
            write_calls: Cell::new(0),
            read_recursive_calls: Cell::new(0),
        }
    }

    /// Creates a new `RecursiveLock` from an existing `BfSharedMutex`.
    pub fn from_mutex(mutex: BfSharedMutex<T>) -> Self {
        RecursiveLock {
            inner: mutex,
            recursive_depth: Cell::new(0),
            write_calls: Cell::new(0),
            read_recursive_calls: Cell::new(0),
        }
    }

    delegate::delegate! {
        to self.inner {
            #[cfg(not(loom))]
            pub fn data_ptr(&self) -> *const T;
            #[cfg(loom)]
            pub fn data_ptr(&self) -> loom::cell::ConstPtr<T>;
            pub fn is_locked(&self) -> bool;
            pub fn is_locked_exclusive(&self) -> bool;
        }
    }

    /// Acquires a write lock on the mutex.
    ///
    /// # Panics
    ///
    /// Panics when called inside a read or write section. In that case the underlying mutex
    /// would not wait for this thread's own lock, handing out `&mut T` while a `&T` or another
    /// `&mut T` is live.
    pub fn write(&self) -> Result<RecursiveLockWriteGuard<'_, T>, Box<dyn Error + '_>> {
        assert!(
            self.recursive_depth.get() == 0,
            "Cannot call write() inside an existing read or write section"
        );
        // Acquire the underlying lock before touching any bookkeeping, so a
        // failed acquisition leaves the recursive state untouched.
        let guard = self.inner.write()?;
        self.write_calls.set(self.write_calls.get() + 1);
        self.recursive_depth.set(1);
        Ok(RecursiveLockWriteGuard { mutex: self, guard })
    }

    /// Acquires a write lock on the mutex without blocking.
    ///
    /// # Panics
    ///
    /// Panics when called inside a read or write section. In that case the underlying mutex
    /// would not wait for this thread's own lock, handing out `&mut T` while a `&T` or another
    /// `&mut T` is live.
    pub fn try_write(&self) -> Result<Option<RecursiveLockWriteGuard<'_, T>>, Box<dyn Error + '_>> {
        assert!(
            self.recursive_depth.get() == 0,
            "Cannot call try_write() inside an existing read or write section"
        );
        // Acquire the underlying lock before touching any bookkeeping, so a
        // failed acquisition leaves the recursive state untouched.
        let guard = self.inner.try_write()?;

        self.write_calls.set(self.write_calls.get() + 1);

        if let Some(guard) = guard {
            self.recursive_depth.set(1);
            Ok(Some(RecursiveLockWriteGuard { mutex: self, guard }))
        } else {
            Ok(None)
        }
    }

    /// Acquires a read lock on the mutex.
    ///
    /// # Panics
    ///
    /// Panics when called inside a read or write section; the raw read lock is not reentrant.
    /// Use [`RecursiveLock::read_recursive`] instead.
    pub fn read(&self) -> Result<BfSharedMutexReadGuard<'_, T>, Box<dyn Error + '_>> {
        assert!(
            self.recursive_depth.get() == 0,
            "Cannot call read() inside an existing read or write section"
        );
        self.inner.read()
    }

    /// Acquires a read lock on the mutex, allowing for recursive read locking.
    ///
    /// May also be called inside a write section: the returned guard then borrows the write
    /// lock instead of acquiring the underlying mutex. While such a guard is alive, mutating
    /// through the [`RecursiveLockWriteGuard`] panics.
    pub fn read_recursive<'a>(&'a self) -> Result<RecursiveLockReadGuard<'a, T>, Box<dyn Error + 'a>> {
        if self.recursive_depth.get() == 0 {
            // Not yet holding a read lock: acquire the shared protocol lock without
            // materialising a guard, so the busy flag stays set until our own guard
            // releases it (via `create_read_guard_unchecked` on drop). The acquisition
            // happens before the bookkeeping is updated, so a failed acquisition leaves
            // the recursive state untouched.
            self.inner.acquire_shared()?;
            self.recursive_depth.set(1);
        } else {
            // Already holding a read lock, so just record the extra level.
            self.recursive_depth.set(self.recursive_depth.get() + 1);
        }
        self.read_recursive_calls.set(self.read_recursive_calls.get() + 1);
        Ok(RecursiveLockReadGuard {
            mutex: self,
            #[cfg(loom)]
            ptr: self.inner.data_ptr(),
        })
    }

    /// Returns the number of times `write()` has been called.
    pub fn write_call_count(&self) -> usize {
        self.write_calls.get()
    }

    /// Returns the number of times `read_recursive()` has been called.
    pub fn read_recursive_call_count(&self) -> usize {
        self.read_recursive_calls.get()
    }
}

#[must_use = "Dropping the guard unlocks the recursive lock immediately"]
pub struct RecursiveLockReadGuard<'a, T> {
    mutex: &'a RecursiveLock<T>,

    #[cfg(loom)]
    ptr: loom::cell::ConstPtr<T>,
}

impl<T> RecursiveLockReadGuard<'_, T> {
    /// Returns the read depth of the recursive lock.
    pub fn read_depth(&self) -> usize {
        self.mutex.recursive_depth.get()
    }
}

/// Allows dereferencing to the underlying object.
impl<T> Deref for RecursiveLockReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: This guard keeps the read lock (or the enclosing write lock) held, so only
        // shared access is handed out and the data pointer (an `UnsafeCell::get`) is non-null.
        #[cfg(not(loom))]
        unsafe {
            self.mutex.inner.data_ptr().as_ref().unwrap_unchecked()
        }

        #[cfg(loom)]
        unsafe {
            self.ptr.deref()
        }
    }
}

impl<T> Drop for RecursiveLockReadGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.recursive_depth.set(self.mutex.recursive_depth.get() - 1);
        if self.mutex.recursive_depth.get() == 0 {
            // SAFETY: The depth reached zero, so the outermost `read_recursive` forgot a real read
            // guard that still holds this thread's `busy` flag. Reconstructing and immediately
            // dropping a guard releases that flag exactly once, matching the forgotten guard.
            unsafe {
                let _ = self.mutex.inner.create_read_guard_unchecked();
            }
        }
    }
}

#[must_use = "Dropping the guard unlocks the recursive lock immediately"]
pub struct RecursiveLockWriteGuard<'a, T> {
    mutex: &'a RecursiveLock<T>,

    guard: BfSharedMutexWriteGuard<'a, T>,
}

/// Allows dereferencing to the underlying object.
impl<T> Deref for RecursiveLockWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // We hold the write guard, so immutable access is safe; defer to it rather than taking a
        // second loom borrow of the cell, which would conflict with the guard's mutable borrow.
        self.guard.deref()
    }
}

/// Allows dereferencing to the underlying object.
impl<T> DerefMut for RecursiveLockWriteGuard<'_, T> {
    /// # Panics
    ///
    /// Panics while a recursive read guard taken inside this write section is alive. Such a
    /// guard hands out `&T` derived from the data pointer, invisible to the borrow checker,
    /// so a `&mut T` would alias it.
    fn deref_mut(&mut self) -> &mut Self::Target {
        assert!(
            self.mutex.recursive_depth.get() == 1,
            "Cannot mutate through RecursiveLockWriteGuard while recursive read guards from its write section are alive"
        );
        // We hold the write guard exclusively and no recursive read guards exist, so mutable
        // access is safe.
        self.guard.deref_mut()
    }
}

impl<T> Drop for RecursiveLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        // Read guards taken with `read_recursive()` inside this write section borrow the write
        // lock: once this guard drops, the underlying mutex is released and their `&T` would be
        // unprotected. Panic instead of silently allowing that use-after-unlock.
        assert!(
            self.mutex.recursive_depth.get() == 1,
            "RecursiveLockWriteGuard dropped while recursive read guards from its write section are still alive"
        );
        self.mutex.recursive_depth.set(0);
    }
}

#[cfg(test)]
mod tests {
    use crate::BfSharedMutex;
    use crate::RecursiveLock;

    /// A `&mut T` obtained from `RecursiveLockWriteGuard::deref_mut` while no nested
    /// recursive read guard is alive must not alias a `&T` obtained from a
    /// subsequently-created nested `read_recursive` guard, even though nothing borrows
    /// the `&mut T` returned from `deref_mut` past the point where it is checked.
    ///
    /// `deref_mut`'s `recursive_depth == 1` assertion is only evaluated at call time; it
    /// does not prevent the caller from retaining the `&mut T` it returns while a nested
    /// `read_recursive` guard is created afterwards, which manufactures a live `&mut T`
    /// and a live `&T` to the same location with no `unsafe` on the caller's part.
    #[test]
    fn test_deref_mut_alias_survives_nested_read_recursive() {
        let lock = RecursiveLock::new(42i32);
        let mut write = lock.write().unwrap();

        // Obtain `&mut i32` through the safe API; depth is 1, so `deref_mut`'s assert
        // passes and this borrow is handed out.
        let data: &mut i32 = &mut *write;
        *data = 100;

        // Nothing about holding `data` prevents this: `read_recursive` only touches
        // `lock`, a value distinct from `write`/`data` as far as the borrow checker is
        // concerned.
        let read = lock.read_recursive().unwrap();

        // Read through the alias first, then write through `data` again: this ordering
        // is what actually exercises the aliasing violation under Stacked Borrows,
        // since a tag invalidated by a foreign access is only caught on its own next
        // use.
        assert_eq!(*read, 100);
        *data += 1;
        assert_eq!(*data, 101);
    }

    #[test]
    fn test_from_mutex() {
        let mutex = BfSharedMutex::new(100);
        let lock = RecursiveLock::from_mutex(mutex);
        assert_eq!(*lock.read().unwrap(), 100);
    }

    #[test]
    fn test_single_recursive_read() {
        let lock = RecursiveLock::new(42);
        let guard = lock.read_recursive().unwrap();
        assert_eq!(*guard, 42);
        assert_eq!(lock.recursive_depth.get(), 1);
    }

    #[test]
    fn test_nested_recursive_reads() {
        let lock = RecursiveLock::new(42);

        let guard1 = lock.read_recursive().unwrap();
        assert_eq!(*guard1, 42);
        assert_eq!(lock.recursive_depth.get(), 1);

        let guard2 = lock.read_recursive().unwrap();
        assert_eq!(*guard2, 42);
        assert_eq!(lock.recursive_depth.get(), 2);

        let guard3 = lock.read_recursive().unwrap();
        assert_eq!(*guard3, 42);
        assert_eq!(lock.recursive_depth.get(), 3);

        drop(guard3);
        assert_eq!(lock.recursive_depth.get(), 2);

        drop(guard2);
        assert_eq!(lock.recursive_depth.get(), 1);

        drop(guard1);
        assert_eq!(lock.recursive_depth.get(), 0);
    }

    #[test]
    fn test_read_recursive_inside_write() {
        let lock = RecursiveLock::new(42);
        let mut write = lock.write().unwrap();
        *write += 1;

        // Piggybacks on the write lock instead of acquiring the underlying mutex.
        let read = lock.read_recursive().unwrap();
        assert_eq!(*read, 43);
        assert_eq!(read.read_depth(), 2);
        drop(read);

        // Mutation is allowed again once the read guard is gone.
        *write += 1;
        assert_eq!(*write, 44);
        drop(write);

        assert_eq!(*lock.read().unwrap(), 44);
    }

    #[test]
    fn test_write_call_counter() {
        let lock = RecursiveLock::new(42);

        // Initially, the counter should be 0
        assert_eq!(lock.write_call_count(), 0);

        // After one write call, counter should be 1
        {
            let _guard = lock.write().unwrap();
            assert_eq!(lock.write_call_count(), 1);
        }

        // After another write call, counter should be 2
        {
            let _guard = lock.write().unwrap();
            assert_eq!(lock.write_call_count(), 2);
        }

        // Counter should remain 2
        assert_eq!(lock.write_call_count(), 2);
    }

    #[test]
    fn test_read_recursive_call_counter() {
        let lock = RecursiveLock::new(42);

        // Initially, the counter should be 0
        assert_eq!(lock.read_recursive_call_count(), 0);

        // After one read_recursive call, counter should be 1
        {
            let _guard = lock.read_recursive().unwrap();
            assert_eq!(lock.read_recursive_call_count(), 1);
        }

        // After another read_recursive call, counter should be 2
        {
            let _guard = lock.read_recursive().unwrap();
            assert_eq!(lock.read_recursive_call_count(), 2);
        }

        // Test nested recursive reads increment the counter
        {
            let _guard1 = lock.read_recursive().unwrap();
            assert_eq!(lock.read_recursive_call_count(), 3);

            let _guard2 = lock.read_recursive().unwrap();
            assert_eq!(lock.read_recursive_call_count(), 4);
        }

        // Counter should remain 4
        assert_eq!(lock.read_recursive_call_count(), 4);
    }

    #[test]
    #[cfg(loom)]
    fn test_loom_recursive_lock() {
        let mut builder = loom::model::Builder::new();
        // Mirrors the bound used for the underlying busy-forbidden mutex.
        builder.preemption_bound = Some(2);

        builder.check(|| {
            let mutex = BfSharedMutex::new(0usize);

            let threads: Vec<_> = (0..2)
                .map(|_| {
                    let mutex = mutex.clone();
                    loom::thread::spawn(move || {
                        // `RecursiveLock` is !Sync, so each thread wraps its own clone of the
                        // shared mutex; the recursion depth is then tracked per thread.
                        let lock = RecursiveLock::from_mutex(mutex);

                        // Nested recursive reads must observe a single consistent value and
                        // release the underlying read lock exactly once when the outermost
                        // guard drops.
                        {
                            let outer = lock.read_recursive().unwrap();
                            let inner = lock.read_recursive().unwrap();
                            assert_eq!(*outer, *inner);
                            assert_eq!(inner.read_depth(), 2);
                        }

                        // Exclusive access through the recursive write path.
                        *lock.write().unwrap() += 1;
                    })
                })
                .collect();

            for th in threads {
                th.join().unwrap();
            }
        });
    }

    #[test]
    fn test_both_counters() {
        let lock = RecursiveLock::new(42);

        // Initially, both counters should be 0
        assert_eq!(lock.write_call_count(), 0);
        assert_eq!(lock.read_recursive_call_count(), 0);

        // Call write and check counters
        {
            let _guard = lock.write().unwrap();
            assert_eq!(lock.write_call_count(), 1);
            assert_eq!(lock.read_recursive_call_count(), 0);
        }

        // Call read_recursive and check counters
        {
            let _guard = lock.read_recursive().unwrap();
            assert_eq!(lock.write_call_count(), 1);
            assert_eq!(lock.read_recursive_call_count(), 1);
        }

        // Call write again
        {
            let _guard = lock.write().unwrap();
            assert_eq!(lock.write_call_count(), 2);
            assert_eq!(lock.read_recursive_call_count(), 1);
        }

        // Call read_recursive multiple times
        {
            let _guard1 = lock.read_recursive().unwrap();
            let _guard2 = lock.read_recursive().unwrap();
            assert_eq!(lock.write_call_count(), 2);
            assert_eq!(lock.read_recursive_call_count(), 3);
        }
    }
}
