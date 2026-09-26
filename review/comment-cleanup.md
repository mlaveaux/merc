# Comment cleanup — trimming Phase 1/1b `# Safety` comments

Verified every claim in each extensive `# Safety`/`SAFETY:` comment added
during Phase 1/1b before trimming it to a concise form; comments whose claims
did not hold were left untrimmed and are listed as findings below instead.

Covered: `crates/sharedmutex`, `crates/unsafety`, `crates/aterm`,
`crates/sabre`, `crates/sabre_compiling`, `tools/mcrl2/crates/mcrl2/src/atermpp`.

## Findings surfaced during the pass (not blindly trimmed)

### 1. `Send for ThreadLocalAllocState` cites a false property — PLAUSIBLE, comment not trimmed

- **Location**: `crates/unsafety/src/block_allocator.rs:461-477`.
- **Claim checked**: the comment asserts `FreeList<Entry<T>>` is `Send` for
  `T: Send` "see `FreeList`'s own `unsafe impl Send`". False: `Entry<T>`'s
  `next: ManuallyDrop<*mut Entry<T>>` field is a raw pointer that blocks the
  auto-trait unconditionally, and no `unsafe impl Send for Entry<T>` exists.
  Confirmed with a throwaway `fn probe<T: Send>() { probe::<FreeList<Entry<u32>>>() }`
  under `cargo check --tests` (fails with the expected `Send` bound error).
- **Status**: the outer manual `unsafe impl Send for ThreadLocalAllocState` may
  still be sound on other grounds (thread-affinity via `ThreadLocal::get_or`,
  a no-op `Drop` path), but its stated justification does not type-check.
  Left untrimmed rather than compressed on a false premise.

### 2. `Send`/`Sync` for `BlockAllocator` cites the same false pattern — PLAUSIBLE, comment not trimmed

- **Location**: `crates/unsafety/src/block_allocator.rs:503-510`.
- **Claim checked**: asserts `Mutex<BlockList<T, N>>` is `Send`/`Sync`
  automatically given `T: Send`. False for the same reason:
  `BlockList<T, N>`'s `head_block: Option<NonNull<Block<T,N>>>` and
  `free_chunks: Vec<NonNull<Entry<T>>>` block auto-`Send` unconditionally, and
  no manual `Send` impl for `BlockList` exists. Confirmed the same way
  (`cargo check --tests` on `Mutex<BlockList<u32, 4>>: Send`).
- **Status**: plausibly still sound (every access is behind the mutex), stated
  justification wrong. Left untrimmed.

### 3. `StablePointer::ptr()` reads the pointee without an `unsafe` marker — PLAUSIBLE, new finding

- **Location**: `crates/unsafety/src/stable_pointer_set.rs:137-148` (the
  `Send`/`Sync` comment and the type's own doc, both claiming only the
  `unsafe fn deref` reads the pointee).
- **Scenario**: `StablePointer::ptr()` is a **safe** method, but for any `T`
  whose `Erasable::unerase` reads memory to reconstruct wide-pointer metadata
  it also reads the pointee — exactly what `SharedTerm::unerase`
  (`crates/aterm/src/storage/shared_term.rs:68-78`) does, via a `ptr::read` to
  recover `symbol.arity()`. `ptr()`'s own doc already says so ("reconstructs
  the wide pointer from the pointee, so it reads the header").
- **Impact**: does not by itself break the `Send`/`Sync` impls (moves are
  inert, concurrent reads-only access stays sound), but `ptr()` can read
  freed/dangling memory for such a `T` without being marked `unsafe`.
- **Status**: PLAUSIBLE, not fixed — flagged for a future pass; the comment
  whose premise this contradicts was left as-is rather than trimmed.

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
