use merc_sharedmutex::BfSharedMutex;
use merc_sharedmutex::RecursiveLock;

/// Regression test for the aliasing bug `with_mut` replaced `DerefMut` to close.
#[test]
#[should_panic(
    expected = "Cannot deref a RecursiveLockReadGuard while a RecursiveLockWriteGuard::with_mut call is in progress"
)]
fn test_read_recursive_deref_during_with_mut_panics() {
    let lock = RecursiveLock::new(42i32);
    let mut write = lock.write().unwrap();

    write.with_mut(|data| {
        *data = 100;
        // Reentrant call while `data` (the closure's `&mut i32`) is still logically
        // live: acquiring the guard is fine (see the test below), but *dereferencing*
        // it must panic rather than hand out an aliasing `&T`.
        let read = lock.read_recursive().unwrap();
        let _ = *read;
        *data += 1;
    });
}

/// Merely *acquiring* a recursive read guard during `with_mut`, without
/// ever dereferencing it, must not panic.
#[test]
fn test_read_recursive_without_deref_during_with_mut_succeeds() {
    let lock = RecursiveLock::new(42i32);
    let mut write = lock.write().unwrap();

    write.with_mut(|data| {
        *data = 100;
        let _guard = lock.read_recursive().unwrap();
        *data += 1;
    });

    assert_eq!(*write, 101);
}

/// The non-overlapping case `with_mut` is meant to keep working.
#[test]
fn test_read_recursive_after_with_mut_succeeds() {
    let lock = RecursiveLock::new(42i32);
    let mut write = lock.write().unwrap();

    write.with_mut(|data| *data = 100);

    let read = lock.read_recursive().unwrap();
    assert_eq!(*read, 100);
}

/// A lock made with `from_mutex` shares its data with the other clones of that mutex.
#[test]
fn test_from_mutex() {
    let mutex = BfSharedMutex::new(100);
    let lock = RecursiveLock::from_mutex(mutex.clone());
    assert_eq!(*lock.read().unwrap(), 100);

    *mutex.write().unwrap() = 7;
    assert_eq!(*lock.read().unwrap(), 7);

    lock.write().unwrap().with_mut(|v| *v = 8);
    assert_eq!(*mutex.read().unwrap(), 8);
}

/// Nested recursive reads track their depth and release the underlying read lock only when
/// the outermost guard drops.
#[test]
fn test_nested_recursive_reads() {
    let lock = RecursiveLock::new(42);

    let guard1 = lock.read_recursive().unwrap();
    assert_eq!(*guard1, 42);
    assert_eq!(guard1.read_depth(), 1);
    assert!(lock.is_locked());

    let guard2 = lock.read_recursive().unwrap();
    assert_eq!(*guard2, 42);
    assert_eq!(guard2.read_depth(), 2);

    let guard3 = lock.read_recursive().unwrap();
    assert_eq!(*guard3, 42);
    assert_eq!(guard3.read_depth(), 3);

    drop(guard3);
    assert_eq!(guard2.read_depth(), 2);
    assert!(lock.is_locked());

    drop(guard2);
    assert_eq!(guard1.read_depth(), 1);
    assert!(lock.is_locked());

    drop(guard1);
    assert!(!lock.is_locked());

    // The read section has ended, so writing is allowed again.
    lock.write().unwrap().with_mut(|v| *v = 43);
    assert_eq!(*lock.read().unwrap(), 43);
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
fn test_call_counters() {
    let lock = RecursiveLock::new(42);

    assert_eq!(lock.write_call_count(), 0);
    assert_eq!(lock.read_recursive_call_count(), 0);

    {
        let _guard = lock.write().unwrap();
        assert_eq!(lock.write_call_count(), 1);
        assert_eq!(lock.read_recursive_call_count(), 0);
    }

    {
        let _guard = lock.read_recursive().unwrap();
        assert_eq!(lock.write_call_count(), 1);
        assert_eq!(lock.read_recursive_call_count(), 1);
    }

    {
        let _guard = lock.write().unwrap();
        assert_eq!(lock.write_call_count(), 2);
        assert_eq!(lock.read_recursive_call_count(), 1);
    }

    // Nested recursive reads each count as a call.
    {
        let _guard1 = lock.read_recursive().unwrap();
        let _guard2 = lock.read_recursive().unwrap();
        assert_eq!(lock.write_call_count(), 2);
        assert_eq!(lock.read_recursive_call_count(), 3);
    }

    // Plain reads are not counted.
    drop(lock.read().unwrap());
    assert_eq!(lock.read_recursive_call_count(), 3);
}
