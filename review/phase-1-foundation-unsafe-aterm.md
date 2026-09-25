# Phase 1 — Foundation unsafe: `merc_aterm` (`crates/aterm`)

## Verdict

The unsafe surface `merc_aterm` owns directly (`StablePointer`/`Erasable`/`SliceDst`-based pointer
casts, the `Transmutable` lifetime-shrink trait, `Send`/`Sync` marker impls, `BlockAllocatorSafe`
impls) held up under review, 5 new Kani proofs, and 20 new targeted Miri boundary tests — no
memory-safety defect was found in that surface itself, and one existing API (`arity` passed to
the private `cast_to_shared_term_ptr`) turned out to be dead code, confirmed live under Miri.
`SharedSymbol`'s `BlockAllocatorSafe` impl rests on a `#[repr(Rust)]` field-order assumption the
type does not actually guarantee (PLAUSIBLE, not exploitable today, one-line fix identified).

**The one CONFIRMED defect found in this crate is severe and crate-wide**, and was surfaced only
by applying the mid-task-directed migration to `merc_sharedmutex`'s new
`RecursiveLockWriteGuard::with_mut` API (see that section below): `GlobalTermPool::collect_garbage`
marks every live `Protected`/`ProtectedSend` container by calling `GcMutex::lock()`, which
reentrantly calls `RecursiveLock::read_recursive()` from *inside* the very `with_mut` closure the
collector itself now runs under. `with_mut` exists specifically to forbid that reentrant call (it
would otherwise alias the closure's live `&mut GlobalTermPool` with a freshly manufactured
`&GlobalTermPool`), so it panics — deterministically, on the very first garbage collection that
runs while any container is alive. This is not a new bug my migration introduced: it is the exact
same class of Stacked-Borrows violation the `merc_sharedmutex` review found and fixed, reached
through a different call path (the collector's own marking pass, not external caller code holding
a guard across statements) — the old `DerefMut`-based API let this aliasing happen *silently*.
The fix now converts that silent unsoundness into a loud, 100%-reproducible panic that currently
aborts the process (poison-on-panic cascades into every thread's `ThreadTermPool::drop`). This
blocks `cargo test -p merc_aterm --lib` entirely (confirmed: it aborts on the first test that
exercises `collect_garbage`/`force_collect_garbage` while a `Protected`/`ProtectedSend` container
is alive — a large fraction of the suite, since `BinaryATermWriter`/`BinaryATermReader` use these
containers internally and force a collection on `Drop`). Fixing it is an architecture-level change
to `GlobalTermPool::mark_roots`/`collect_garbage` (they need `&mut self` for sweeping while also
needing shared access to mark containers reentrantly through the same lock) that is outside this
review's test/proof/doc-comment charter; I did not attempt it. **This needs the implementor's
attention before `crates/aterm` can be considered compiling-and-green again.**

## Findings

### 1. `GlobalTermPool::collect_garbage` cannot run while any `Protected`/`ProtectedSend` container is alive — CONFIRMED, crate-wide blocking

- **Location**: `crates/aterm/src/storage/global_aterm_pool.rs:401` (`GlobalTermPool::mark_roots`'s
  `container.mark(&mut marker)` for the per-thread `container_protection_set`; the same pattern
  at `:422` for `send_container_protection_sets`), reached via
  `crates/aterm/src/storage/gc_mutex.rs:57` (`GcMutex::lock`) →
  `crates/sharedmutex/src/recursive_lock.rs:144` (`RecursiveLock::read_recursive`, the assert that
  fires) — from inside `crates/sharedmutex/src/recursive_lock.rs:285`
  (`RecursiveLockWriteGuard::with_mut`'s closure invocation), entered via
  `crates/aterm/src/storage/thread_aterm_pool.rs:474`/`484` (`force_collect_garbage`/
  `collect_garbage`, both now wrapping `GlobalTermPool::collect_garbage`/
  `trigger_garbage_collection` in `with_mut` per the required migration).
- **Scenario**: any live `Protected<C>` or `ProtectedSend<C>` container (used internally by
  `BinaryATermWriter`/`BinaryATermReader` for their `function_symbols`/`terms`/`stack` fields, and
  directly by any external user of `Protected::new`) → any garbage collection pass, forced or
  automatic-budget-triggered → `mark_roots` reaches that container and calls `.mark()` on it →
  `GcMutex::lock()` → `read_recursive()` → panics with "Cannot call read_recursive() while a
  RecursiveLockWriteGuard::with_mut call is in progress", because the collector's own call stack
  is *itself* still inside the `with_mut` closure that `force_collect_garbage`/`collect_garbage`
  opened to call `trigger_garbage_collection`/`collect_garbage`.
- **Evidence** — minimal, isolated reproduction (own test binary; the panic poisons the process-wide
  lock, so this is kept out of shared binaries and `#[ignore]`d — see the test file's doc comment):

  ```
  cd /home/user/merc && cargo test -p merc_aterm --test gc_reentrant_container_marking_test \
    -- --ignored --test-threads=1 --nocapture
  ```

  ```
  thread 'test_collect_garbage_with_live_protected_container_panics_on_reentrant_read_recursive' panicked at crates/sharedmutex/src/recursive_lock.rs:144:9:
  Cannot call read_recursive() while a RecursiveLockWriteGuard::with_mut call is in progress
  stack backtrace:
     2: merc_sharedmutex::recursive_lock::RecursiveLock<T>::read_recursive
     3: merc_aterm::storage::gc_mutex::GcMutex<T>::lock::{{closure}}
     6: merc_aterm::storage::gc_mutex::GcMutex<T>::lock
     7: <merc_aterm::storage::gc_mutex::GcMutex<T> as merc_aterm::markable::Markable>::mark
     8: merc_aterm::storage::global_aterm_pool::GlobalTermPool::mark_roots
     9: merc_aterm::storage::global_aterm_pool::GlobalTermPool::collect_garbage
    10: merc_aterm::storage::thread_aterm_pool::ThreadTermPool::force_collect_garbage::{{closure}}
    11: merc_sharedmutex::recursive_lock::RecursiveLockWriteGuard<T>::with_mut
    12: merc_aterm::storage::thread_aterm_pool::ThreadTermPool::force_collect_garbage

  thread '...' panicked at crates/aterm/src/storage/thread_aterm_pool.rs:602:48:
  Lock poisoned!: PoisonError { .. }
  fatal runtime error: thread local panicked on drop, aborting
  ```

  The second panic (`Lock poisoned!`, in `ThreadTermPool::drop`) is the cascade: the first panic
  poisons the shared, process-wide `std::sync`-backed lock inside `RecursiveLock`, so every
  subsequent `ThreadTermPool::drop` on any thread (including this test's own, during thread-local
  teardown) panics again, which Rust escalates to a process abort.
- **Blast radius, confirmed directly**: the crate's own library test suite aborts on this:

  ```
  cargo test -p merc_aterm --lib -- --test-threads=1
  ```
  aborts (`SIGABRT`) the first time `ThreadTermPool::drop` observes the poison, seeded by
  `aterm_binary_stream::tests::test_binary_stream_roundtrips_int_and_list_subterms` (its
  `BinaryATermWriter`/`BinaryATermReader` hold `ProtectedSend`/`Protected` containers and force a
  collection on `Drop`). Every test that follows it in the same process is also lost. This is not
  specific to that one test — any test that keeps a `Protected`/`ProtectedSend` container alive
  across a `collect_garbage()` call (forced, or automatic once the GC budget is exhausted) hits
  the same panic.
- **Why this would pass once fixed**: the assertion in `read_recursive` fails only because the
  collector's own marking pass is nested inside `with_mut`'s closure; a fix that lets
  `mark_roots`'s container-marking loop run without needing `&mut GlobalTermPool` for that part
  (e.g. restructuring so the mutable state `mark_roots` needs — `marked_terms`/`marked_symbols`/
  `stack`, and the end-of-pass `send_*_protection_sets` reclamation — is threaded separately from
  the shared access needed to call into each container's own lock) would let this exact scenario
  complete without hitting the reentrant `read_recursive()` at all.
- **Status**: CONFIRMED. This is not a defect I introduced by choosing an unusual `with_mut`
  placement — I wrapped only the `&mut self`-requiring calls, as instructed and as the API
  requires; the reentrancy is inherent to `mark_roots`'s design (`container.mark()` needing to
  call back into the *same* lock while the collector already holds it exclusively) and was already
  latent, silent unsoundness under the old `DerefMut`-based API (a `&mut self` method
  (`trigger_garbage_collection`) manufacturing a `&self`-equivalent access to the same object via
  a side channel — the thread-local `THREAD_TERM_POOL`'s handle to the identical
  `RecursiveLock<GlobalTermPool>` — while its own `&mut self` receiver was still live for the rest
  of the method). The `with_mut` fix correctly detects and rejects it; it does not, and structurally
  cannot, "fix" `crates/aterm`'s side of the reentrancy on its own.
- **Fix direction** (architecture-level, out of this review's charter): `mark_roots` needs a way to
  mark containers (which requires calling back into the pool's own lock via `GcMutex::lock`)
  without holding the pool's `&mut self` for that specific step — e.g. take the container `Arc`s
  out of the mutable borrow first (they're already `Arc`s, so cloning the handles needs no
  `&mut`), mark them via a separate, non-`with_mut` code path once the collector's own `&mut`
  access to `marked_terms`/`marked_symbols`/`stack`/`send_*_protection_sets` is no longer needed
  for that step, or (more invasively) change `GcMutex::lock`'s marking-time access to not go
  through `read_recursive()` at all, since the collector holding the pool's write lock already
  implies no other thread can be mutating the container concurrently.

### 2. `cast_to_shared_term_ptr`'s `arity` parameter has no observable effect — CONFIRMED (not a safety bug; dead/misleading parameter)

- **Location**: `crates/aterm/src/storage/aterm_storage.rs` (private fn `cast_to_shared_term_ptr`,
  called from `insert_fixed_iter`/`insert_int_term`/`retain` with `arity` values 0–7).
- **Scenario**: `cast_to_shared_term_ptr(ptr, arity)` builds a transient fat pointer
  `slice_from_raw_parts_mut(ptr.ptr().as_ptr(), arity) as *mut SharedTerm`, then immediately hands
  it to `StablePointer::from_related_ptr`, whose `Thin::new` calls `SharedTerm`'s custom
  `Erasable::erase`, which is just `this.cast()` on the fat pointer — for a fat-to-thin cast, this
  **discards the slice-length metadata entirely** (the address survives, the length does not). So
  the `arity` argument is thrown away at construction time, and every later read of the returned
  `StablePointer<SharedTerm>` (`.ptr()`/`.deref()`) goes through `SharedTerm`'s custom `unerase`,
  which **recomputes** the slice length independently, from the live symbol header
  (`symbol.arity()`) — never from the value originally passed to `cast_to_shared_term_ptr`.
- **Evidence** (added as `crates/aterm/src/storage/aterm_storage.rs`'s
  `probe_cast_to_shared_term_ptr_arity_argument_effect`, run under both plain `cargo test` and
  Miri): a genuine one-argument term is built through the real `terms_1` storage path, then
  `cast_to_shared_term_ptr(&fixed, 0)` is called with a **deliberately wrong** arity (0 instead of
  1). The returned pointer's `.arguments().len()` is still `1`.

  ```
  cargo test -p merc_aterm --lib probe_cast_to_shared_term_ptr_arity_argument_effect
  # test storage::aterm_storage::tests::probe_cast_to_shared_term_ptr_arity_argument_effect ... ok

  MIRIFLAGS="-Zmiri-disable-isolation --cfg chacha20_force_soft" \
    cargo +nightly miri test -p merc_aterm --lib probe_cast_to_shared_term_ptr_arity_argument_effect
  # test storage::aterm_storage::tests::probe_cast_to_shared_term_ptr_arity_argument_effect ... ok
  ```
- **Why this is not itself a safety bug**: since every real call site always passes the arity that
  actually matches the source allocation's own symbol (checked directly for `insert_fixed_iter`'s
  `terms_1..terms_7`/`insert_int_term`'s call sites, and structurally true by construction — the
  `SharedTermFixed<N>`/`SharedTermInt` values are always built with a symbol whose arity is
  exactly `N`/`0`), the value the parameter is silently ignoring is always the same value
  `unerase` recomputes anyway; there is currently no path where they could diverge and produce a
  wrong length.
- **Status**: CONFIRMED (the "no effect" claim, directly demonstrated) as a code-quality/dead-code
  finding, not a memory-safety CONFIRMED. It is worth fixing (one line: drop the parameter, or
  assert it against the header-derived length in debug builds) because a future caller who *did*
  pass a mismatched arity, expecting it to matter, would get no compile error, no debug-mode
  assertion, and no runtime signal — only a silently-correct-anyway result today, and undefined
  behaviour the moment some future refactor makes any code path trust the passed-in length instead
  of (or in addition to) the header.
- **Fix direction**: either remove the parameter (since it is provably unused after erasure), or
  add a `debug_assert_eq!(unsafe { ptr.deref() }.arity_via_header(), arity)`-style check at the
  call sites so a future mismatch fails loudly instead of silently working by coincidence.

### 3. `SharedSymbol`'s `BlockAllocatorSafe` impl depends on a `#[repr(Rust)]` field-order fact the type does not guarantee — PLAUSIBLE

- **Location**: `crates/aterm/src/storage/symbol_pool.rs` (`unsafe impl BlockAllocatorSafe for
  SharedSymbol {}`), contrasted with `crates/aterm/src/storage/aterm_storage.rs`'s
  `SharedTermFixed<N>`/`SharedTermInt`, which are both `#[repr(C)]`.
- **Scenario**: `BlockAllocatorSafe` requires the first `size_of::<*mut _>()` bytes of every live
  value to never equal the allocator's sentinel (`usize::MAX`, all bytes `0xFF`) — see
  `merc_unsafety::block_allocator::BlockAllocatorSafe`'s own `# Safety` doc. `SharedSymbol { name:
  String, arity: usize }` is **not** `#[repr(C)]`, so the compiler is free to place either field
  first. With the layout the current toolchain actually produces (`name` first, verified below),
  the property holds for the right reason (`name`'s internal heap pointer is never
  `usize::MAX`). But nothing pins that layout: if a future compiler version (or a different
  codegen configuration) placed `arity: usize` first instead, the property would instead depend on
  `arity` never reaching exactly `usize::MAX` — a value `Symbol::new`/`SharedSymbol::new` accept
  with no bound at all.
- **Evidence for the current layout** (empirical, both standalone and in-crate):

  ```
  # standalone rustc, matching struct shape
  offset of name: 0
  offset of arity: 24
  size: 32
  ```

  and, pinned as a regression guard in the crate itself
  (`crates/aterm/src/storage/symbol_pool.rs`'s `mod tests`):
  `const _: () = assert!(offset_of!(SharedSymbol, name) == 0);` — confirmed to compile (i.e. hold)
  via `cargo check -p merc_aterm --all-targets`.
- **Why PLAUSIBLE and not CONFIRMED**: I cannot force rustc to choose the other field order to
  demonstrate an actual failure; there is no way to make this fail executably without controlling
  the compiler's internal field-reordering heuristic, which is deliberately unspecified for
  `#[repr(Rust)]`.
- **Status**: PLAUSIBLE.
- **Fix direction** (one line, not applied — production-code change beyond this review's
  charter): add `#[repr(C)]` to `SharedSymbol`, matching `SharedTermFixed`/`SharedTermInt`'s
  existing precedent in the same crate. The `offset_of!` assertion I added would then express a
  guarantee instead of merely observing the status quo, and would need no further changes.

## Kani proof harnesses (new)

`crates/aterm/Cargo.toml` gained the `[package.metadata.kani]` block (copied verbatim from
`crates/unsafety/Cargo.toml`, per the task brief). Five new harnesses were added, following
`crates/unsafety/src/freelist.rs`'s pattern (contracts/attributes on the target fn where
applicable, `#[cfg(kani)] mod verification` at the bottom of the same file). All five build real
pointer/memory operations from minimal, self-contained fixtures (leaked heap allocations built
with the crate's own `StablePointer`/`Erasable`/`SliceDst` machinery) — none of them touch
`THREAD_TERM_POOL` or the global pool's locking, which is exactly what the task brief flagged as
too large to bound cheaply. `SymbolRef::from_index`/`ATermRef::from_index` and the various
`Transmutable` impls were deliberately *not* given their own contract-checking harnesses beyond
what's below: they are pure pointer copies / same-layout lifetime relabels with no real pointer
arithmetic, so a Kani proof of them would be close to vacuous (the task brief's explicit guidance
against padding the count with marker-impl-style proofs) — I gave them targeted Miri boundary
tests and mathematical `# Safety` contracts instead (next section).

1. **`crates/aterm/src/symbol.rs`**, `symbol_ref_from_index_preserves_identity_and_reads` — proves
   `SymbolRef::from_index` (`symbol.rs:69`) produces a reference whose pointer identity matches
   the source `SymbolIndex` exactly, and whose `arity()`/`name()` reads observe the same
   `SharedSymbol` the index pointed at.
2. **`crates/aterm/src/storage/aterm_storage.rs`**,
   `cast_to_shared_term_ptr_arity_zero_preserves_address_and_length` — proves the private
   `cast_to_shared_term_ptr` helper (see finding #2 above) reconstructs a `SharedTerm` fat pointer
   at exactly the source address, with an empty `arguments()` slice, for a zero-arity source.
3. **`crates/aterm/src/storage/aterm_storage.rs`**,
   `cast_to_shared_term_ptr_with_one_argument_preserves_argument_identity` — same, for a
   populated argument slot: `arguments()[0]`'s address matches the argument's original pointer
   exactly.
4. **`crates/aterm/src/storage/aterm_storage.rs`**,
   `value_unchecked_pattern_reads_back_stored_annotation` — proves the *exact* expression behind
   `ATermInt::value_unchecked` (`aterm_int.rs:81`,
   `self.shared().ptr_with_len(0).cast::<SharedTermInt>().as_ref().value()`) reads back the value
   written into `annotation`, for a Kani-symbolic `usize` covering the **full representable
   range** — including the boundary values `0` and `usize::MAX` — not just a couple of sampled
   points.
5. **`crates/aterm/src/storage/shared_term.rs`**,
   `shared_term_construct_then_read_roundtrips_one_argument` — proves `SharedTerm::construct`
   (the `SliceDst`/`Erasable`-driven write path `ATermStorage::insert`'s unbounded-arity table
   uses) followed by `symbol()`/`arguments()` observes exactly what was written, for a
   one-argument term.

Run (from `crates/aterm`):

```
cargo kani
```

Real output (full run, no `--harness` filter, confirming all 5 and no regressions elsewhere in
the crate):

```
Checking harness storage::aterm_storage::verification::value_unchecked_pattern_reads_back_stored_annotation...
...
VERIFICATION:- SUCCESSFUL
Checking harness storage::aterm_storage::verification::cast_to_shared_term_ptr_with_one_argument_preserves_argument_identity...
...
VERIFICATION:- SUCCESSFUL
Checking harness storage::aterm_storage::verification::cast_to_shared_term_ptr_arity_zero_preserves_address_and_length...
...
VERIFICATION:- SUCCESSFUL
Checking harness storage::shared_term::verification::shared_term_construct_then_read_roundtrips_one_argument...
...
VERIFICATION:- SUCCESSFUL
Checking harness symbol::verification::symbol_ref_from_index_preserves_identity_and_reads...
...
VERIFICATION:- SUCCESSFUL

Complete - 5 successfully verified harnesses, 0 failures, 5 total.
```

No genuine violation was found by any harness (unlike the mid-task GC reentrancy discovery, which
came from applying the `with_mut` migration, not from Kani).

## Miri boundary tests + mathematical `# Safety` contracts (mid-task addition)

For unsafe surface judged *not* a good Kani target (bare `Send`/`Sync` marker impls especially,
and the `Transmutable` lifetime-relabel impls, whose only "pointer arithmetic" is a same-layout
`mem::transmute`), I added targeted, deterministic Miri boundary tests plus precise mathematical
`# Safety` contracts in place of prose hand-waving, per the mid-task instruction.

### Boundary tests added

All in `crates/aterm/tests/miri_aterm.rs` unless noted; run with:

```
MIRIFLAGS="-Zmiri-disable-isolation --cfg chacha20_force_soft" \
  cargo +nightly miri nextest run --no-fail-fast -p merc_aterm
# (or: cargo +nightly miri test -p merc_aterm --test miri_aterm --test aterm_int_test)
```

- `test_boundary_zero_arity_term_has_no_arguments` — empty case: an arity-0 term's `ATermArgs`
  iterator is empty in both directions, no underflow in `next_back`'s `self.arity -= 1`.
- `test_boundary_max_fixed_arity_and_first_dynamic_arity` — the `MAX_FIXED_ARITY == 7` boundary:
  arity 7 (last fixed-size table, `terms_7`) and arity 8 (first symbol to fall through to the
  dynamically sized `terms` table) both construct and read back correctly.
- `test_boundary_shared_argument_aliasing` — `f(a, a)`: `arg(0)`/`arg(1)` are the same interned
  node; reads through both aliases at once, exercised under Miri's Stacked/Tree Borrows checker.
- `test_boundary_arg_at_last_valid_index_succeeds` / `test_boundary_arg_one_past_last_valid_index_panics`
  — the exact end of the valid argument-index region: `arg(arity - 1)` succeeds,
  `arg(arity)` panics via the ordinary checked slice index (not silent out-of-bounds UB).
- `test_boundary_transmutable_empty_containers` — empty-collection boundary for `Transmutable`:
  `Vec`, `VecDeque`, `Option::None`, and an empty slice all transmute-lifetime cleanly with no
  element ever read.
- `test_boundary_transmutable_single_element_preserves_identity` — single-element boundary: a
  one-element `Vec<ATermRef<'static>>` preserves the element's identity (pointer/index) across the
  lifetime shrink.
- `crates/aterm/tests/aterm_int_test.rs`,
  `test_boundary_aterm_int_min_and_max_value_round_trip` — the payload
  `ATermInt::value_unchecked` reads via a raw pointer cast round-trips at both representable
  extremes (`0`, `usize::MAX`) through the *real* public API (`ATermInt::new`/`.value()`), as a
  deterministic complement to Kani harness #4 above (which covers the full symbolic range but
  through a lower-level, hand-built fixture, not the real term pool).
- `crates/aterm/tests/aterm_int_test.rs`,
  `test_boundary_reserved_name_at_different_arity_is_not_an_int_term` — the reserved-name-guard
  boundary: `<aterm_int>` at a *different* arity than the reserved marker (0) is a legitimate,
  distinct symbol, and `is_int_term` correctly does not mistake a term built from it for a real
  integer term (which has no `annotation` for `value_unchecked` to read).
- `crates/aterm/src/storage/aterm_storage.rs`,
  `probe_cast_to_shared_term_ptr_arity_argument_effect` — see finding #2.
- `crates/aterm/src/storage/symbol_pool.rs`, `offset_of!(SharedSymbol, name) == 0` — see finding
  #3 (a compile-time regression guard, not a runtime test).

Real output (the full set above, run together under Miri; the `f(a, a)` aliasing test's log line
is the interesting one — 15 tests, all passing, no Stacked/Tree Borrows violation):

```
running 15 tests
test test_aterm_args_size_hint_is_exact ... ok
test test_boundary_shared_argument_aliasing ... ok
test test_boundary_transmutable_empty_containers ... ok
test test_boundary_transmutable_single_element_preserves_identity ... ok
test test_boundary_zero_arity_term_has_no_arguments ... ok
test test_boundary_arg_at_last_valid_index_succeeds ... ok
test test_boundary_max_fixed_arity_and_first_dynamic_arity ... ok
test test_miri_binary_writer_survives_gc ... ok
test test_miri_global_protected_send_across_threads ... ok
test test_miri_maximal_sharing ... ok
test test_miri_send_roundtrip ... ok
test test_miri_term_iterator ... ok
test test_miri_protection_survives_gc ... ok
test test_miri_term_arguments ... ok
test test_boundary_arg_one_past_last_valid_index_panics - should panic ... ok

test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

(This particular run predates the `with_mut` migration; none of these tests touch
`Protected`/`ProtectedSend` + `collect_garbage` together, so they are unaffected by finding #1 and
still pass identically afterward — reconfirmed with a plain, non-Miri `cargo test` run after the
migration landed, see the "sharedmutex migration" section.)

### Mathematical `# Safety` contracts added (doc-comment only, no logic changes)

Marker `Send`/`Sync` impls cannot be proved by a test or a Kani harness (per the mid-task
instruction: "a test can't prove a marker trait"); each was instead given a precise contract
stating exactly which field makes it not automatically `Send`/`Sync`, and why sharing/moving is
sound given the rest of the type's invariants:

- `crates/aterm/src/aterm.rs`, `unsafe impl Send/Sync for ATermRef<'_>` — the blocking field is
  `shared: StablePointer<SharedTerm>` (a raw, `!Send`/`!Sync` pointer by default); soundness rests
  on (1) every `SharedTerm` being immutable once inserted (`ATermStorage::insert` only ever writes
  fresh allocations; the only other writer, the sweep, removes whole entries) and (2) the address
  only being reclaimed under `GlobalTermPool`'s exclusive write lock, which cannot be held while
  any thread holds a live, in-bounds `ATermRef` (a `read_recursive()` guard, or an equivalent
  protection-set registration).
- `crates/aterm/src/aterm.rs`, `unsafe impl Send/Sync for ATermSend` — shown to need no *new*
  reasoning beyond `ATermRef`'s (every field is `Send`/`Sync` once `ATermRef`'s own impl is
  accepted); the impl exists only because a struct containing an unasserted `!Send`/`!Sync` field
  anywhere in its transitive closure (`SharedTerm`, self-referentially, via `ATermIndex`) cannot
  have the traits auto-derived, even though every individual field's *own* requirement is already
  satisfied.
- `crates/aterm/src/storage/gc_mutex.rs`, `unsafe impl<T: Send> Send` /
  `unsafe impl<T: Send + Sync> Sync for GcMutex<T>` — blocking field is `inner: UnsafeCell<T>`
  (`!Sync` unconditionally); soundness for `Sync` rests on `GcMutex::lock`/`lock_mut`'s two guard
  types enforcing the same shared-xor-exclusive discipline an ordinary `RwLock` would (`lock_mut`
  takes `&mut self`, so the borrow checker already excludes overlapping guards from one handle).
- `crates/aterm/src/storage/global_aterm_pool.rs`, `unsafe impl Sync/Send for ThreadPoolList` —
  blocking field is `Arc<UnsafeCell<SharedTermProtection>>` (never auto-`Sync`, and therefore never
  auto-`Send` either via `Arc`'s own bound); soundness rests on the two access paths (each
  thread's own cell access, and the collector's cross-thread walk during marking) being made
  mutually exclusive by the surrounding `RecursiveLock`/`GlobalBfSharedMutex`, not by anything
  intrinsic to `UnsafeCell` — the same temporal-exclusivity argument underlying finding #1, spelled
  out explicitly here as a contract for the first time.
- `crates/aterm/src/storage/symbol_pool.rs` and `crates/aterm/src/storage/aterm_storage.rs`, the
  three `BlockAllocatorSafe` impls — restated against the trait's own two required properties
  (sentinel-avoidance, full initialization) instead of the previous prose ("heap pointers are
  always well above the small alignment value returned by `dangling_mut()`", which does not
  actually establish avoidance of the real sentinel, `usize::MAX`, an unrelated high value — a
  latent comment-accuracy issue independent of finding #3, now corrected); see finding #3 for the
  `SharedSymbol` case specifically.
- `crates/aterm/src/transmutable.rs`, the `Transmutable` trait itself and both of its methods —
  restated as two precise implementor-side properties (layout identity; consistent nested
  targets) and, on the unsafe methods, an explicit caller-side temporal requirement ("`'a` must
  not outlive the *actual* GC-root protection of the reachable `ATermRef`/`SymbolRef`s, not merely
  the `&self` borrow used to call this method") instead of the previous one-line "must ensure `'a`
  does not outlive the borrow of `self`", which technically already said the essential thing but
  did not spell out that nothing in the signature enforces it or what "outlive" means for a GC-rooted
  reference.

## `merc_sharedmutex` `RecursiveLockWriteGuard::with_mut` migration (mid-task addition)

Per the coordinator's mid-task instruction, all four call sites in
`crates/aterm/src/storage/thread_aterm_pool.rs` that previously mutated through
`RecursiveLockWriteGuard`'s (now-removed) `DerefMut` were migrated to the new
`with_mut(impl FnOnce(&mut T) -> R) -> R` API:

- `ThreadTermPool::new` (`:87-90`): `pool.register_thread_term_pool()` → `pool.with_mut(|pool|
  pool.register_thread_term_pool())`.
- `ThreadTermPool::automatic_garbage_collection` (`:465-467`): wrapped in `with_mut`.
- `ThreadTermPool::force_collect_garbage` (`:471-478`): both `collect_garbage()`/
  `reset_gc_budget()` calls wrapped in one `with_mut`.
- `ThreadTermPool::collect_garbage` (`:482-489`): `trigger_garbage_collection()` wrapped in
  `with_mut`.
- `Drop for ThreadTermPool` (`:598-622`): this one *does* mutate through the guard
  (`protect_orphan_roots`, `deregister_thread_pool`, both `&mut self`) — the coordinator's message
  asked me to check this specifically; it needed the same treatment and now has it, with the
  `debug!("{}", write.metrics())` call (immutable, `&self`) kept between the two `with_mut` calls
  to preserve the original ordering.

`cargo check -p merc_aterm --all-targets` is clean (no warnings) after the migration. However,
`cargo test -p merc_aterm --lib` (and any integration test exercising the same pattern) now
**aborts the process** — this is finding #1 above, not a mechanical mistake in the migration
itself (the migration is a faithful, minimal port of exactly the calls that needed it; I did not
find a different `with_mut` placement that avoids the panic, because the conflict is inside
`GlobalTermPool::mark_roots`/`collect_garbage`, which I did not modify). **This is unresolved and
needs the implementor's attention**; per the review charter I have not attempted the
architecture-level fix (see finding #1's "Fix direction").

Kani proofs are unaffected by this migration (none of the 5 harnesses touch `THREAD_TERM_POOL` or
`RecursiveLock`; reconfirmed with a full `cargo kani` run after the migration, same 5/5 result as
above). The Miri/plain-`cargo test` boundary tests added in the previous section are likewise
unaffected (none of them combine a live `Protected`/`ProtectedSend` container with a
`collect_garbage()` call), reconfirmed with:

```
cargo test -p merc_aterm --test miri_aterm --test aterm_int_test
# running 15 tests ... test result: ok. 15 passed; 0 failed
# running 4 tests ... test result: ok. 4 passed; 0 failed
```

## Checked and found correct

- **The `ATermStorage`/`SharedTermFixed<N>`/`SharedTermInt`/`SharedTerm` pointer-aliasing scheme**
  (`aterm_storage.rs`, `shared_term.rs`): traced every cast in `cast_to_shared_term_ptr`,
  `SharedTerm::construct`, `Erasable`/`SliceDst` for `SharedTerm`, and the `offset_of!` static
  assertions that pin `symbol`'s position across all three representations. Confirmed correct
  (modulo finding #2's dead parameter, which does not affect correctness today) by both Kani
  proofs #2–#4 and the existing/new Miri tests.
- **`ATermInt::value_unchecked`'s raw pointer cast** (`aterm_int.rs:81`): the exact expression is
  now Kani-proved for the full symbolic `usize` range (harness #4) and Miri-tested at both
  boundary values through the real public API.
- **The GC protection-set / orphan-adoption machinery in `thread_aterm_pool.rs`/
  `global_aterm_pool.rs`** *up to but not including* the container-marking reentrancy in finding
  #1: `ThreadTermPool::drop`'s ordering (adopt orphan roots under the write lock, then
  deregister, both now correctly under the same lock as before) is sound; the existing
  `test_orphaned_thread_term_remains_readable_after_gc` and `test_send_term_outlives_creating_thread`
  tests (neither of which touches a `Protected`/`ProtectedSend` container) still pass after the
  `with_mut` migration.
- **`Protected::write`/`ProtectedSend::write`'s raw-pointer `&mut GcMutex<C>` construction from a
  shared `Arc`** (`protected.rs:58`, `:163`): specifically scrutinized as a plausible aliasing
  hazard (a `&mut` conjured from `Arc::as_ptr`, a shared pointer). Confirmed sound: `GcMutex::lock_mut`
  internally takes a `read_recursive()` guard on the *same* `RecursiveLock<GlobalTermPool>` that
  `collect_garbage` needs the *write* lock for, so no GC pass (and hence no concurrent access to
  the same container through a different path) can be in progress while a `ProtectedWriteGuard` is
  alive — the temporal-exclusivity argument holds, backed by the pre-existing multi-threaded Miri
  tests (`test_miri_global_protected_send_across_threads`, which forces GC from one thread while a
  `ProtectedSend` created on another is read) showing no data-race flag.
- **`SymbolRef`'s absence of an explicit `Send`/`Sync` impl**: verified this is *not* a gap.
  `SharedSymbol { name: String, arity: usize }` has no recursive reference back to
  `ATermRef`/`SymbolRef` (unlike `SharedTerm`), so `StablePointer<SharedSymbol>`'s conditional
  `Send`/`Sync` impls (from `merc_unsafety`, bounded on `T: Send`/`T: Send + Sync`) discharge
  automatically, and so does `SymbolRef<'a>`'s.
- **`ATermArgs`'s `DoubleEndedIterator`/`ExactSizeIterator`/`size_hint` implementation**: traced
  interleaved `next()`/`next_back()` calls (both directions, including exhausting one after the
  other) for underflow in `self.arity -= 1`; none found. The existing
  `test_aterm_args_size_hint_is_exact` regression test already covers the `size_hint`/`len`
  half; the new zero-arity and last-valid-index tests cover the remaining edges.
- **The binary stream reader's handling of adversarial/corrupt input**
  (`aterm_binary_stream.rs`): `symbol_index`/`arg_index` are always bounds-checked
  (`.get(..).ok_or(..)`) before use, never trusted as in-bounds; the packet-type decode's
  `unreachable!()` is genuinely unreachable (the caller always reads exactly `PACKET_BITS == 2`
  bits first); `resolve_read_symbol` cannot be tricked into forging the reserved `<aterm_int>`
  symbol at its reserved arity (only an exact `(name, arity)` match reuses it; see also the new
  `test_boundary_reserved_name_at_different_arity_is_not_an_int_term`).
- **`merc_derive_terms`'s generated `Transmutable` impl for every `#[merc_term]` `..Ref<'a>` type**
  (`crates/macros/src/merc_derive_terms.rs`): the generated impl is `unsafe { transmute::<&Self,
  &'a Name_Ref<'a>>(self) }` — the same pattern as every hand-written impl in `transmutable.rs`,
  monomorphized to the same generic struct with only the lifetime parameter varying, which has no
  effect on layout. Read, not independently re-proved (out of `crates/aterm`'s own scope, and
  `crates/macros` is a different crate), but consistent with the layout-identity contract now
  stated on the `Transmutable` trait.
- **Miri baseline** (pre-`with_mut`-migration, full crate): `MIRIFLAGS="-Zmiri-disable-isolation
  --cfg chacha20_force_soft" cargo +nightly miri nextest run --no-fail-fast -p merc_aterm` — 29/29
  tests passed (including the pre-existing multi-threaded `ATermSend`/`ProtectedSend` tests), no
  Stacked/Tree Borrows violation, no leak.

Verifiers run: **Miri** (baseline + all new boundary tests, and the isolated finding-#1 repro),
**Kani** (5 new harnesses, full-crate run). **Loom** and the **address/thread sanitizers** were
not run: nothing in `crates/aterm`'s own unsafe surface is loom-gated (the crate's concurrency
correctness rests on `merc_sharedmutex`'s `RecursiveLock`/`GlobalBfSharedMutex`, which is that
crate's own review's responsibility, not duplicated here), and the sanitizer runs were skipped for
time/scope — Miri's data-race detector already exercises the same multi-threaded
`ATermSend`/`ProtectedSend` paths a TSan run would target, and no new concurrency-shaped unsafe
code was added by this review beyond doc comments.

## Tests / proofs added

- `crates/aterm/Cargo.toml` — `[package.metadata.kani]` block (the only Cargo.toml change).
- `crates/aterm/src/symbol.rs` — `#[cfg(kani)] mod verification` (1 harness).
- `crates/aterm/src/storage/aterm_storage.rs` — `#[cfg(kani)] mod verification` (3 harnesses),
  plus `probe_cast_to_shared_term_ptr_arity_argument_effect` in the existing `mod tests`.
- `crates/aterm/src/storage/shared_term.rs` — `#[cfg(kani)] mod verification` (1 harness).
- `crates/aterm/src/storage/symbol_pool.rs` — `offset_of!(SharedSymbol, name) == 0` static
  assertion in the existing `mod tests`.
- `crates/aterm/tests/miri_aterm.rs` — 8 new boundary tests (listed above).
- `crates/aterm/tests/aterm_int_test.rs` — 2 new boundary tests (listed above).
- `crates/aterm/tests/gc_reentrant_container_marking_test.rs` — new file, 1 `#[ignore]`d
  regression test for finding #1 (left failing/ignored in the tree, as it demonstrates a defect
  this review does not fix; ignored specifically because running it aborts its process, so it is
  isolated in its own test binary to avoid taking down other targets).

How to run everything:

```
# Kani (from crates/aterm):
cd crates/aterm && cargo kani

# Plain tests (fast, excludes the intentionally-`#[ignore]`d finding-#1 repro):
cargo test -p merc_aterm --test miri_aterm --test aterm_int_test
cargo test -p merc_aterm --lib probe_cast_to_shared_term_ptr_arity_argument_effect test_symbol_sharing test_shared_term_lookup

# Miri, the same boundary tests:
MIRIFLAGS="-Zmiri-disable-isolation --cfg chacha20_force_soft" \
  cargo +nightly miri test -p merc_aterm --test miri_aterm --test aterm_int_test

# Finding #1's isolated repro (aborts the process by design -- run on its own):
cargo test -p merc_aterm --test gc_reentrant_container_marking_test -- --ignored --test-threads=1 --nocapture

# NOT currently green -- see finding #1:
cargo test -p merc_aterm --lib
```

No production-code *logic* was changed by this review: the only non-doc-comment,
non-test/proof changes are (a) the four `with_mut` call-site migrations in
`thread_aterm_pool.rs`, explicitly directed mid-task to keep the crate compiling against the
fixed `merc_sharedmutex` API, and (b) the `Cargo.toml` kani metadata block. Everything else is
`#[cfg(kani)]` proof code, `#[cfg(test)]` test code, or `# Safety`/doc-comment text.
