# Phase 1 — Foundation unsafe: `merc_sharedmutex` and `merc_unsafety` (5 unproved files)

## Verdict

Unsound: `RecursiveLock` (`crates/sharedmutex/src/recursive_lock.rs`) lets fully safe caller code construct a live `&mut T` and a live `&T` to the same object at the same time — no `unsafe` block required at the call site — and this is a confirmed Stacked-Borrows violation under Miri, not a theoretical concern. Everything else examined in `bf_sharedmutex.rs`, `bf_vec.rs`, and the five `merc_unsafety` files (`erasable.rs`, `counting_allocator.rs`, `concurrent_append_vec.rs`, `stable_pointer_set.rs`, `block_allocator.rs`) held up under review, Miri, and (where practical) Kani. `create_read_guard_unchecked` — flagged as the standout Kani target — turned out not to be a viable one at all: every Kani harness that touches `BfSharedMutex`'s inner `std::sync::Mutex`, even along a code path that never contends it, makes CBMC exhaust memory unwinding the real futex-based lock. `merc_unsafety` gained 6 new passing Kani harnesses (22/22 total, no regressions); `merc_sharedmutex` gained none, for the reason above, and relies instead on the boundary-test/doc-contract track added per the mid-task instruction. Both crates have zero loom coverage for `merc_unsafety`'s concurrent structures (`concurrent_append_vec.rs`, `block_allocator.rs`), which is itself a gap since neither uses the `cfg(loom)`-swappable primitive pattern `bf_sharedmutex.rs` does.

## Findings

### 1. `RecursiveLock` allows a live `&mut T` to alias a live `&T` — CONFIRMED

- **Location**: `crates/sharedmutex/src/recursive_lock.rs:234` (`RecursiveLockWriteGuard::deref_mut`) interacting with `crates/sharedmutex/src/recursive_lock.rs:180` (`RecursiveLockReadGuard::deref`).
- **Scenario**: `deref_mut`'s only protection against handing out `&mut T` while a nested `read_recursive()` guard is alive is an `assert!(recursive_depth == 1)` checked *at the moment `deref_mut` is called* — not for the lifetime of the reference it returns. Nothing stops a caller from taking that `&mut T` into a local variable, then calling `lock.read_recursive()` afterwards: `read_recursive` only borrows `lock` (the `RecursiveLock<T>` itself, `&self`), a value the borrow checker treats as independent of `write` (the `RecursiveLockWriteGuard` borrowed from it). The result is a genuinely live `&mut T` and a genuinely live `&T` to the same location, manufactured with no `unsafe` at the call site:

  ```rust
  let mut write = lock.write().unwrap();
  let data: &mut i32 = &mut *write;   // depth == 1, deref_mut's assert passes
  *data = 100;
  let read = lock.read_recursive().unwrap();   // depth -> 2; lock's own path, `write` untouched
  // `data` (&mut i32) and `*read` (&i32) are both live now.
  ```

- **Evidence** (test added at `crates/sharedmutex/src/recursive_lock.rs:273`, `test_deref_mut_alias_survives_nested_read_recursive`):

  ```
  MIRIFLAGS="-Zmiri-disable-isolation --cfg chacha20_force_soft" \
    cargo +nightly miri test -p merc_sharedmutex test_deref_mut_alias_survives_nested_read_recursive
  ```

  ```
  error: Undefined Behavior: attempting a read access using <137158> at alloc42336[0xa0], but that tag does not exist in the borrow stack for this location
     --> crates/sharedmutex/src/recursive_lock.rs:292:9
      |
  292 |         *data += 1;
      |         ^^^^^^^^^^ this error occurs as part of an access at alloc42336[0xa0..0xa4]
  help: <137158> was created by a Unique retag at offsets [0xa0..0xa4]
     --> crates/sharedmutex/src/recursive_lock.rs:279:30
      |
  279 |         let data: &mut i32 = &mut *write;
      |                              ^^^^^^^^^^^
  help: <137158> was later invalidated at offsets [0xa0..0xa4] by a SharedReadOnly retag
     --> crates/sharedmutex/src/recursive_lock.rs:185:13
      |
  185 |             self.mutex.inner.data_ptr().as_ref().unwrap_unchecked()
      |             ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
  ```

  Full run: `13 passed, 1 failed, 5 skipped` — every other test in the crate (including the 3 other new boundary tests below) is unaffected; only this one fails, and it fails at the exact write-after-alias line the scenario predicts, with Miri's own trace naming the read (`recursive_lock.rs:185`, `RecursiveLockReadGuard::deref`) as the access that invalidated the `&mut i32` tag created at `recursive_lock.rs:279`. This is exactly the failure the scenario describes, so it would pass once `deref_mut`'s exposure is scoped so it cannot outlive a subsequent `read_recursive` call.
- **Why this would pass once fixed**: the assertion sequence (`*read == 100`, then `*data += 1`, then `*data == 101`) only fails because Miri detects the invalidated tag; a fix that prevents the two references from coexisting (e.g. by construction, not by an assert) would make this exact interleaving either not compile or not reach the invalidated access.
- **Status**: CONFIRMED.
- **Fix direction** (one line): stop returning a `'a`-lifetime `&mut T` from `deref_mut` at all — thread mutable access through a scoped `with_mut(impl FnOnce(&mut T))`-style callback (or an RAII "mutation token" that `read_recursive` cannot be called while borrowed) so the borrow checker, not a call-time assert, enforces exclusivity for the whole duration of the mutation.

### 2. `bf_vec.rs`: `BfVecShared<T>` is never `Sync`, yet `BfVec<T>: Send`'s justification depends on `&BfVecShared<T>` being shareable — observation, not a bug

- **Location**: `crates/sharedmutex/src/bf_vec.rs:258` (`unsafe impl<T: Send + Sync> Send for BfVecShared<T>`) and `:270` (`unsafe impl<T: Send + Sync> Send for BfVec<T>`).
- No `unsafe impl Sync for BfVecShared<T>` exists anywhere in the file, and `BfVecShared<T>` cannot auto-derive `Sync` either (blocked by `buffer: Option<NonNull<T>>`). `BfVec<T>: Send` is only sound because `&BfVecShared<T>` really is safe to read from multiple threads concurrently — `buffer`/`capacity` are only ever mutated from behind a `BfSharedMutex` *write* lock (mutually exclusive with every read lock under the busy-forbidden protocol) and read from behind a *read* lock, while `reserved`/`len` are plain atomics designed to be raced on by concurrent readers — but nothing in the type system records that; the compiler never checks it for a manual `unsafe impl`, it's purely a human argument. I verified the argument holds (traced every mutation site of `buffer`/`capacity`/`reserved`/`len` against which lock guards them), so this is not a demonstrated bug, but the missing `Sync` impl means the soundness of `Send for BfVec<T>` is currently undocumented anywhere except the new `# Safety` comment I added — a future reader auditing `unsafe impl Sync for BfVecShared<T>`'s *absence* has no signal that its soundness is silently required elsewhere.
- **Status**: PLAUSIBLE (no runtime failure exists to test against — this is a documentation/traceability gap, not a memory-safety bug).
- No fix needed beyond what the added doc comment (`bf_vec.rs:260-274`) already states; could optionally be closed by adding the missing `unsafe impl<T: Send + Sync> Sync for BfVecShared<T>` to make the dependency explicit and compiler-checkable, but that is a production-code change outside this review's charter.

### 3. `merc_unsafety` has zero loom coverage for its lock-free concurrent structures

- `ConcurrentAppendVec` (`concurrent_append_vec.rs`) and `BlockAllocator` (`block_allocator.rs`) both have genuine multi-thread access patterns (`push`/`get`'s per-block Release/Acquire publication; `allocate_object`/`deallocate_object`'s thread-local freelist race against `remove_free_blocks`'s documented "must not run concurrently" contract) with no loom-gated test anywhere in the crate:

  ```
  RUSTFLAGS="--cfg loom" cargo nextest run --no-fail-fast --release -p merc_unsafety -- --include-ignored loom
  # Starting 0 tests across 1 binary (29 tests skipped)
  # Summary [0.000s] 0 tests run: 0 passed, 29 skipped
  ```

- Unlike `crates/sharedmutex/src/bf_sharedmutex.rs`, which isolates its atomics behind a `#[cfg(loom)] mod inner { ... }` swap so the same code compiles against loom's instrumented primitives, **none** of the five reviewed `merc_unsafety` files use that pattern — they import `std::sync::atomic::*`/`std::sync::Mutex` unconditionally. So this gap cannot be closed with a test-only change: it needs the same `cfg(loom)`-swap indirection introduced into production code first. Flagging as a genuine coverage gap per the task brief, not fixing it (out of charter).
- **Status**: confirmed gap (not a "bug" with a failure scenario — a missing verification surface).

## Checked and found correct

- `BfSharedMutex<T>`/`BfSharedMutexReadGuard`/`BfSharedMutexWriteGuard`/`GlobalBfSharedMutex<T>` `Send`/`Sync` marker impls (`bf_sharedmutex.rs:80, 240, 263, 599-600`): traced each field's actual concurrent-access pattern against the bound imposed; all sound. `BfSharedMutex<T>` is deliberately not `Sync` (each clone's `index`/`control` is only valid for the thread owning that clone), and the doc comment says so; I did not find a way to violate that (no code anywhere derives or asserts `Sync` for it).
- `BfVecShared<T>`/`BfVec<T>` `Send` impls (`bf_vec.rs:258, 270`): the aliasing argument for `buffer` holds — traced every read (`push`, `at`) and write (`reserve`) site against which `BfSharedMutex` guard protects it; write access is always exclusive relative to every read. See finding #2 for the one gap this surfaced (missing `Sync`, not a bug).
- `RecursiveLock`'s own bookkeeping (`recursive_depth`/`write_calls`/`read_recursive_calls` `Cell`s): traced the depth-counter arithmetic across every combination of `write()`, `read_recursive()` (fresh vs. nested), and out-of-order guard drops (`test_nested_recursive_reads` drops in reverse-of-creation order and still lands on `depth == 0`); found no path where `recursive_depth` underflows or a `busy` release fires more than once for one acquisition. The bug in finding #1 is specifically about the *aliasing* the API permits, not this counter logic.
- `create_read_guard_unchecked` (`bf_sharedmutex.rs:393`): re-audited every call site (the only one is `recursive_lock.rs`'s `RecursiveLockReadGuard::drop`, which pairs it correctly with a prior `acquire_shared`) and exercised the intended pairing directly (miri test below); it behaves exactly as documented when its (now more precisely stated) contract is honored.
- `ConcurrentAppendVec<T, BLOCK>` (`concurrent_append_vec.rs`): `push`'s block-reservation `fetch_add` gives every pushing thread a disjoint `(bucket, block, offset)` triple (proved for all `usize` via Kani, see below); the per-block `commits[block]` Release/Acquire pairing between `push` and `get`/`get_unchecked` is correctly ordered; `Bucket::drop`/`ConcurrentAppendVec::drop`/`clear` each drop committed slots exactly once (existing `drops_each_written_value_once` test, re-verified under Miri).
- `BlockAllocator<T, N>` (`block_allocator.rs`): the per-thread bump-allocation pointer arithmetic (`data_ptr.add(offset)` with `offset < N`) stays in bounds and non-null — proved directly with Kani (see below) rather than only argued in the comment. `Entry<T>`'s union-based freelist link (`get_next`/`set_next`) round-trips correctly (Kani-proved). `remove_free_blocks`'s sentinel-marking pass is internally consistent with `refill_local_free_from_chunks`'s "local freelist must be empty before refill" invariant (existing debug_assert, unchanged).
- `erasable.rs`'s `Erasable for T: Sized` blanket impl and `Thin<T>` round-trip: Kani-proved for arbitrary pointer values, not just argued.
- `stable_pointer_set.rs`: `StablePointer<T>`/`Entry<T>` `Send`/`Sync` bounds are exactly what the API needs (traced `deref`/hashing/dropping call sites); `insert`/`insert_equiv`/`remove`/`retain`/`clear`'s allocate-then-check-then-maybe-free sequencing is internally consistent (no double-free or leak path found on the "lost the insert race" branches). Not a good Kani target — see below.
- `counting_allocator.rs`: `AllocCounter`'s `Allocator`/`GlobalAlloc` impls correctly special-case the zero-size layout (never forwarded to `System.alloc`, per `GlobalAlloc`'s own documented UB otherwise); the peak-tracking CAS loops are best-effort under `Relaxed` as documented, not a correctness requirement.

## Miri

Ran per the `unsafe-verify` skill's exact invocation, scoped to each crate:

```
MIRIFLAGS="-Zmiri-disable-isolation --cfg chacha20_force_soft" cargo +nightly miri nextest run --no-fail-fast -p merc_sharedmutex
MIRIFLAGS="-Zmiri-disable-isolation --cfg chacha20_force_soft" cargo +nightly miri nextest run --no-fail-fast -p merc_unsafety
```

- **`merc_unsafety`** (pre-existing suite, before any of my changes): `19 tests run: 19 passed, 10 skipped`. All green; no regressions from doc-comment-only production edits (re-confirmed via the later `cargo kani` build, which compiles the same crate and also succeeded — see below).
- **`merc_sharedmutex`**, final run after all test/doc additions: `14 tests run: 13 passed, 1 failed, 5 skipped` — the 1 failure is the intentional UB demonstration in finding #1; every other test, including the 3 new boundary tests, passes.
- **`merc_unsafety`**, targeted boundary test: `bucket_boundary_crossing_resolves_correctly` — `1 test run: 1 passed`.

## Loom

```
RUSTFLAGS="--cfg loom" cargo nextest run --no-fail-fast --release -p merc_sharedmutex -- --include-ignored loom
# Starting 3 tests across 1 binary (13 tests skipped) — 3 passed, 0 failed
RUSTFLAGS="--cfg loom" cargo nextest run --no-fail-fast --release -p merc_unsafety -- --include-ignored loom
# Starting 0 tests across 1 binary (29 tests skipped) — no loom tests exist (finding #3)
```

`merc_sharedmutex`'s 3 existing loom tests (`test_loom_bf_shared_mutex`, `test_loom_bf_shared_mutex_try_write`, `test_loom_recursive_lock`) all pass, including `test_loom_recursive_lock`, which does exercise nested `read_recursive()` inside a `write()` section under loom — but not the specific `deref_mut`-then-`read_recursive` aliasing pattern in finding #1, which is a single-threaded aliasing bug (no interleaving needed), so loom would not have caught it regardless; Miri's Stacked Borrows check is the right tool here and did catch it.

## Kani

### `merc_unsafety` — 6 new harnesses added, all passing; 16 pre-existing unaffected

```
cd crates/unsafety && cargo kani
...
Complete - 22 successfully verified harnesses, 0 failures, 22 total.
```

New harnesses (all `VERIFICATION:- SUCCESSFUL`):

- `erasable.rs::verification::erase_unerase_roundtrip_is_identity` — proves `Erasable for T: Sized`'s round-trip contract (`unerase(erase(p)) == p`) for arbitrary `NonNull<u64>`.
- `erasable.rs::verification::thin_new_as_nonnull_roundtrip_is_identity` — same, through `Thin::new`/`Thin::as_nonnull`.
- `block_allocator.rs::verification::entry_get_next_set_next_roundtrip` — proves the `Entry<T>` union's `FreeListEntry` impl round-trips an arbitrary next-pointer value without corruption.
- `block_allocator.rs::verification::bump_allocation_offset_stays_in_bounds` — proves, for the actual `Block<T, N>` type (`N = 4`) and any symbolic `offset < N`, that `data_ptr.add(offset)` stays within the block's array and the resulting `T`-pointer is never null — this is the exact claim `allocate_object`'s fast-path `SAFETY` comment makes, now machine-checked rather than only argued.
- `concurrent_append_vec.rs::verification::bucket_of_block_bounds_and_offset_consistent` — proves, for the full unbounded `usize` domain, that `bucket_of_block` never indexes outside `NUM_BUCKETS`, and that `bucket_start_blocks`/`bucket_blocks` are mutually consistent with it (`block` always falls inside its own bucket's `[start, start+count)` range). This is the structural invariant every unsafe slot index downstream (`push`, `get`, `get_unchecked`, `bucket_or_alloc`) depends on.
- `concurrent_append_vec.rs::verification::locate_slot_within_bucket_bounds` — proves `locate`'s flattened `block * BLOCK + offset` slot index stays under the bucket's allocated slot count, for `ConcurrentAppendVec<u64, 256>` and any index `< 2^40` (bounded only to keep the flattened-index multiplication itself from overflowing `usize` in the harness's own arithmetic; the property it proves is otherwise the general one).

**Not attempted / deliberately not written**:
- Bare `unsafe impl Send/Sync` marker impls throughout both crates (`ConcurrentAppendVec`, `ThreadLocalAllocState`, `BlockAllocator`, `StablePointer`, `Entry` in `stable_pointer_set.rs`) — per the task brief, these get no harness; Kani proves memory safety of executable code, not "is this trait bound sound to assert," which is a semantic argument about aliasing across threads that a single-threaded symbolic-execution proof cannot state. These are instead covered by the upgraded `# Safety` doc-comment contracts (see below).
- `stable_pointer_set.rs`'s `StablePointerSet`/`insert`/`remove`/`retain` — assessed and rejected as a Kani target: the type is generic over `DashSet` (a third-party sharded concurrent hash set) and `allocator_api2::Allocator`, neither of which Kani can reason about without either stubbing out their internals (losing the property being checked) or an intractable state space. Covered by the existing 9-test suite (re-run clean under Miri) and the added `# Safety` doc comments instead.
- `counting_allocator.rs` — no unsafe pointer manipulation beyond delegating straight to `System.alloc`/`System.dealloc`; nothing there is a meaningfully boundable Kani target beyond what the existing tests (re-run clean under Miri, including the multi-threaded `test_thread_safety`) already cover.

### `merc_sharedmutex` — no Kani harnesses added; `create_read_guard_unchecked` is not a viable Kani target under this toolchain

The task brief named `create_read_guard_unchecked` the standout Kani candidate in this crate. After adding the `[package.metadata.kani]` block (copied verbatim from `merc_unsafety`'s `Cargo.toml`) to `crates/sharedmutex/Cargo.toml`, I wrote and ran two harnesses; both had to be discarded:

1. A harness for `write()`/`acquire_exclusive` (which locks the `other: Mutex<Vec<Option<Arc<..>>>>` registration table): did not terminate after several minutes of CBMC CPU time.
2. A harness for `create_read_guard_unchecked`, paired correctly with a prior `acquire_shared()` on a freshly-constructed, uncontended `BfSharedMutex` (i.e. `forbidden == false`, so `acquire_shared`'s `while forbidden { .. }` loop body — the only place that locks the inner `Mutex` — is not logically reachable on this concrete execution): **still failed**, with CBMC running out of memory unwinding `std::sync::mutex::futex::Mutex::lock_contended`'s internal retry loop past 384 iterations:

   ```
   cd crates/sharedmutex && cargo kani
   ...
   Unwinding loop _RNv...5mutex5futexNtB2_5Mutex14lock_contended.0 iteration 384 ...
   CBMC failed
   VERIFICATION:- FAILED
   CBMC appears to have run out of memory. You may want to rerun your proof in an
   environment with additional memory or use stubbing to reduce the size of the
   code the verifier reasons about.
   Manual Harness Summary:
   Verification failed for - bf_sharedmutex::verification::create_read_guard_unchecked_matches_checked_read
   Complete - 0 successfully verified harnesses, 1 failures, 1 total.
   ```

   CBMC evidently cannot statically resolve `forbidden`'s concrete `false` value through the `Arc<CachePadded<SharedMutexControl>>` indirection and so explores the `.lock()`-calling branch of `acquire_shared` regardless, at which point Kani's default (unstubbed) `std::sync::Mutex` — a real futex-based implementation — has an internal contention-retry loop with no static bound, which CBMC then tries to fully unwind.

   This is a genuine tooling limitation, not a code defect: **any** function in `bf_sharedmutex.rs` that reaches `self.shared.other` (directly, or via `SharedMutexControl`'s registration table) is not currently a viable Kani target, regardless of which specific unsafe operation it performs. Both harnesses were removed from the tree rather than left failing or artificially bounded with an unjustified `#[kani::unwind]` that would silently stop verifying the property.

   `create_read_guard_unchecked`'s contract is instead covered by:
   - a formalized `# Safety` doc contract (`bf_sharedmutex.rs:326-355` in spirit — see the Contracts section below), and
   - the targeted Miri test `test_create_read_guard_unchecked_paired_with_acquire_shared` (`bf_sharedmutex.rs:641`), which exercises exactly the pairing the discarded Kani harness attempted, and passes under Miri.

`cd crates/sharedmutex && cargo kani` now runs 0 harnesses (clean, since none exist) — this is expected, not a failure.

## Miri boundary tests (added per the mid-task instruction)

All small, deterministic, single-threaded — no `random_test`/`test_threads` stress patterns — targeting the actual edges of each unsafe contract:

- `bf_sharedmutex.rs:641` `test_create_read_guard_unchecked_paired_with_acquire_shared` — the exact `acquire_shared` → `create_read_guard_unchecked` → drop pairing the function's `# Safety` contract requires; checks the reconstructed guard derefs to the same value a checked `read()` would and clears `busy` on drop.
- `bf_sharedmutex.rs:660` `test_read_then_write_guard_boundary` — the instant a read guard drops, a write on the *same* clone must succeed without blocking.
- `bf_sharedmutex.rs:675` `test_write_then_read_guard_boundary_on_other_clone` — the instant a write guard drops, a read on a *different* clone must succeed without blocking (the empty-writer-table-slot boundary between clones).
- `bf_sharedmutex.rs:691` `test_drop_unused_clone_does_not_block_write` — an unused clone, registered and dropped without ever being locked (the empty end of the `other` table's lifecycle), must not disturb a write on a separate live clone.
- `bf_vec.rs:323` `test_first_reserve_boundary` — the very first `reserve` (`capacity` 0 → 8, the empty-buffer branch with nothing to copy or free) followed immediately by the boundary that triggers the *second* reserve (8 → 16, the non-empty copy-and-free branch), checking every element below the boundary survives the resize.
- `concurrent_append_vec.rs:470` `bucket_boundary_crossing_resolves_correctly` — crosses the bucket-0/bucket-1 boundary exactly (index 64 with `BLOCK = 4`, `FIRST_BLOCKS = 16`), checking both the last slot of bucket 0 and the first slot of the newly-allocated bucket 1 resolve correctly and `iter()` still sees every element.

All ran and passed under Miri (see the Miri section above and each test's own run for individual confirmation).

## Mathematical `# Safety` contracts (doc-comment-only changes)

Rewrote the safety comments on every function/impl named in the task, from one-line prose into precise pre/postcondition statements. All changes are comments only — no production logic changed. Full text is in the files; summary of what each now states:

- **`bf_sharedmutex.rs:393` `create_read_guard_unchecked`**: formalized as requiring a single outstanding "logical read acquisition" `A` established by a prior `acquire_shared` (or `read()` + `mem::forget`), for which no guard currently exists; states `self.control.busy == true` must hold for the guard's entire lifetime, and that producing two live guards for the same `A` is UB because `Drop` would then clear `busy` twice for one acquisition, letting a concurrent writer observe `busy == false` while a live `&T` from the still-outstanding guard remains reachable. States the postcondition (deref equivalent to `read()`'s, `Drop` clears `busy`).
- **`bf_sharedmutex.rs:80` `Send for BfSharedMutex<T>`**: names `shared: Arc<CachePadded<SharedData<T>>>`'s `UnsafeCell<T>` as the specific field blocking auto-`Send` (never auto-`Sync` regardless of `T`), and separately states why `control`/`index` impose no bound while `shared` requires exactly `T: Send + Sync`.
- **`bf_sharedmutex.rs:240, 263` `Sync for BfSharedMutexWriteGuard`/`ReadGuard`**: names the blocking field (`mutex: &'a BfSharedMutex<T>`, itself never `Send`/`Sync` since `BfSharedMutex` is deliberately `!Sync`) and states the only operation reachable via `&Guard` is `Deref::deref` → `&T`, so `T: Sync` is exactly sufficient; notes `Drop` is unreachable via `&self`.
- **`bf_sharedmutex.rs:599-600` `Send`/`Sync for GlobalBfSharedMutex<T>`**: traces both to the single `shared_mutex: BfSharedMutex<T>` field and the `share()` method's own internal `Mutex`-serialized registration.
- **`bf_vec.rs:243, 260` `Send for BfVecShared<T>`/`BfVec<T>`**: names `buffer: Option<NonNull<T>>` as the blocking field, states the `T: Send` requirement comes from cross-thread drop/ownership transfer and `T: Sync` from `at`'s clone-while-others-read pattern; the `BfVec<T>` comment explicitly flags the missing `Sync for BfVecShared<T>` impl (finding #2 above) rather than glossing over it.
- **`concurrent_append_vec.rs` `Send`/`Sync for ConcurrentAppendVec<T, BLOCK>`**: names `_marker: PhantomData<*mut T>` as the blocking field; states the no-two-threads-write-the-same-slot argument precisely (unique `fetch_add`-reserved block composed with a thread-local bump offset) and the happens-before argument for the Release/Acquire `commits[block]` pairing.
- **`stable_pointer_set.rs` `Send`/`Sync for StablePointer<T>`**: names `ptr: Thin<T>`'s `ErasedPtr`/`NonNull` as the blocking field; states the handle itself needs no bound to move (it's inert — see the type's own doc comment), only `unsafe fn deref` needs `T: Sync`, given its own contract is upheld.
- **`stable_pointer_set.rs` `Send`/`Sync for Entry<T>`**: names `ptr: NonNull<T>`, states the requirement comes from `DashSet` potentially dropping/reading an entry on a thread other than the one that inserted it.
- **`block_allocator.rs` `Send for ThreadLocalAllocState<T, N>`**: names `current_block: Cell<*mut Block<T, N>>`; states the thread-affinity invariant precisely (`ThreadLocal::get_or` guarantees single-thread access to `current_block`/`bump_offset`/`free`) and why cross-thread final `Drop` is still sound (no dereference of `current_block` happens in `Cell`/`FreeList`'s own drop).

## Tests and proofs added

All in `#[cfg(test)]`/`#[cfg(kani)]` modules or Cargo.toml metadata; no production logic changed.

| File | What | Run with |
|---|---|---|
| `crates/sharedmutex/Cargo.toml` | `[package.metadata.kani]` block (verbatim copy from `merc_unsafety`) | n/a (enables `cargo kani`) |
| `crates/sharedmutex/src/recursive_lock.rs:273` | `test_deref_mut_alias_survives_nested_read_recursive` — **CONFIRMED UB regression test, intentionally fails under Miri** | `MIRIFLAGS="-Zmiri-disable-isolation --cfg chacha20_force_soft" cargo +nightly miri test -p merc_sharedmutex test_deref_mut_alias_survives_nested_read_recursive` |
| `crates/sharedmutex/src/bf_sharedmutex.rs:641,660,675,691` | 4 Miri boundary tests (see above) | `cargo +nightly miri nextest run -p merc_sharedmutex` (or plain `cargo test -p merc_sharedmutex`) |
| `crates/sharedmutex/src/bf_vec.rs:323` | `test_first_reserve_boundary` | same |
| `crates/unsafety/src/erasable.rs` | 2 Kani proofs (`erase_unerase_roundtrip_is_identity`, `thin_new_as_nonnull_roundtrip_is_identity`) | `cd crates/unsafety && cargo kani` |
| `crates/unsafety/src/block_allocator.rs` | 2 Kani proofs (`entry_get_next_set_next_roundtrip`, `bump_allocation_offset_stays_in_bounds`) | same |
| `crates/unsafety/src/concurrent_append_vec.rs` | 2 Kani proofs (`bucket_of_block_bounds_and_offset_consistent`, `locate_slot_within_bucket_bounds`) + 1 Miri boundary test (`bucket_boundary_crossing_resolves_correctly`) | `cd crates/unsafety && cargo kani`; `cargo +nightly miri nextest run -p merc_unsafety bucket_boundary_crossing_resolves_correctly` |

Doc-comment-only `# Safety`/`SAFETY` rewrites (no test to "run", evidence is the contract text itself, per the mid-task instruction): `bf_sharedmutex.rs` (`create_read_guard_unchecked`, the `BfSharedMutex`/`BfSharedMutexWriteGuard`/`BfSharedMutexReadGuard`/`GlobalBfSharedMutex` marker impls), `bf_vec.rs` (`BfVecShared`/`BfVec` marker impls), `concurrent_append_vec.rs` (`ConcurrentAppendVec` marker impls), `stable_pointer_set.rs` (`StablePointer`/`Entry` marker impls), `block_allocator.rs` (`ThreadLocalAllocState` marker impl).

All absolute paths are under `/home/user/merc/crates/sharedmutex/src/` and `/home/user/merc/crates/unsafety/src/`.
