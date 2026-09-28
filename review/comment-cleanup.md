# Comment cleanup — trimming Phase 1/1b `# Safety` comments

Verified every claim in each extensive `# Safety`/`SAFETY:` comment added
during Phase 1/1b before trimming it to a concise form; comments whose claims
did not hold were left untrimmed and are listed as findings below instead.

Covered: `crates/sharedmutex`, `crates/unsafety`, `crates/aterm`,
`crates/sabre`, `crates/sabre_compiling`, `tools/mcrl2/crates/mcrl2/src/atermpp`.

## Findings surfaced during the pass (not blindly trimmed)

### 1. `Send for ThreadLocalAllocState` cites a false property — FIXED (comment corrected)

- **Location**: `crates/unsafety/src/block_allocator.rs:461-477` (now
  `458-485` after the rewrite).
- **Claim checked**: the comment asserts `FreeList<Entry<T>>` is `Send` for
  `T: Send` "see `FreeList`'s own `unsafe impl Send`". False: `Entry<T>`'s
  `next: ManuallyDrop<*mut Entry<T>>` field is a raw pointer that blocks the
  auto-trait unconditionally, and no `unsafe impl Send for Entry<T>` exists.
  Confirmed with a throwaway `fn probe<T: Send>() { probe::<FreeList<Entry<u32>>>() }`
  under `cargo check --tests` (fails with `E0277`, the expected `Send` bound
  error — `Entry<T>`'s `ManuallyDrop<*mut Entry<T>>` is what's unsatisfied).
- **Status**: FIXED. The outer manual `unsafe impl Send for ThreadLocalAllocState`
  *is* sound — traced independently of the false citation, on the grounds the
  comment already gestured at: `ThreadLocal::get_or` gives thread-affine access
  to `current_block`/`bump_offset`/`free` for `allocate_object`/
  `deallocate_object`, the one exception being `remove_free_blocks` (via
  `alloc_state.iter_mut()`, `&mut self`), which reads/clears another thread's
  `free` list but is fenced by that method's own "must not run concurrently
  with allocation/deallocation" contract; and the value's own final drop
  (from whichever thread drops the owning `ThreadLocal`) dereferences none of
  `current_block`/`free`'s pointers. The comment now states this real
  argument instead of the false `FreeList`-auto-Send citation, and explicitly
  says the field is blocked from auto-`Send` unconditionally (like
  `current_block`), so the impl below is a fully manual claim, not one
  inherited from any field.

### 2. `Send`/`Sync` for `BlockAllocator` cites the same false pattern — FIXED (comment corrected)

- **Location**: `crates/unsafety/src/block_allocator.rs:503-510` (now
  `~517-533` after the rewrite).
- **Claim checked**: asserts `Mutex<BlockList<T, N>>` is `Send`/`Sync`
  automatically given `T: Send`. False for the same reason:
  `BlockList<T, N>`'s `head_block: Option<NonNull<Block<T,N>>>` and
  `free_chunks: Vec<NonNull<Entry<T>>>` block auto-`Send` unconditionally, and
  no manual `Send` impl for `BlockList` exists. Confirmed the same way
  (`cargo check --tests` on `Mutex<BlockList<u32, 4>>: Send`).
- **Status**: FIXED. The outer manual `unsafe impl Send/Sync for BlockAllocator`
  is sound on the grounds the comment already partly stated: every access to
  `head_block`/`free_chunks` happens while the mutex is held (mutual
  exclusion), and `T: Send` is what licenses the `T` values those blocks/
  entries own to be safely read, written or dropped by whichever thread
  currently holds the lock, regardless of which thread allocated them — not
  because `Mutex<BlockList<T,N>>` is itself auto-`Send`/`Sync` (it never is,
  with or without `T: Send`). The comment now states this and explicitly
  flags that `Mutex<X>`'s conditional impls never actually fire for this
  field, so the outer impl is a fully manual claim.

### 3. `StablePointer::ptr()` reads the pointee without an `unsafe` marker — FIXED (now `unsafe fn`)

- **Location**: `crates/unsafety/src/stable_pointer_set.rs:106-124` (the
  `ptr()` method), `:153-167` (the `Send`/`Sync` comment), and
  `crates/unsafety/src/erasable.rs:47-68` (`Thin::as_nonnull`, the actual root
  cause `ptr()` delegates to).
- **Scenario confirmed real, not just a doc gap**: added a throwaway test type
  (`HeaderDst`, a `SliceDst`+`Erasable` DST mirroring `SharedTerm`'s pattern —
  length stored in the pointee's own header) plus a test that allocates one,
  wraps it in a `StablePointer`, deallocates the backing memory, then calls
  the then-*safe* `.ptr()` with **no** `unsafe` at the call site. Under Miri:
  ```
  MIRIFLAGS="-Zmiri-disable-isolation" cargo +nightly miri test -p merc_unsafety \
    ptr_reads_freed_pointee_without_an_unsafe_marker
  ```
  ```
  error: Undefined Behavior: memory access failed: alloc56912 has been freed, so this pointer is dangling
     --> crates/unsafety/src/stable_pointer_set.rs:813:27
      |
  813 |                 let len = std::ptr::read(this.as_ptr().cast::<usize>());
      |                           ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ Undefined Behavior occurred here
  stack backtrace:
    0: <HeaderDst as Erasable>::unerase
    1: Thin::<HeaderDst>::as_nonnull
    2: StablePointer::<HeaderDst>::ptr        <-- reached via a completely safe call site
    3: ptr_reads_freed_pointee_without_an_unsafe_marker
  ```
  This is exactly `SharedTerm::unerase`'s own pattern
  (`crates/aterm/src/storage/shared_term.rs`, `ptr::read` to recover
  `symbol.arity()`), so the same use-after-free is reachable through
  `ATermIndex::ptr()` for a `StablePointer<SharedTerm>` that has been removed
  from its owning set. (This throwaway test/type was used only to gather this
  evidence and was not committed — the compile-fail test below is the
  permanent regression artifact.)
- **Fix**: `Thin::as_nonnull` (the root cause) and `StablePointer::ptr` are now
  both `unsafe fn`, with a `# Safety` contract identical in spirit to
  `deref`'s ("the element must still be present in its owning set"). Every
  call site across `crates/unsafety`, `crates/aterm`, `crates/sabre_compiling`
  and `crates/sabre_compiling/sabre_ffi` was updated with an explicit
  `unsafe` block and a `SAFETY` comment justifying why the pointee is live at
  that call (all were already operating on live data — no behavior changed,
  only the API's safety marker). A new `trybuild` compile-fail regression
  test (`crates/unsafety/tests/build_tests.rs` +
  `tests/input/stable_pointer_ptr_requires_unsafe.rs`) proves `ptr()` can no
  longer be called without `unsafe`.
- **Impact reassessed**: no live exploit existed (every caller already held a
  live handle), but this was a genuine latent soundness gap in a *safe*
  public function, not merely a stale doc claim — the Miri reproduction above
  confirms it was reachable, not just plausible.
- **Full verification** (completed after the fix landed, once container
  resource contention that blocked it earlier had cleared): `cargo check -p
  merc_unsafety -p merc_aterm -p merc_sabre-compiling -p merc_sabre-ffi
  --all-targets` clean; `cargo test -p merc_unsafety --lib` 30/30 pass;
  `cargo test -p merc_aterm --lib` 23/23 pass; the new `trybuild` regression
  test (`cargo test -p merc_unsafety --test build_tests`) generated and
  confirmed its golden `.stderr` matches exactly the intended
  `error[E0133]: call to unsafe function` StablePointer::<T>::ptr` is unsafe
  and requires unsafe block`; `cargo clippy` clean for every touched file;
  `cargo +nightly fmt --all -- --check` clean; `MIRIFLAGS="-Zmiri-disable-isolation"
  cargo +nightly miri test -p merc_unsafety` — 20/20 pass, 0 failed; `cd
  crates/aterm && cargo kani` — 6/6 harnesses verified, 0 failures (covers
  the edited `#[cfg(kani)]` proof-module `unsafe` wraps). The two
  `merc_sabre-compiling` test failures seen when run in combination with
  the other three crates (`test_sabre_compiling_example`,
  `test_sabre_compiling_survives_garbage_collection_between_calls`) were
  confirmed to be a pre-existing test-isolation issue (both tests shell out
  to `cargo build` into the same `./tmp` directory and stomp on each other
  under parallel execution) — both pass individually and under
  `--test-threads=1`, unrelated to this fix.

### 4. `Debug for GlobalTermPool` races with a concurrent `write_exclusive` — upgraded from PLAUSIBLE to CONFIRMED

- **Location**: `tools/mcrl2/crates/mcrl2/src/atermpp/busy_forbidden.rs`
  (`BfTermPool::write_exclusive`'s `# Safety` comment) and
  `tools/mcrl2/crates/mcrl2/src/atermpp/thread_aterm_pool.rs:423-430`
  (`Display`/`Debug`, reached from `GlobalTermPool`'s `Debug`).
- **Scenario**: `Debug`/`Display` calls `.read()` on *every* thread's
  protection set, including one concurrently mutated via `write_exclusive`
  from another thread (e.g. inside `protect_with` during term creation) —
  violating the busy/forbidden protocol's "no other thread may `read()` while
  a `write_exclusive` guard is live" invariant.
- **Evidence**: an existing test, `thread_aterm_pool.rs`'s
  `read_races_with_concurrent_term_creation`, was already written specifically
  to reproduce this under a thread sanitizer.
- **Status**: was previously listed as "Plausible ... unreachable in practice
  (no current callers)"; the comment-trim pass confirmed it is real and
  reproducible, not merely theoretical (see `review/README.md`'s findings
  table, updated accordingly). Still not fixed.

## Verification

- `cargo check -p merc_sharedmutex -p merc_unsafety --all-targets`: passed.
- `cargo test -p merc_sharedmutex -p merc_unsafety --lib`: 21 + 30 passed.
- `cargo test -p merc_aterm --lib`: 23 passed.
- `cargo test -p merc_sabre -p merc_sabre-compiling --lib`: 26 + 4 passed;
  `cd crates/sabre && cargo kani`: 3/3 harnesses verified.
- `cd tools/mcrl2 && cargo check -p mcrl2 --lib`: passed.
- `cargo clippy -p merc_aterm --all-targets`, `cargo +nightly fmt -p merc_aterm
  -p merc_sharedmutex -p merc_unsafety -p merc_sabre -p merc_sabre-compiling
  -- --check`: clean (one pre-existing, unrelated formatting diff in
  `concurrent_append_vec.rs` test/kani code, predating this pass).
