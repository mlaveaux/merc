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

    /// Set for the duration of a [`RecursiveLockWriteGuard::with_mut`] call, so that a
    /// [`RecursiveLock::read_recursive`] call made anywhere during that window (including
    /// reentrantly, from inside the closure) sees it and panics instead of manufacturing an
    /// aliased `&T` alongside the live `&mut T` the closure holds.
    mutating: Cell<bool>,

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
            mutating: Cell::new(false),
            write_calls: Cell::new(0),
            read_recursive_calls: Cell::new(0),
        }
    }

    /// Creates a new `RecursiveLock` from an existing `BfSharedMutex`.
    pub fn from_mutex(mutex: BfSharedMutex<T>) -> Self {
        RecursiveLock {
            inner: mutex,
            recursive_depth: Cell::new(0),
            mutating: Cell::new(false),
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
    ///
    /// # Panics
    ///
    /// Panics if called while a [`RecursiveLockWriteGuard::with_mut`] closure is currently
    /// executing (on this thread) — that closure holds a live `&mut T`, invisible to the
    /// borrow checker across this call, so handing out a `&T` here would alias it.
    pub fn read_recursive<'a>(&'a self) -> Result<RecursiveLockReadGuard<'a, T>, Box<dyn Error + 'a>> {
        assert!(
            !self.mutating.get(),
            "Cannot call read_recursive() while a RecursiveLockWriteGuard::with_mut call is in progress"
        );
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

impl<T> RecursiveLockWriteGuard<'_, T> {
    /// Grants scoped mutable access to the underlying value.
    ///
    /// Deliberately not a `DerefMut` impl returning a bare `&mut T`: such a reference has no
    /// `Drop` hook, so nothing could tell [`RecursiveLock::read_recursive`] that it is still
    /// live once handed out — the caller could stash it in a `let` binding, call
    /// `read_recursive()` afterwards (which only touches the separate `RecursiveLock` value,
    /// not this guard, so the borrow checker sees no conflict), and obtain a `&T` aliasing the
    /// still-held `&mut T`. Scoping mutation to a closure whose parameter cannot outlive the
    /// call, combined with the `mutating` flag `read_recursive` checks for the closure's whole
    /// dynamic extent (including a `read_recursive` call made reentrantly from inside `f`),
    /// closes both the "held across later statements" and the "called from within `f`" forms
    /// of that aliasing.
    ///
    /// # Panics
    ///
    /// Panics if a recursive read guard taken inside this write section (before this call) is
    /// still alive — such a guard hands out `&T` derived from the data pointer, invisible to
    /// the borrow checker, so a `&mut T` would alias it.
    pub fn with_mut<R>(&mut self, f: impl FnOnce(&mut T) -> R) -> R {
        assert!(
            self.mutex.recursive_depth.get() == 1,
            "Cannot mutate through RecursiveLockWriteGuard while recursive read guards from its write section are alive"
        );

        struct ResetOnDrop<'a>(&'a Cell<bool>);
        impl Drop for ResetOnDrop<'_> {
            fn drop(&mut self) {
                self.0.set(false);
            }
        }

        // Marks the mutation as in progress for the whole call to `f`, including any
        // `read_recursive()` call `f` reentrantly makes; reset even if `f` panics, so a
        // caught unwind does not leave the lock permanently unable to read_recursive().
        self.mutex.mutating.set(true);
        let _reset = ResetOnDrop(&self.mutex.mutating);

        // We hold the write guard exclusively and no recursive read guards exist, so mutable
        // access is safe; `mutating` additionally rules out one being created during `f`.
        f(self.guard.deref_mut())
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

    /// Regression test for the aliasing bug `with_mut` replaced `DerefMut` to close:
    /// calling `read_recursive()` *from inside* the closure passed to `with_mut` must panic
    /// (instead of silently succeeding and manufacturing a `&T` aliasing the closure's live
    /// `&mut T`, as the old `deref_mut`-based API allowed — see git history for the original
    /// repro, which relied on stashing `&mut *write` in a `let` binding across a later
    /// `read_recursive()` call; that call shape no longer type-checks at all now that
    /// `DerefMut` is gone, which is itself part of the fix).
    #[test]
    #[should_panic(
        expected = "Cannot call read_recursive() while a RecursiveLockWriteGuard::with_mut call is in progress"
    )]
    fn test_read_recursive_during_with_mut_panics() {
        let lock = RecursiveLock::new(42i32);
        let mut write = lock.write().unwrap();

        write.with_mut(|data| {
            *data = 100;
            // Reentrant call while `data` (the closure's `&mut i32`) is still logically
            // live: must panic rather than hand out an aliasing `&T`.
            let _ = lock.read_recursive().unwrap();
            *data += 1;
        });
    }

    /// The non-overlapping case `with_mut` is meant to keep working: mutate via a scoped
    /// closure, then take a recursive read only *after* that closure has returned (so the
    /// `mutating` flag is already clear) — must succeed and observe the mutation.
    #[test]
    fn test_read_recursive_after_with_mut_succeeds() {
        let lock = RecursiveLock::new(42i32);
        let mut write = lock.write().unwrap();

        write.with_mut(|data| *data = 100);

        let read = lock.read_recursive().unwrap();
        assert_eq!(*read, 100);
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
        write.with_mut(|v| *v += 1);

        // Piggybacks on the write lock instead of acquiring the underlying mutex.
        let read = lock.read_recursive().unwrap();
        assert_eq!(*read, 43);
        assert_eq!(read.read_depth(), 2);
        drop(read);

        // Mutation is allowed again once the read guard is gone.
        write.with_mut(|v| *v += 1);
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
                        lock.write().unwrap().with_mut(|v| *v += 1);
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
