//! Regression test for a GC/locking reentrancy defect surfaced by the `RecursiveLock::with_mut`
//! fix in `merc_sharedmutex` (see the phase-1 review report for `crates/aterm` for the full
//! analysis). Kept in its own test binary/file on purpose: the defect currently makes the
//! process abort, and an abort in this binary must not take down any other test target.

use merc_aterm::ATerm;
use merc_aterm::ATermRef;
use merc_aterm::Protected;
use merc_aterm::Symbol;
use merc_aterm::storage::THREAD_TERM_POOL;

/// `GlobalTermPool::collect_garbage` marks every live `Protected`/`ProtectedSend` container by
/// calling `GcMutex::lock()` on it (see `Markable for GcMutex<T>` and
/// `GlobalTermPool::mark_roots`'s `container.mark(&mut marker)` loops), which reentrantly calls
/// `RecursiveLock::read_recursive()`. But the collector itself now runs inside
/// `RecursiveLockWriteGuard::with_mut`'s closure (`ThreadTermPool::force_collect_garbage` /
/// `collect_garbage` both wrap their `GlobalTermPool::trigger_garbage_collection`/
/// `collect_garbage` calls in `with_mut`), and `with_mut` explicitly forbids exactly this
/// reentrant `read_recursive()` call, for exactly the reason it would otherwise be unsound: it
/// would alias the live `&mut GlobalTermPool` the closure holds with a `&GlobalTermPool`
/// manufactured mid-call.
///
/// Before the `RecursiveLock::with_mut` fix (when `RecursiveLockWriteGuard` exposed a bare
/// `DerefMut`), this same reentrant call silently succeeded and produced exactly that aliased
/// `&mut GlobalTermPool` / `&GlobalTermPool` pair for the duration of `mark_roots`'s
/// container-marking loop -- the identical class of Stacked Borrows violation the fix targets,
/// just reached through the collector's *own* marking pass rather than through external caller
/// code holding a guard across statements. So this is not a defect introduced by adapting
/// `crates/aterm` to the new API; the new API turns a pre-existing silent aliasing violation
/// into a deterministic panic here, whereas it previously went undetected (miri did not exercise
/// a `Protected`/`ProtectedSend` container alive across a `collect_garbage()` call together with
/// a thread-sanitizer or data-race-aware pass sensitive to this specific single-threaded
/// same-object aliasing pattern).
///
/// # Why this test is `#[ignore]`d
///
/// The panic happens while `ThreadTermPool::force_collect_garbage` holds the pool's write lock.
/// Unwinding through `with_mut` leaves the underlying (poison-on-panic) lock poisoned; since
/// `RecursiveLock`'s inner mutex is shared by every thread's `ThreadTermPool`, *every*
/// subsequent `ThreadTermPool::drop` (thread-local teardown, including this test's own thread)
/// then panics again on its own `.expect("Lock poisoned!")`. A panic during thread-local
/// destruction is escalated by the Rust runtime to a full process abort ("thread local panicked
/// on drop, aborting"), which is why this test lives in its own file/binary and is `#[ignore]`d
/// -- so a normal `cargo test` run does not abort. Run it explicitly to observe the failure:
///
/// ```text
/// cargo test -p merc_aterm --test gc_reentrant_container_marking_test -- --ignored --test-threads=1
/// ```
#[test]
#[ignore = "aborts the process by design -- see the module/test doc comment; run explicitly"]
fn test_collect_garbage_with_live_protected_container_panics_on_reentrant_read_recursive() {
    let mut container = Protected::<Vec<ATermRef<'static>>>::new(vec![]);
    let term = ATerm::constant(&Symbol::new("gc_reentrant_marking_probe", 0));
    container.write().push(term.get());

    // This is the call that panics (inside `GlobalTermPool::mark_roots`'s container-marking
    // loop, via `GcMutex::lock()` -> `RecursiveLock::read_recursive()`), not the `Protected`
    // container construction above.
    THREAD_TERM_POOL.with(|tp| tp.force_collect_garbage());
}
