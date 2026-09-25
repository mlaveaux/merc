# Phase 1b — mCRL2 FFI `atermpp` adversarial review

Scope: `tools/mcrl2/crates/mcrl2/src/atermpp/` (the Rust wrapper around the real
mCRL2 C++ `atermpp` term pool, crossing the FFI boundary via `mcrl2-sys`).

**Verdict: the change under review — this module as it stands — is not sound.**
There is one **CONFIRMED**, directly reachable memory-safety bug (a real
SIGSEGV, reproduced below): a 100%-safe public entry point
(`ATerm::with_args`) silently reads out of bounds and stores garbage
`*const _aterm` pointers when the caller passes an argument count that
doesn't match the symbol's arity, because both the Rust-side and the C++-side
guards against this are `debug_assert`/`assert`-only and compile out in
release builds. There is also one **PLAUSIBLE** but not sanitizer-confirmed
data race in the busy/forbidden locking scheme (`BfTermPool::write_exclusive`
vs `BfTermPool::read`), currently unreachable because nothing in the crate
calls the affected `Display`/`Debug` path, but real if that ever changes.
Everything else checked (protection-set bookkeeping, GC marking, `ATermSend`,
`Symbol` refcounting, the empty-list guards already present) held up.

## Environment notes (read first — they cap what could be CONFIRMED here)

- **`cargo build -p mcrl2` and `cargo test -p mcrl2` both work in this
  container** — the real C++ `mCRL2` library (vendored via the
  `mcrl2-sys` git dependency, checked out at
  `/root/.cargo/git/checkouts/mcrl2-sys-12d8cf56a310324e/6762cca`) builds and
  links fine. `cargo build -p mcrl2` finished in 5m17s, warnings only.
- **Miri cannot run any test in this crate**, confirmed empirically, not just
  predicted. `ThreadTermPool::new()` — reached by simply touching the
  thread-local `THREAD_TERM_POOL` the very first time, i.e. by *any* test in
  this module — calls real FFI (`mcrl2_aterm_list_function_symbol`,
  `mcrl2_aterm_pool_enable_automatic_garbage_collection`, ...), and miri has
  no execution model for calling already-compiled foreign (C++) code:
  ```
  MIRIFLAGS="-Zmiri-disable-isolation --cfg chacha20_force_soft" \
    cargo +nightly miri test -p mcrl2 test_aterm_int_value
  ...
  test atermpp::aterm_int::tests::test_aterm_int_value ... error: unsupported operation:
    can't call foreign function `atermpp$cxxbridge1$202$mcrl2_aterm_pool_enable_automatic_garbage_collection` on OS `linux`
  error: aborting due to 1 previous error
  ```
  This is not specific to that one test — every unsafe block in this module
  either calls into `mcrl2_sys::atermpp::ffi` directly, or is only reachable
  through `THREAD_TERM_POOL`, which calls FFI in its constructor. So **no
  miri evidence is possible for this crate at all**, only source-level
  safety-contract review plus real (non-interpreted) tests, sanitizers where
  they can be gotten to run, and reading the vendored C++ implementation.
- **ThreadSanitizer via `cargo +nightly xtask thread-sanitizer` does not work
  out of the box in this container**: it fails with `-Zsanitizer` ABI
  mismatches against build-script/proc-macro dependencies
  (`could not compile 'quote'/'proc-macro2'/'libc' (build script)`). Adding
  `-Cunsafe-allow-abi-mismatch=sanitizer` (rustc's own suggested workaround)
  gets past that, but the resulting `-Zbuild-std` rebuild of the whole
  dependency graph plus the C++ library under TSan is heavy, and in this
  specific container run it collided with **disk exhaustion** from other
  concurrent sessions' builds sharing the same `target/` directory
  (`ld terminated with signal 7 [Bus error]` — a linker crash caused by
  `ENOSPC`, not a sanitizer verdict). I freed 3.7G by deleting the completed,
  no-longer-needed `target/miri` directory (safe: it was mine, and I'd
  already extracted the result above), which fixed the disk pressure enough
  to run the CONFIRMED test below, but I did not re-attempt the TSan build a
  third time given the container's shared, resource-constrained state — so
  the race finding below is reasoned from source plus a deterministic
  (non-sanitized) repro test, not a TSan report.
- ASan was not attempted for the same reason (a second heavy sanitizer
  rebuild in an already disk-constrained, multi-tenant container).

## Findings

### 1. `ATerm::with_args` / `ThreadTermPool::create` — arity mismatch reads out of bounds and dereferences garbage in release builds. **CONFIRMED.**

`tools/mcrl2/crates/mcrl2/src/atermpp/thread_aterm_pool.rs:197-221` (`create`)
and `tools/mcrl2/crates/mcrl2/src/atermpp/aterm.rs:246-252` (`ATerm::with_args`,
the fully safe public entry point).

**Scenario.** `ATerm::with_args(&symbol, &arguments)` is called with
`arguments.len() != symbol.arity()` (e.g. a caller bug elsewhere passes one
argument too few for a 4-ary symbol). The only guard on the Rust side is:

```rust
debug_assert_eq!(
    symbol.borrow().arity(),
    tmp_args.len(),
    "Number of arguments does not match arity"
);
```

which is compiled out under `cfg(not(debug_assertions))`, i.e. in every
`--release` build (`merc-lps`/`merc-pbes` production binaries included). The
call then reaches `mcrl2_aterm_create(symbol, &tmp_args)` — a raw
`rust::Slice<*const _aterm>` over the (too-short) `Vec` — and I traced the
exact C++ dispatch this hits (vendored source, not guessed):

- `mcrl2-sys/cpp/atermpp.h::mcrl2_aterm_create` → `make_term_appl(result, symbol, aterm_slice.begin(), aterm_slice.end())`.
- `aterm.h::make_term_appl` (ForwardIterator overload) → `aterm_pool::create_appl_dynamic(target, sym, begin, end)`.
- `aterm_pool_implementation.h::aterm_pool::create_appl_dynamic` **dispatches
  by `sym.arity()`, not by `std::distance(begin, end)`**:
  ```cpp
  const std::size_t arity = sym.arity();
  switch (arity) {
    case 1: return std::get<1>(m_appl_storage).create_appl_iterator<ForwardIterator>(term, sym, begin, end);
    ...
    case 7: return std::get<7>(m_appl_storage).create_appl_iterator<ForwardIterator>(term, sym, begin, end);
    default: return m_appl_dynamic_storage.create_appl_dynamic(term, sym, begin, end);
  }
  ```
- which reaches `_aterm_appl<N>`'s iterator constructor
  (`atermpp/detail/aterm.h`), the one actually used here:
  ```cpp
  template<typename Iterator>
  _aterm_appl(const function_symbol& symbol, Iterator it, [[maybe_unused]] Iterator end, bool)
    requires (mcrl2::utilities::is_iterator<Iterator>::value)
      : _aterm(symbol)
  {
    for (std::size_t i = 0; i < symbol.arity(); ++i)
    {
      // Prevent bound checking, the allocator must make sure that symbol.arity() arguments fit.
      assert(it != end);
      m_arguments.data()[i] = *it;
      ++it;
    }
    assert(it == end);
  }
  ```
  This loop runs exactly `symbol.arity()` times **regardless of where the
  caller's real `end` is**, guarded only by `assert(it != end)` — itself
  compiled out under `-DNDEBUG=1`, which is exactly the flag this
  workspace's own release C++ build uses (visible in the `cc1plus`
  invocation captured live during this review: `... -D NDEBUG=1 ...
  -std=c++20 ... -O3 ...` for `target/release`). So in a release build there
  is **no bounds check on either side of the FFI boundary**: the loop reads
  `symbol.arity() - arguments.len()` elements past the end of the Rust
  `Vec<*const ffi::_aterm>` and stores whatever bytes it finds there as if
  they were live `*const _aterm` pointers — which are then dereferenced by
  anything that later walks the term (`arg()`, `Debug`, hashing, GC marking).

**Evidence.** Added
`atermpp::aterm::tests::with_args_arity_mismatch_reads_out_of_bounds_in_release`
in `aterm.rs` (constant `BOGUS_ARITY = 40`, one real argument). It is
`#[ignore]`d under `debug_assertions` (the Rust-side `debug_assert_eq!` would
panic first, which is a different, unrelated failure mode) and runs for real
in `--release`:

```
$ cargo test --release -p mcrl2 --lib with_args_arity_mismatch_reads_out_of_bounds_in_release -- --test-threads=1 --nocapture

running 1 test
test atermpp::aterm::tests::with_args_arity_mismatch_reads_out_of_bounds_in_release ... arg[0] = with_args_arity_mismatch_reads_out_of_bounds_in_release_a
arg[1] = None
arg[2] = None
arg[3] = None
arg[4] = None
error: test failed, to rerun pass `-p mcrl2 --lib`

Caused by:
  process didn't exit successfully: `.../target/release/deps/mcrl2-8ea3b674a3ed8f95 with_args_arity_mismatch_reads_out_of_bounds_in_release --test-threads=1 --nocapture` (signal: 11, SIGSEGV: invalid memory reference)
```

`arg[0]` is the one real argument, printed correctly. `arg[1..4]` print as
`None` — not because they're legitimately absent, but because
`ATermRef::is_default()` (a null-pointer check) happened to read zero bytes
at those out-of-bounds Vec slots on this particular run; this is
non-deterministic garbage, not a real "no argument" state. The process then
**segfaults** dereferencing `arg[5]`, i.e. a genuine, reproducible crash, not
a hypothetical one.

**Why the test would pass once fixed:** with a real bounds/arity check on
either side (the natural fix is making the Rust-side check unconditional,
e.g. `assert_eq!` instead of `debug_assert_eq!`, or returning a `Result`),
this call would panic cleanly with a descriptive message instead of reading
past the `Vec` and segfaulting — matching the pattern the codebase already
applies to `ATermList::head()`/`tail()` (see "Checked and found correct"
below), which fixed exactly this class of bug for the empty-list case but
not for `create`/`with_args` itself.

**Severity / reachability:** `ATerm::with_args` is a widely used, fully safe
public API (used throughout `merc_lps`/`merc_pbes`), so this is reachable by
an ordinary caller bug (e.g. constructing a symbol with the wrong arity
somewhere upstream) with zero `unsafe` at the call site — and only in release
builds, which is exactly when it would ship. This is the single largest
finding of this phase.

**Direction of a fix:** upgrade the Rust-side `debug_assert_eq!` in
`ThreadTermPool::create`/`create_data_application` to a real,
always-checked `assert_eq!` (or a `Result`-returning API), matching what was
already done for `ATermList::head()`/`tail()`.

### 2. `BfTermPool::write_exclusive` vs `BfTermPool::read` — the busy/forbidden contract is violated by `GlobalTermPool`'s own `Debug` impl. **PLAUSIBLE**, not sanitizer-confirmed (see environment notes).

`tools/mcrl2/crates/mcrl2/src/atermpp/busy_forbidden.rs` (`write_exclusive`,
`read`) and `tools/mcrl2/crates/mcrl2/src/atermpp/global_aterm_pool.rs:188-230`
(`impl Debug for GlobalTermPool`), reached via
`tools/mcrl2/crates/mcrl2/src/atermpp/thread_aterm_pool.rs:423-430`
(`impl Display for ThreadTermPool`).

**Scenario.** The mCRL2 busy/forbidden protocol has two lock classes:
*shared* (`mcrl2_aterm_pool_lock_shared`/`unlock_shared`, a per-thread "busy"
flag; any number of threads may hold it concurrently) and *exclusive*
(`mcrl2_aterm_pool_lock_exclusive`, which waits for every thread's busy flag
to clear — used for GC and hash-table resize). `BfTermPool::read` and
`BfTermPool::write_exclusive` **both take the shared lock**, `read` handing
out `&T`, `write_exclusive` handing out `&mut T`. They are therefore *not*
mutually exclusive with each other — soundness instead depends on the
undocumented (until this review; see the doc-comment fixes below) invariant
that no thread other than the one currently inside a `write_exclusive` guard
ever calls `.read()`/`.get()` on that same `BfTermPool<T>` value.

Every genuine use of `write_exclusive` in `ThreadTermPool` upholds this
(each thread only ever mutates its own `SharedProtectionSet`/
`SharedContainerProtectionSet`), **except** `GlobalTermPool`'s `Debug` impl,
which calls `set.read()` on *every* thread's protection set —
`global_aterm_pool.rs:194-214` — from whatever thread happens to format it,
with no coordination against the owning thread's concurrent
`write_exclusive` (e.g. inside `protect_with` while creating a term). This
`Debug` impl is reached by `ThreadTermPool`'s `Display` impl
(`thread_aterm_pool.rs:423-430`, `write!(f, "{:?}", GLOBAL_TERM_POOL.lock())`),
which is `pub`. Concretely: thread A creates terms in a loop (mutating its
own protection set via `write_exclusive`, i.e. `&mut ProtectionSet<ATermPtr>`
live); thread B concurrently formats `THREAD_TERM_POOL` (`{}`  on a
`ThreadTermPool`), which calls `.read()` (`&ProtectionSet<ATermPtr>`) on
thread A's set — aliased mutable/shared access to the same
`ProtectionSet<ATermPtr>`, and a genuine data race if the underlying `Vec`
reallocates mid-iteration.

**Currently unreachable:** grepping the whole `mcrl2` crate (not just
`atermpp/`) shows `Display for ThreadTermPool` has zero callers anywhere in
this codebase today — it's dead code from the caller's side, so this cannot
fire in the current binaries. It is nonetheless a live, `pub`, `pub(crate)`-
reachable API whose safety argument is simply wrong, and it is exactly the
kind of debug/logging helper someone reaches for first when diagnosing a
pool-size issue — at which point it becomes live.

**Evidence gathered (deterministic repro, not sanitizer-confirmed):** added
`atermpp::thread_aterm_pool::tests::read_races_with_concurrent_term_creation`
in `thread_aterm_pool.rs` — 4 threads continuously creating terms while a 5th
repeatedly formats `THREAD_TERM_POOL`. It passed in a plain debug run
(`cargo test -p mcrl2 --lib read_races_with_concurrent_term_creation`, 0.08s)
without crashing, which is expected and proves nothing either way — data
races frequently don't manifest visibly without a sanitizer, especially in a
sub-second run. I could not get a ThreadSanitizer report in this container
(see environment notes: ABI-mismatch build errors, then a disk-exhaustion-
induced linker crash on retry after working around the ABI issue). Downgrading
to **PLAUSIBLE**: a concrete scenario and a deterministic repro test exist,
but no sanitizer report backs it.

**Direction of a fix:** either make `write_exclusive` take the real C++
*exclusive* lock (defeating its own purpose — the comment in
`thread_aterm_pool.rs` explains resizing/GC must stay off the per-term-
creation hot path), or have `GlobalTermPool::Debug`/`Display for
ThreadTermPool` only read a thread's protection set from that thread itself
(post a request and read the result), or gate it behind the same exclusive
GC-pause mechanism `mark_protection_sets` already uses.

## Checked and found correct

- `ATermList::head()`/`tail()` already have exactly the right fix for the
  same debug-vs-release FFI-bounds-check gap that finding 1 describes,
  applied locally (`assert!(!self.is_empty(), ...)` before calling `arg(0)`/
  `arg(1)`), with tests (`head_on_empty_list_panics`,
  `tail_on_empty_list_panics`) that I re-ran and confirmed do fail without
  the guard reasoning holding (they exercise the `assert!`, not a
  `debug_assert!`, so they hold in release too). Finding 1 is the same class
  of bug at a different, more fundamental call site that didn't get the same
  fix.
- `ProtectionSet`/`ProtectionIndex` (`crates/unsafety/src/protection_set.rs`,
  read for context, out of this phase's file scope but load-bearing here):
  generational indices correctly reject a stale `ProtectionIndex` whose slot
  was freed and reused (`contains_root`), and `unprotect`'s own
  `# Safety`/kani contract matches how `ThreadTermPool::drop_term`/
  `drop_container` call it (each `ATerm`/`Protected` root is unprotected
  exactly once, on `Drop`).
- `ATermSend` and the global `SEND_PROTECTION_SET`: unlike the thread-local
  scheme, this is a real `parking_lot::Mutex<ProtectionSet<ATermPtr>>`
  guarding both mutation (`protect`/`clone`/`drop`, from any thread) and
  reading (`mark_protection_sets`'s walk of it during GC) — genuinely
  race-free, confirmed by reading every call site. `test_aterm_send_cross_thread`
  (already present) exercises creation on one thread, survival across a
  forced GC and use on a second thread, and cross-thread drop; it passes.
- `Symbol`/`SymbolRef` refcounting: `Symbol::take` (no protect, for
  `mcrl2_function_symbol_create`'s output, which already bumped the
  refcount) vs `Symbol::from_ptr` (protects, for borrowed FFI pointers like
  `mcrl2_aterm_list_function_symbol()`'s result) are used consistently
  everywhere they're called in this module; `Drop for Symbol` decrements
  exactly once per `Symbol`, balanced correctly against both construction
  paths.
- `mark_protection_sets`/`register_mark_callback`: the raw `.get()` calls
  that bypass the busy/forbidden lock during marking are justified — per the
  vendored FFI docs (`mcrl2-sys/src/atermpp.rs`), garbage collection and hash
  table resizing require every thread to be outside a shared section, so no
  thread can be concurrently inside `write_exclusive` while marking runs;
  unlike finding 2, marking itself does not violate its own contract.
- `ATermRef`'s lifetime-elision-based design (`arg()`, `upgrade()`,
  `upgrade_unchecked()`, `copy()`): traced through how a subterm's `ATermRef`
  lifetime is always tied to (never wider than) a borrow of its parent, and
  how `upgrade`/`upgrade_unchecked` extend it back up to an already-live
  ancestor rather than fabricating a wider one — internally consistent with
  the invariant documented at the top of `aterm.rs`, and every internal call
  site (`ATermArgs`, `TermIterator`, `ATermListIterRef`) upgrades against an
  actual direct parent/subterm relationship.
- `Protected<T>`: `!Send` (via `PhantomUnsend`) correctly forces creation and
  drop onto the same thread, matching its own doc comment; `handle()`
  deliberately hands out a plain `Arc<T>` with no protection root attached,
  which is fine since the container itself stays reachable via the
  `Protected`'s own root as long as any `Arc` clone is alive to be inserted
  into.
- `Symbol::from_ptr`'s callers (`list_symbol`/`empty_list_symbol` in
  `ThreadTermPool::new`) do pass pointers returned live from the FFI at the
  point of the call, satisfying the precondition I tightened below.

## Doc-only changes: `# Safety` contracts tightened

No production logic was changed. I rewrote the `# Safety` sections on every
`unsafe fn`/`unsafe impl` in scope into precise pre/postcondition form
(previously several were prose-only or, in `write_exclusive`'s case, stated
the contract backwards from what actually makes it sound — see finding 2):

- `busy_forbidden.rs`: `BfTermPool` struct-level contract (spells out the
  shared-vs-exclusive lock classes and exactly which concurrent call
  combinations are/aren't sound — this is where finding 2's precondition is
  now stated precisely), its `Send`/`Sync` impls, `get()`, `write_exclusive()`.
- `aterm.rs`: `ATermRef`'s `Send`/`Sync` impls, `ATermRef::new`,
  `ATermRef::upgrade`, `ATermRef::upgrade_unchecked`, `ATerm::from_ptr`,
  `ATermSend::from_ptr`.
- `symbol.rs`: `Symbol::from_ptr`.
- `aterm_string.rs`: `ATermStringRef::from_address` (corrected from an
  initial draft that over-claimed "permanently interned" — verified against
  actual callers in `merc_pbes`/`merc_lps` instead, which only require
  reachability for as long as the reference is read, not forever).
- `global_aterm_pool.rs`: `ATermPtr`'s `Send`/`Sync` impls.

## Tests added

All in `tools/mcrl2/crates/mcrl2/src/atermpp/`, run from `tools/mcrl2/`:

- `aterm.rs`,
  `atermpp::aterm::tests::with_args_arity_mismatch_reads_out_of_bounds_in_release`
  — **CONFIRMED**, finding 1. Debug builds: `#[ignore]`d (the
  `debug_assert_eq!` panics first, a different code path). Release:
  ```
  cargo test --release -p mcrl2 --lib with_args_arity_mismatch_reads_out_of_bounds_in_release -- --test-threads=1 --nocapture
  ```
  reproduces the SIGSEGV quoted above. Run in its own process
  (`--test-threads=1`, and ideally alone) since it crashes the test binary.
- `thread_aterm_pool.rs`,
  `atermpp::thread_aterm_pool::tests::read_races_with_concurrent_term_creation`
  — repro for finding 2 (currently passes without a sanitizer; see finding 2
  for why that's expected and not exculpatory). Run:
  ```
  cargo test -p mcrl2 --lib read_races_with_concurrent_term_creation
  ```
  For real evidence, run under ThreadSanitizer once the container has a
  working sanitizer toolchain / enough disk:
  ```
  cargo +nightly xtask thread-sanitizer test --no-fail-fast -p mcrl2 -- --lib read_races_with_concurrent_term_creation
  ```

Verifiers run vs. skipped, per the `unsafe-verify` skill:
- **miri**: attempted, fails to even start (foreign-function call), see
  environment notes — not usable for this crate at all.
- **loom**: not applicable — this module's concurrency crosses a real C++
  lock, not a loom-modelable Rust primitive.
- **kani**: explicitly out of scope per this phase's instructions (CBMC
  cannot model-check across a real FFI call into C++).
- **ASan/TSan (xtask)**: attempted TSan twice; first failed on
  `-Zsanitizer` ABI mismatches against build-script dependencies, second
  (after adding `-Cunsafe-allow-abi-mismatch=sanitizer`) got much further but
  hit a disk-exhaustion-induced linker crash in this shared, multi-tenant
  container. Not re-attempted a third time. ASan not attempted at all (same
  resource constraints). Neither finding above rests on a sanitizer report;
  finding 1 rests on an actual reproduced SIGSEGV plus full source-level
  tracing through the vendored C++, finding 2 on source-level tracing plus a
  deterministic (currently non-crashing) repro test.
