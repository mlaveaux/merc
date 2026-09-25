// Authors: Maurice Laveaux, Flip van Spaendonck and Jan Friso Groote

use std::error::Error;
use std::fmt::Debug;
use std::ops::Deref;
use std::ops::DerefMut;

#[cfg(not(loom))]
mod inner {
    pub(super) use std::cell::UnsafeCell;
    pub(super) use std::hint::spin_loop;
    pub(super) use std::sync::Arc;
    pub(super) use std::sync::Mutex;
    pub(super) use std::sync::MutexGuard;
    pub(super) use std::sync::TryLockError;
    pub(super) use std::sync::atomic::AtomicBool;
    pub(super) use std::sync::atomic::Ordering;
    pub(super) use std::sync::atomic::fence;
}

// We replace the standard implementation by loom's implementation.
#[cfg(loom)]
mod inner {
    pub use std::mem::ManuallyDrop;
    pub use std::sync::TryLockError;

    pub use loom::cell::UnsafeCell;
    pub use loom::hint::spin_loop;
    pub use loom::sync::Arc;
    pub use loom::sync::Mutex;
    pub use loom::sync::MutexGuard;
    pub use loom::sync::atomic::AtomicBool;
    pub use loom::sync::atomic::Ordering;
    pub use loom::sync::atomic::fence;
}

use inner::*;

use crossbeam_utils::CachePadded;

/// A readers-writer lock implementation based on the busy-forbidden protocol.
///
/// Unlike a regular [`std::sync::Mutex`], this type is `Send` but not `Sync`: every thread must
/// hold its own clone, and clones of the same mutex give shared access through `read` and
/// exclusive access through `write`.
///
/// # Poisoning
///
/// A panic inside a `write` section poisons the internal registration mutex. After that, every
/// `read` and `write` call returns `Err`; the poison is only cleared once enough clones are
/// dropped.
pub struct BfSharedMutex<T> {
    /// The local control bits of each instance.
    control: Arc<CachePadded<SharedMutexControl>>,

    /// Index into the `other` table.
    index: usize,

    /// Information shared between all clones.
    shared: Arc<CachePadded<SharedData<T>>>,
}

// SAFETY: `BfSharedMutex<T>` is not auto-`Send` because `shared: Arc<CachePadded<SharedData<T>>>`
// contains `object: UnsafeCell<T>`, which is never `Sync`, so `Arc<..UnsafeCell<T>..>` is not
// auto-`Send` regardless of `T`.
//
// Contract discharged by this impl, given `T: Send + Sync`:
//   - `control: Arc<CachePadded<SharedMutexControl>>` is unique per clone (freshly allocated in
//     `new`/`clone`, never shared with another clone's `control`), so moving it only transfers
//     bits that the destination thread already exclusively owns; this field imposes no bound
//     on `T`.
//   - `shared: Arc<CachePadded<SharedData<T>>>` is aliased by every clone of this mutex, and its
//     `object: UnsafeCell<T>` is dereferenced as `&T` by any thread holding a live read guard and
//     as `&mut T` by any thread holding the live write guard, and dropped (`SharedData::drop`) by
//     whichever thread drops the last `Arc` — not necessarily the thread that produced `T`.
//     `T: Send` makes that final drop (and any other thread taking ownership of `T`) sound;
//     `T: Sync` makes a `&T` produced on one thread and observed on another sound.
//   - `index: usize` is `Copy` data with no aliasing to preserve.
// So after the move, every field the destination thread now owns is safe to drive from there.
unsafe impl<T: Send + Sync> Send for BfSharedMutex<T> {}

/// The busy and forbidden flags used to implement the protocol.
#[derive(Default)]
struct SharedMutexControl {
    busy: AtomicBool,
    forbidden: AtomicBool,
}

/// The shared data between all instances of the shared mutex.
struct SharedData<T> {
    /// The object that is being protected.
    object: UnsafeCell<T>,

    /// The list of all the shared mutex instances.
    other: Mutex<Vec<Option<Arc<CachePadded<SharedMutexControl>>>>>,
}

impl<T> BfSharedMutex<T> {
    /// Constructs a new shared mutex for protecting access to the given object.
    pub fn new(object: T) -> Self {
        let control = Arc::new(CachePadded::new(SharedMutexControl::default()));

        Self {
            control: control.clone(),
            shared: Arc::new(CachePadded::new(SharedData {
                object: UnsafeCell::new(object),
                other: Mutex::new(vec![Some(control.clone())]),
            })),
            index: 0,
        }
    }
}

impl<T> Clone for BfSharedMutex<T> {
    fn clone(&self) -> Self {
        // Register a new instance in the other list
        let control = Arc::new(CachePadded::new(SharedMutexControl::default()));

        let mut other = self.shared.other.lock().expect("Failed to lock mutex");
        let index = match other.iter().position(|slot| slot.is_none()) {
            Some(index) => {
                // Reuse an empty slot if available.
                other[index] = Some(control.clone());
                index
            }
            None => {
                other.push(Some(control.clone()));
                other.len() - 1
            }
        };

        Self {
            control,
            index,
            shared: self.shared.clone(),
        }
    }
}

impl<T> Drop for BfSharedMutex<T> {
    fn drop(&mut self) {
        // A panic inside a write section poisons this mutex, since the write guard holds it.
        // Deregistering is still safe then, and panicking in Drop during unwinding would abort.
        let mut other = self
            .shared
            .other
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Remove ourselves from the table.
        other[self.index] = None;

        // Trim trailing None slots to keep the vec compact.
        while other.last().is_some_and(|slot| slot.is_none()) {
            other.pop();
        }
    }
}

/// The guard object for exclusive access to the underlying object.
#[must_use = "Dropping the guard unlocks the shared mutex immediately"]
pub struct BfSharedMutexWriteGuard<'a, T> {
    #[allow(dead_code)]
    mutex: &'a BfSharedMutex<T>,

    guard: MutexGuard<'a, Vec<Option<Arc<CachePadded<SharedMutexControl>>>>>,

    /// When loom is enabled, we store a write reference tracked by Loom.
    #[cfg(loom)]
    ptr: ManuallyDrop<loom::cell::MutPtr<T>>,
}

/// Allows dereferencing the underlying object.
impl<T> Deref for BfSharedMutexWriteGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: We are the only guard after `write()`, so immutable access to the underlying
        // object is sound.
        #[cfg(not(loom))]
        unsafe {
            &*self.mutex.shared.object.get()
        }

        #[cfg(loom)]
        unsafe {
            self.ptr.deref().deref()
        }
    }
}

impl<T> DerefMut for BfSharedMutexWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: We are the only guard after `write()`, so exclusive mutable access to the
        // underlying object is sound.
        #[cfg(not(loom))]
        unsafe {
            &mut *self.mutex.shared.object.get()
        }

        #[cfg(loom)]
        unsafe {
            self.ptr.deref().deref()
        }
    }
}

impl<T> Drop for BfSharedMutexWriteGuard<'_, T> {
    fn drop(&mut self) {
        // End Loom's write tracking before releasing the protocol.
        #[cfg(loom)]
        unsafe {
            ManuallyDrop::drop(&mut self.ptr);
        }

        // Allow other threads to acquire access to the shared mutex.
        for control in self.guard.iter().flatten() {
            control.forbidden.store(false, Ordering::Release);
        }

        // The mutex guard is then dropped here.
    }
}

// SAFETY: `BfSharedMutexWriteGuard<'_, T>` holds `mutex: &'a BfSharedMutex<T>`, and
// `BfSharedMutex<T>` is deliberately not `Sync` (see its doc comment), so `&'a BfSharedMutex<T>`
// is neither auto-`Send` nor auto-`Sync`, and the field blocks auto-deriving `Sync` for the
// guard even though `T: Sync` is available.
//
// Contract discharged by this impl, given `T: Sync`:
//   - The only operation reachable through `&BfSharedMutexWriteGuard` is `Deref::deref`, which
//     reads the (invariant, set-once-at-construction) `mutex`/`guard` pointers and returns `&T`;
//     no method reachable via `&self` mutates `SharedMutexControl` state or the `other` table.
//   - Concurrent calls to `deref` from multiple threads, each holding `&Guard`, therefore only
//     ever produce concurrently-live `&T` values to the one object this guard already has
//     exclusive access to under the protocol; `T: Sync` is exactly the bound that makes sharing
//     those `&T` values across threads sound.
//   - `Drop` (which releases the `forbidden` flags) takes `&mut self`, so it is unreachable
//     through a shared `&Guard` and is unaffected by this impl.
unsafe impl<T: Sync> Sync for BfSharedMutexWriteGuard<'_, T> {}

#[must_use = "Dropping the guard unlocks the shared mutex immediately"]
pub struct BfSharedMutexReadGuard<'a, T> {
    mutex: &'a BfSharedMutex<T>,

    /// When loom is enabled, we store a read reference tracked by Loom.
    #[cfg(loom)]
    ptr: ManuallyDrop<loom::cell::ConstPtr<T>>,
}

// SAFETY: `BfSharedMutexReadGuard<'_, T>` holds `mutex: &'a BfSharedMutex<T>`, which — like
// `BfSharedMutexWriteGuard` above — is neither auto-`Send` nor auto-`Sync` because
// `BfSharedMutex<T>` is deliberately `!Sync`; that field is what blocks auto-deriving `Sync`
// for this guard.
//
// Contract discharged by this impl, given `T: Sync`:
//   - The only operation reachable through `&BfSharedMutexReadGuard` is `Deref::deref`,
//     returning `&T` from the same (already read-locked) object every other live read guard on
//     this mutex also derefs to; `T: Sync` licenses sharing that `&T` across the threads that
//     concurrently call `deref` through their own `&Guard`.
//   - `Drop` (which clears this instance's `busy` flag) takes `&mut self` and so cannot be
//     invoked through a shared `&Guard`, and is unaffected by this impl.
unsafe impl<T: Sync> Sync for BfSharedMutexReadGuard<'_, T> {}

/// Allows dereferencing the underlying object.
impl<T> Deref for BfSharedMutexReadGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: There can only be shared guards while this guard is alive, so immutable
        // access to the object is sound.
        #[cfg(not(loom))]
        unsafe {
            &*self.mutex.shared.object.get()
        }

        #[cfg(loom)]
        unsafe {
            self.ptr.deref().deref()
        }
    }
}

impl<T> Drop for BfSharedMutexReadGuard<'_, T> {
    fn drop(&mut self) {
        debug_assert!(
            self.mutex.control.busy.load(Ordering::Relaxed),
            "Cannot unlock shared lock that was not acquired"
        );

        // End Loom's read tracking before releasing the protocol.
        #[cfg(loom)]
        unsafe {
            ManuallyDrop::drop(&mut self.ptr);
        }

        // Release is sufficient, this synchronises the writes of this thread with writer.
        self.mutex.control.busy.store(false, Ordering::Release);
    }
}

impl<T> BfSharedMutex<T> {
    /// Provides read access to the underlying object, allowing multiple immutable references to it.
    ///
    /// # Panics
    ///
    /// Panics when called while this instance already holds a read guard. Reentrant reads would
    /// corrupt the `busy` flag.
    pub fn read<'a>(&'a self) -> Result<BfSharedMutexReadGuard<'a, T>, Box<dyn Error + 'a>> {
        self.acquire_shared()?;

        // We now have immutable access to the object due to the protocol.
        Ok(BfSharedMutexReadGuard {
            mutex: self,
            #[cfg(loom)]
            ptr: ManuallyDrop::new(self.shared.object.get()),
        })
    }

    /// Runs the busy-forbidden protocol to acquire shared (read) access, setting this instance's
    /// `busy` flag, without materialising a guard.
    ///
    /// The caller becomes responsible for eventually clearing the `busy` flag, e.g. by dropping a
    /// guard reconstructed with [`Self::create_read_guard_unchecked`]. Unlike [`Self::read`], this
    /// takes no data borrow of the cell, so under loom it can be paired with `mem::forget`-style
    /// ownership transfer without leaking a read borrow.
    ///
    /// # Panics
    ///
    /// Panics if this instance already holds a read guard.
    pub(crate) fn acquire_shared(&self) -> Result<(), Box<dyn Error + '_>> {
        assert!(
            !self.control.busy.load(Ordering::Relaxed),
            "Cannot acquire read access again inside a reader section"
        );

        self.control.busy.store(true, Ordering::Relaxed);
        fence(Ordering::SeqCst);
        while self.control.forbidden.load(Ordering::Acquire) {
            // Signal the writer that this thread is no longer busy, allowing it to make progress.
            self.control.busy.store(false, Ordering::Relaxed);

            // For loom with spin locks we must ensure that other threads can make progress for fairness.
            #[cfg(loom)]
            loom::thread::yield_now();

            // Wait for the mutex of the writer.
            let _guard = self.shared.other.lock()?;

            // Allow another thread to acquire the lock.
            #[cfg(loom)]
            loom::thread::yield_now();

            self.control.busy.store(true, Ordering::Relaxed);

            // The busy store must become visible before the forbidden load is performed, exactly
            // as on the initial acquisition above.
            fence(Ordering::SeqCst);
        }

        Ok(())
    }

    /// Creates a new `BfSharedMutexReadGuard` without checking if the lock is held.
    ///
    /// # Safety
    ///
    /// Formally, at the call site there must exist a single outstanding "logical read
    /// acquisition" `A` on `self` — established by a prior call to [`Self::acquire_shared`] (or
    /// to [`Self::read`] followed by `mem::forget`ing the returned guard) on this same clone,
    /// for which no [`BfSharedMutexReadGuard`] currently exists (any guard previously
    /// reconstructed from `A` must already have been dropped or `mem::forget`en). Equivalently:
    /// `self.control.busy` must be `true`, and this call must be the *unique* reconstruction of
    /// a guard for the acquisition that set it so.
    ///
    /// Requires:
    ///   - `self.control.busy.load(Relaxed) == true` for the entire lifetime of the returned
    ///     guard (this call does not itself set `busy`, unlike [`Self::read`]).
    ///   - No writer guard for `self`'s mutex (i.e. `self.control.forbidden`) transitions to
    ///     forbidding a write while this guard is alive that this guard's own protocol
    ///     compliance is not already accounting for — in practice, satisfied automatically by
    ///     `A` having been established through `acquire_shared`.
    ///   - Exactly one live guard (this one) accounts for acquisition `A` at a time; producing a
    ///     second live guard for the same `A` without first retiring this one (drop, or
    ///     `mem::forget` followed by a fresh acquisition) is undefined behaviour, since `Drop`
    ///     would then clear `busy` twice for what the protocol treats as one acquisition,
    ///     letting a concurrent writer observe `busy == false` while a live `&T` from the
    ///     still-outstanding guard remains reachable.
    ///
    /// Guarantees: the returned guard derefs to `&T` exactly as [`Self::read`]'s guard would —
    /// i.e. to a snapshot consistent with holding the busy-forbidden protocol's read access —
    /// and its `Drop` clears `self.control.busy`, retiring acquisition `A`.
    pub unsafe fn create_read_guard_unchecked(&self) -> BfSharedMutexReadGuard<'_, T> {
        BfSharedMutexReadGuard {
            mutex: self,
            #[cfg(loom)]
            ptr: ManuallyDrop::new(self.shared.object.get()),
        }
    }

    /// Returns a raw pointer to the underlying data.
    ///
    /// This is useful when combined with `mem::forget` to hold a lock without
    /// the need to maintain a [`BfSharedMutexReadGuard`] or [`BfSharedMutexWriteGuard`] object
    /// alive, for example when dealing with FFI.
    ///
    /// # Safety
    ///
    /// You must ensure that there are no data races when dereferencing the
    /// returned pointer, for example if the current thread logically owns a
    /// [`BfSharedMutexReadGuard`] or [`BfSharedMutexWriteGuard`] but that guard has been discarded
    /// using `mem::forget`.
    #[cfg(not(loom))]
    pub fn data_ptr(&self) -> *mut T {
        self.shared.object.get()
    }

    #[cfg(loom)]
    pub fn data_ptr(&self) -> loom::cell::ConstPtr<T> {
        self.shared.object.get()
    }

    /// Provides write access to the underlying object, blocking until every other clone's read
    /// or write access has ended so only one mutable reference exists at a time.
    pub fn write<'a>(&'a self) -> Result<BfSharedMutexWriteGuard<'a, T>, Box<dyn Error + 'a>> {
        let other = self.shared.other.lock()?;
        Ok(self.acquire_exclusive(other))
    }

    /// Attempts to acquire write access without blocking on the inner registration mutex.
    ///
    /// Returns `Ok(None)` when that inner mutex is already locked by another thread, which
    /// happens while another clone is acquiring or releasing a write lock (or is otherwise
    /// inside the protocol's critical section). This lets a caller bail out instead of waiting,
    /// for example to skip garbage collection when another thread is already performing it.
    pub fn try_write<'a>(&'a self) -> Result<Option<BfSharedMutexWriteGuard<'a, T>>, Box<dyn Error + 'a>> {
        let other = match self.shared.other.try_lock() {
            Ok(other) => other,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(err)) => return Err(Box::new(err)),
        };

        Ok(Some(self.acquire_exclusive(other)))
    }

    /// Completes the busy-forbidden protocol for exclusive access once the inner registration
    /// mutex has been locked, forbidding all instances and waiting for any busy readers to exit.
    fn acquire_exclusive<'a>(
        &'a self,
        other: MutexGuard<'a, Vec<Option<Arc<CachePadded<SharedMutexControl>>>>>,
    ) -> BfSharedMutexWriteGuard<'a, T> {
        debug_assert!(
            !self.control.busy.load(Ordering::Relaxed),
            "Can only exclusive lock outside of a shared lock, no upgrading!"
        );
        debug_assert!(
            !self.control.forbidden.load(Ordering::Relaxed),
            "Can not acquire exclusive lock inside of exclusive section"
        );

        // Make all instances wait due to forbidden access.
        for control in other.iter().flatten() {
            debug_assert!(
                !control.forbidden.load(Ordering::Relaxed),
                "Other instance is already forbidden, this cannot happen"
            );

            control.forbidden.store(true, Ordering::Relaxed);
        }

        fence(Ordering::SeqCst);

        // Wait for the instances to exit their busy status.
        for (index, option) in other.iter().enumerate() {
            if index != self.index
                && let Some(object) = option
            {
                // We just synchronize with the busy store of the other instances.
                while object.busy.load(Ordering::Acquire) {
                    spin_loop();
                }
            }
        }

        // We now have exclusive access to the object according to the protocol
        BfSharedMutexWriteGuard {
            mutex: self,
            guard: other,
            #[cfg(loom)]
            ptr: ManuallyDrop::new(self.shared.object.get_mut()),
        }
    }

    /// Returns whether this clone currently holds a read lock. Other clones may independently
    /// hold their own.
    pub fn is_locked(&self) -> bool {
        self.control.busy.load(Ordering::Relaxed)
    }

    /// Returns whether this clone is forbidden from acquiring a read lock, which indicates that
    /// another clone is holding or acquiring a write lock.
    pub fn is_locked_exclusive(&self) -> bool {
        self.control.forbidden.load(Ordering::Relaxed)
    }

    /// Obtain mutable access to the object without locking.
    ///
    /// Returns `None` when other clones of this shared mutex exist, since those clones can
    /// concurrently hold read or write guards on the shared object.
    pub fn get_mut(&mut self) -> Option<&mut T> {
        {
            let other = self.shared.other.lock().expect("Failed to lock mutex");
            if other.iter().flatten().count() != 1 {
                return None;
            }
        }

        // SAFETY: The registration table contains only this instance, so no other handle exists
        // (guards borrow their handle, and new clones can only be created from a handle). Holding
        // `&mut self` therefore guarantees no guard is alive and none can be created.
        #[cfg(not(loom))]
        unsafe {
            Some(&mut *self.shared.object.get())
        }
        #[cfg(loom)]
        unsafe {
            Some(self.shared.object.get_mut().with(|ptr| &mut *ptr))
        }
    }
}

impl<T: Debug> Debug for BfSharedMutex<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Recover from poisoning so that formatting (often invoked from logging or while
        // already panicking) never panics itself.
        let other = self
            .shared
            .other
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        f.debug_map()
            .entry(&"busy", &self.control.busy.load(Ordering::Relaxed))
            .entry(&"forbidden", &self.control.forbidden.load(Ordering::Relaxed))
            .entry(&"index", &self.index)
            .entry(&"len(other)", &other.len())
            .finish()?;

        writeln!(f)?;
        writeln!(f, "other values: [")?;
        for control in other.iter().flatten() {
            f.debug_map()
                .entry(&"busy", &control.busy.load(Ordering::Relaxed))
                .entry(&"forbidden", &control.forbidden.load(Ordering::Relaxed))
                .finish()?;
            writeln!(f)?;
        }

        writeln!(f, "]")
    }
}

/// A `BfSharedMutex` held as a single global instance; call `share` to obtain a clone with
/// read/write access.
pub struct GlobalBfSharedMutex<T> {
    /// The shared mutex that is used to protect the global data.
    shared_mutex: BfSharedMutex<T>,
}

impl<T> GlobalBfSharedMutex<T> {
    /// Constructs a new global shared mutex for protecting access to the given object.
    pub fn new(object: T) -> Self {
        Self {
            shared_mutex: BfSharedMutex::new(object),
        }
    }

    /// Returns a clone of the global shared mutex, which allows writing and reading.
    pub fn share(&self) -> BfSharedMutex<T> {
        self.shared_mutex.clone()
    }
}

// SAFETY: `GlobalBfSharedMutex<T>` has one field, `shared_mutex: BfSharedMutex<T>`; it is not
// auto-`Send`/auto-`Sync` for exactly the reasons `BfSharedMutex<T>` itself is not (see that
// type's `unsafe impl Send` above): its `shared: Arc<..UnsafeCell<T>..>` field is never
// auto-`Sync`, and `BfSharedMutex<T>` is deliberately never `Sync` at all.
//
// Contract discharged by this impl, given `T: Send + Sync`:
//   - `Send`: moving `GlobalBfSharedMutex<T>` moves its single `BfSharedMutex<T>` field, which
//     `unsafe impl<T: Send + Sync> Send for BfSharedMutex<T>` already establishes is sound to
//     move under this same bound.
//   - `Sync`: the only operation reachable through `&GlobalBfSharedMutex<T>` is `share()`, which
//     takes `&self` and returns `self.shared_mutex.clone()`. `BfSharedMutex::clone` serialises
//     all registration bookkeeping (allocating a fresh `control`, inserting it into the `other`
//     table) behind `shared.other`'s own `Mutex`, so concurrent `share()` calls from multiple
//     threads, each racing on that inner lock, produce distinct, non-aliasing `BfSharedMutex<T>`
//     clones with no data race on `GlobalBfSharedMutex`'s own state.
unsafe impl<T: Send + Sync> Send for GlobalBfSharedMutex<T> {}
unsafe impl<T: Send + Sync> Sync for GlobalBfSharedMutex<T> {}

#[cfg(kani)]
mod verification {
    use super::*;

    /// Exercises `create_read_guard_unchecked` along its documented pairing with
    /// `acquire_shared`: after `acquire_shared` sets `busy`, reconstructing and dropping a guard
    /// must observe the same value `read()` would and must clear `busy` again — the same
    /// postcondition a guard obtained through the checked `read()` path gives.
    #[kani::proof]
    fn create_read_guard_unchecked_matches_checked_read() {
        let mutex: BfSharedMutex<u32> = BfSharedMutex::new(42);

        mutex.acquire_shared().expect("uncontended acquire never fails");
        assert!(mutex.is_locked(), "acquire_shared must set busy");

        // SAFETY: `acquire_shared` above set `busy` for this instance, and no guard for this
        // acquisition has been produced yet, satisfying `create_read_guard_unchecked`'s contract.
        let guard = unsafe { mutex.create_read_guard_unchecked() };
        assert_eq!(*guard, 42);
        drop(guard);

        assert!(!mutex.is_locked(), "dropping the reconstructed guard must clear busy");
    }

    /// The write path must never observe a set `busy` flag on any registered clone once it has
    /// acquired exclusive access, matching `acquire_exclusive`'s debug assertions.
    #[kani::proof]
    fn write_excludes_concurrent_reader_state() {
        let mutex: BfSharedMutex<u32> = BfSharedMutex::new(0);
        let other = mutex.clone();

        assert!(!mutex.is_locked());
        assert!(!other.is_locked());
        assert!(!mutex.is_locked_exclusive());
        assert!(!other.is_locked_exclusive());

        let mut guard = mutex.write().expect("uncontended write never fails");
        *guard = 7;
        drop(guard);

        assert!(!mutex.is_locked_exclusive(), "write guard drop must clear forbidden");
        assert_eq!(*mutex.read().expect("uncontended read never fails"), 7);
    }
}

#[cfg(test)]
mod tests {
    use std::hint::black_box;

    use rand::RngExt;

    use merc_utilities::random_test_threads;
    use merc_utilities::test_threads;

    use super::BfSharedMutex;

    /// Small, deterministic (miri-friendly) exercise of `create_read_guard_unchecked`'s
    /// documented pairing with `acquire_shared`: the guard it reconstructs must deref to the
    /// same value a checked `read()` guard would, and dropping it must clear `busy` exactly
    /// once, matching a guard produced by `read()`.
    #[test]
    fn test_create_read_guard_unchecked_paired_with_acquire_shared() {
        let mutex = BfSharedMutex::new(5);

        mutex.acquire_shared().unwrap();
        assert!(mutex.is_locked(), "acquire_shared must set busy");

        // SAFETY: `acquire_shared` above set `busy` for this instance's one outstanding
        // acquisition, and no guard has yet been produced for it.
        let guard = unsafe { mutex.create_read_guard_unchecked() };
        assert_eq!(*guard, 5);
        drop(guard);

        assert!(!mutex.is_locked(), "dropping the reconstructed guard must clear busy");
    }

    /// Boundary transition: the instant a read guard is dropped, a write on the very same
    /// clone must succeed without blocking (`busy` must already read `false`), and the value
    /// it wrote must be visible to a read acquired immediately afterwards.
    #[test]
    fn test_read_then_write_guard_boundary() {
        let mutex = BfSharedMutex::new(1);
        {
            let r = mutex.read().unwrap();
            assert_eq!(*r, 1);
        }

        *mutex.write().unwrap() = 2;
        assert_eq!(*mutex.read().unwrap(), 2);
    }

    /// Boundary transition in the other direction: the instant a write guard is dropped
    /// (clearing every clone's `forbidden` flag), a read on a *different* clone must succeed
    /// without blocking.
    #[test]
    fn test_write_then_read_guard_boundary_on_other_clone() {
        let mutex = BfSharedMutex::new(1);
        let other = mutex.clone();

        {
            let mut w = mutex.write().unwrap();
            *w = 9;
        }

        assert_eq!(*other.read().unwrap(), 9);
    }

    /// A clone registered and immediately dropped without ever being locked (the empty
    /// boundary of the `other` registration table) must not disturb a concurrent write on a
    /// second, still-live clone: the drop path must remove exactly its own slot.
    #[test]
    fn test_drop_unused_clone_does_not_block_write() {
        let mutex = BfSharedMutex::new(0);
        let unused = mutex.clone();
        drop(unused);

        *mutex.write().unwrap() = 1;
        assert_eq!(*mutex.read().unwrap(), 1);
    }

    // These are just simple tests.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_bf_shared_mutex_exclusive() {
        let shared_number = BfSharedMutex::new(5);
        let num_iterations = 500;
        let num_threads = 3;

        test_threads(
            num_threads,
            || shared_number.clone(),
            move |number| {
                for _ in 0..num_iterations {
                    *number.write().unwrap() += 5;
                }
            },
        );

        assert_eq!(*shared_number.write().unwrap(), num_threads * num_iterations * 5 + 5);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_bf_shared_mutex() {
        let shared_vector = BfSharedMutex::new(vec![]);

        let num_threads = 20;
        let num_iterations = 5000;

        random_test_threads(
            num_iterations,
            num_threads,
            || shared_vector.clone(),
            |rng, shared_vector| {
                if rng.random_bool(0.95) {
                    // Read a random index.
                    let read = shared_vector.read().unwrap();
                    if !read.is_empty() {
                        let index = rng.random_range(0..read.len());
                        assert_eq!(*black_box(&read[index]), 5);
                    }
                } else {
                    // Add a new vector element.
                    shared_vector.write().unwrap().push(5);
                }
            },
        );
    }

    /// A `try_write` that observes the inner registration mutex already locked must report
    /// failure instead of handing out a second exclusive reference.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_bf_shared_mutex_try_write_fails_when_locked() {
        let shared_mutex = BfSharedMutex::new(0);
        let other = shared_mutex.clone();

        let guard = shared_mutex.write().unwrap();

        // While the write guard holds the inner mutex, no clone can acquire it.
        assert!(other.try_write().unwrap().is_none());
        assert!(shared_mutex.try_write().unwrap().is_none());

        drop(guard);

        // Once released, a non-blocking write succeeds again.
        let mut guard = other
            .try_write()
            .unwrap()
            .expect("try_write should succeed after unlock");
        *guard += 1;
        drop(guard);

        assert_eq!(*shared_mutex.read().unwrap(), 1);
    }

    /// Many threads racing through `try_write` must never lose an increment: each successful
    /// attempt is guaranteed exclusive access, so retrying until success yields an exact count.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_concurrent_bf_shared_mutex_try_write() {
        let shared_number = BfSharedMutex::new(0);
        let num_iterations = 500;
        let num_threads = 4;

        test_threads(
            num_threads,
            || shared_number.clone(),
            move |number| {
                for _ in 0..num_iterations {
                    // Retry until exclusive access is granted; a contended attempt returns None.
                    loop {
                        if let Some(mut guard) = number.try_write().unwrap() {
                            *guard += 1;
                            break;
                        }
                    }
                }
            },
        );

        assert_eq!(*shared_number.write().unwrap(), num_threads * num_iterations);
    }

    #[test]
    #[cfg(loom)]
    fn test_loom_bf_shared_mutex() {
        let mut builder = loom::model::Builder::new();
        // A bound of at least 2 is needed to find the missing fence.
        builder.preemption_bound = Some(2);

        builder.check(|| {
            let shared_mutex = BfSharedMutex::new(false);

            let threads: Vec<_> = (0..3)
                .map(|_| {
                    let shared_mutex = shared_mutex.clone();
                    loom::thread::spawn(move || {
                        // Just perform some operations on the shared mutex.
                        let result = *shared_mutex.read().unwrap();

                        *shared_mutex.write().unwrap() = !result;
                    })
                })
                .collect();

            for th in threads {
                th.join().unwrap();
            }
        });
    }

    #[test]
    #[cfg(loom)]
    fn test_loom_bf_shared_mutex_try_write() {
        let mut builder = loom::model::Builder::new();
        // A bound of at least 2 is needed to find the missing fence.
        builder.preemption_bound = Some(2);

        builder.check(|| {
            let shared_mutex = BfSharedMutex::new(0u32);

            let threads: Vec<_> = (0..2)
                .map(|_| {
                    let shared_mutex = shared_mutex.clone();
                    loom::thread::spawn(move || {
                        // A non-blocking write attempt either succeeds with exclusive
                        // access or reports contention; both branches must be sound.
                        if let Some(mut guard) = shared_mutex.try_write().unwrap() {
                            *guard += 1;
                        }

                        // A subsequent read must always succeed under the protocol.
                        let _ = *shared_mutex.read().unwrap();
                    })
                })
                .collect();

            for th in threads {
                th.join().unwrap();
            }
        });
    }
}
