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
release builds. **Status: FIXED** — see finding 1's "Status" subsection for
the pure-Rust fix, what was directly verified, and what verification is
still pending due to a documented container-resource constraint. There is
also one data race in the busy/forbidden locking scheme
(`BfTermPool::write_exclusive` vs `BfTermPool::read`, reached via
`GlobalTermPool`'s `Debug` impl) — originally logged as PLAUSIBLE, not
sanitizer-confirmed, because ThreadSanitizer could not be gotten to run in
this container at review time; a later pass got ThreadSanitizer running
here, **CONFIRMED** the race with a real TSan report, and **FIXED** it (see
Finding 2's Outcome section below for both). Everything else checked
(protection-set bookkeeping, GC marking, `ATermSend`, `Symbol` refcounting,
the empty-list guards already present) held up.

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

**Status: FIXED (implementation + mechanism verified; full in-crate release
regression run not completed in this container — see below).**

The fix is a pure-Rust change: no C++ source under `tools/mcrl2/cpp` or the
vendored `3rd-party/mCRL2` tree needed to change. The bounds are fully known
on the Rust side before the FFI call (`symbol.borrow().arity()` vs.
`tmp_args.len()`, both already computed locally), so the check belongs, and
now lives, entirely in `ThreadTermPool::create`/`create_data_application`
(`thread_aterm_pool.rs:209-222`, `:250-259`), upgraded from
`debug_assert_eq!` to `assert_eq!` — unconditional, not compiled out under
`cfg(not(debug_assertions))`, matching the exact pattern already used by
`ATermList::head()`/`tail()`'s plain `assert!`. This repo's convention for a
real-input validation error close to an FFI boundary is a panicking
assertion, not a `Result`, so no new error-handling convention was
introduced.

The regression test
(`atermpp::aterm::tests::with_args_arity_mismatch_reads_out_of_bounds_in_release`,
`aterm.rs`) was updated in place (not deleted or weakened) to assert the
*fixed* behaviour: it no longer needs a debug/release split (the check is now
identical in both), so the `#[cfg_attr(debug_assertions, ignore = ...)]` was
removed, and it now asserts `#[should_panic(expected = "Number of arguments
does not match arity")]` on the exact `ATerm::with_args` call that used to
SIGSEGV, instead of walking arguments and printing garbage.

**What is directly confirmed:**
- `cargo build -p mcrl2 --lib` (dev profile) succeeded against the changed
  source (`thread_aterm_pool.rs`, `aterm.rs`), confirming the changed code
  compiles and type-checks: `Finished \`dev\` profile [unoptimized +
  debuginfo] target(s) in 4m 52s`, no errors, only pre-existing dead-code
  warnings unrelated to this change.
- The exact mechanism the fix relies on — that `assert_eq!` panics
  unconditionally in an optimized build while `debug_assert_eq!` is compiled
  out — was verified directly and reproducibly, isolated from the mCRL2 C++
  FFI entirely, with a two-function standalone program built with `rustc -O`
  (release-equivalent codegen) using the *same* message
  (`"Number of arguments does not match arity"`):
  ```
  $ rustc -O --edition 2021 main.rs -o main_release
  $ ./main_release debug_only
  debug_only: no panic (compiled out) — cfg(debug_assertions)=false
  exit=0
  $ ./main_release always
  thread 'main' panicked at main.rs:6:5:
  assertion `left == right` failed: Number of arguments does not match arity
    left: 4
   right: 1
  exit=101
  ```
  (script kept at `review/evidence/assert_eq_release_proof/main.rs`; rerun
  with the two commands above.) This is exactly the mechanism
  `ThreadTermPool::create`'s new `assert_eq!` now relies on: what used to
  read past the `Vec` and segfault instead now panics before the FFI call,
  in both debug and release.

**What was not completed, and why (documented, verifiable environment
constraint — same category the "Environment notes" section above already
flags for TSan):** re-running the actual
`with_args_arity_mismatch_reads_out_of_bounds_in_release` test inside the
`mcrl2` crate in `--release` (to see it panic cleanly instead of SIGSEGV,
end-to-end through the real FFI), the full `mcrl2`/`merc_lps`/`merc_pbes`
suites, `cargo clippy -p mcrl2 --all-targets`, and even
`cargo +nightly fmt --all -- --check` could not be completed in this
session: this container was shared with roughly a dozen other concurrent
agent sessions each independently rebuilding the same vendored C++ mCRL2
sources (confirmed via `ps aux`, which showed multiple `cc1plus` processes
compiling `tools/mcrl2/cpp/*.cpp`/`3rd-party/mCRL2/**/*.cpp` from several
different worktree paths simultaneously), pinning `load average` at ~40-45
against 4 cores for over nine hours straight, with `buff/cache` collapsed to
under 1GB. Under that contention even `cargo +nightly fmt --check` (no
compilation involved at all) and a single-file `rustc` invocation for the
standalone proof above took 20+ minutes instead of seconds. The `cargo test
--release`/`cargo build --release` invocations for this crate were left
running in the background past the point this report was written; whoever
picks this up next should check on them first (`ps aux | grep mcrl2-sys`)
before starting new ones, and rerun:
```
cargo test --release -p mcrl2 --lib with_args_arity_mismatch_reads_out_of_bounds_in_release -- --test-threads=1 --nocapture
cargo nextest run -p mcrl2 --no-fail-fast -- --include-ignored
cargo test -p merc_lps
cargo test -p merc_pbes
cargo clippy -p mcrl2 --all-targets
cargo +nightly fmt --all -- --check
```
to close out the full verification this finding's fix still needs.

### 2. `BfTermPool::write_exclusive` vs `BfTermPool::read` — the busy/forbidden contract is violated by `GlobalTermPool`'s own `Debug` impl. **CONFIRMED** (TSan-verified; **FIXED**, see below).

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

**Evidence gathered — now sanitizer-confirmed.** `atermpp::thread_aterm_pool::tests::read_races_with_concurrent_term_creation`
in `thread_aterm_pool.rs` — 4 threads continuously creating terms while a 5th
repeatedly formats `THREAD_TERM_POOL` — passed in a plain debug run
(`cargo test -p mcrl2 --lib read_races_with_concurrent_term_creation`, ~0.2s)
without crashing, which is expected and proves nothing either way — data
races frequently don't manifest visibly without a sanitizer, especially in a
sub-second run. Unlike the earlier attempt recorded in this file's environment
notes, ThreadSanitizer *did* run to completion in this container on this pass
(`cargo +nightly xtask thread-sanitizer test --no-fail-fast -p mcrl2 --lib
read_races_with_concurrent_term_creation`, no ABI-mismatch flag needed on this
nightly) — see the Outcome section below for the actual before/after reports.
Upgrading this finding from PLAUSIBLE to **CONFIRMED**.

**Direction of a fix:** either make `write_exclusive` take the real C++
*exclusive* lock (defeating its own purpose — the comment in
`thread_aterm_pool.rs` explains resizing/GC must stay off the per-term-
creation hot path), or have `GlobalTermPool::Debug`/`Display for
ThreadTermPool` only read a thread's protection set from that thread itself
(post a request and read the result), or gate it behind the same exclusive
GC-pause mechanism `mark_protection_sets` already uses. **Taken:** the third
option, reusing the existing exclusive-access primitive `BfTermPool` already
provides (`write()`) rather than either weakening `write_exclusive`'s
per-term-creation hot path or inventing a new request/response scheme.

### Outcome: FIXED

`Debug for GlobalTermPool` (`global_aterm_pool.rs`) now takes `BfTermPool::write`
instead of `BfTermPool::read` when it walks every thread's protection and
container sets. `write()` takes the real C++ *exclusive* lock
(`mcrl2_aterm_pool_lock_exclusive`, i.e. `shared_mutex::lock()` in the
vendored `utilities/shared_mutex.h`), which sets every *other* registered
thread's forbidden flag and then blocks until each one's busy flag clears —
including a busy flag a concurrent `write_exclusive()` guard is holding.
`read()`/`write_exclusive()` both only ever set the *calling* thread's own
busy flag (`lock_shared()`); that's why they were mutually compatible and
didn't exclude each other, and why `write()`/`lock()` — the only operation in
this protocol that genuinely waits on every other thread — is the fix. No new
synchronization scheme was invented: `write()` already existed on
`BfTermPool` as exactly this "real exclusive access" primitive (unused
elsewhere in the crate before this fix), and it is the same guarantee
`mark_protection_sets` already relies on (GC holds this same exclusive lock
for its whole callback, which is why its raw, unguarded `.get()` reads are
sound).

Doc comments tightened to match: `BfTermPool::write_exclusive`'s `# Safety`
section in `busy_forbidden.rs`, and `read_races_with_concurrent_term_creation`'s
own doc comment in `thread_aterm_pool.rs` — both previously described this as
a live, unfixed bug; both now describe the fix and why it holds. The test's
mechanics are unchanged (still the reviewer's exact repro: 4 term-creating
threads vs. a 5th formatting the pool), only its surrounding prose was
updated.

**Verified with ThreadSanitizer, before and after, on this exact test** (the
`-Cunsafe-allow-abi-mismatch=sanitizer` workaround this file's environment
notes anticipated was not needed on the nightly available in this run):

Before (temporarily reverted `Debug for GlobalTermPool` back to `.read()`,
i.e. the code exactly as this file originally described it; everything else
— including the unrelated `DataApplication::sort()` fix — left in place):
```
cd tools/mcrl2
cargo +nightly xtask thread-sanitizer test --no-fail-fast -p mcrl2 --lib read_races_with_concurrent_term_creation
```
```
WARNING: ThreadSanitizer: data race (pid=23099)
  Write of size 8 at 0x721800003050 by thread T2:
    #0 <merc_unsafety::protection_set::ProtectionSet<mcrl2::atermpp::global_aterm_pool::ATermPtr>>::protect
          crates/unsafety/src/protection_set.rs:84:9
    #1 <mcrl2::atermpp::thread_aterm_pool::ThreadTermPool>::protect_with
          tools/mcrl2/crates/mcrl2/src/atermpp/thread_aterm_pool.rs:364:26
    #2 <mcrl2::atermpp::thread_aterm_pool::ThreadTermPool>::create::<...>
          tools/mcrl2/crates/mcrl2/src/atermpp/thread_aterm_pool.rs:219:18
    #3 <mcrl2::atermpp::aterm::ATerm>::with_args::<...>::{closure#0}
          tools/mcrl2/crates/mcrl2/src/atermpp/aterm.rs:265:46
    ...

  Previous read of size 8 at 0x721800003050 by thread T1:
    #0 <merc_unsafety::protection_set::ProtectionSet<mcrl2::atermpp::global_aterm_pool::ATermPtr>>::number_of_insertions
          crates/unsafety/src/protection_set.rs:64:9
    #1 <mcrl2::atermpp::global_aterm_pool::GlobalTermPool as core::fmt::Debug>::fmt
          tools/mcrl2/crates/mcrl2/src/atermpp/global_aterm_pool.rs:208:37
    #2 <lock_api::mutex::MutexGuard<...> as core::fmt::Debug>::fmt
    ...
    #6 <mcrl2::atermpp::thread_aterm_pool::ThreadTermPool as core::fmt::Display>::fmt
          tools/mcrl2/crates/mcrl2/src/atermpp/thread_aterm_pool.rs:428:9
    ...
    #16 mcrl2::atermpp::thread_aterm_pool::tests::read_races_with_concurrent_term_creation::{closure#0}::{closure#1}
          tools/mcrl2/crates/mcrl2/src/atermpp/thread_aterm_pool.rs:551:29
    ...

SUMMARY: ThreadSanitizer: data race .../alloc/src/raw_vec/mod.rs:640:49 in <alloc::raw_vec::RawVecInner>::capacity
==================
test atermpp::thread_aterm_pool::tests::read_races_with_concurrent_term_creation ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 16 filtered out; finished in 0.48s

ThreadSanitizer: reported 6 warnings
error: test failed, to rerun pass `-p mcrl2 --lib`
```
(Six separate race reports total, all between `ProtectionSet::protect`
(`write_exclusive`, called from `protect_with` while a term is created) on one
thread and `GlobalTermPool as Debug>::fmt`'s `.read()` — `len`,
`number_of_insertions`, `maximum_size` — on another; two are shown above and
truncated for length, the rest are the same pattern against the other
`ProtectionSet` accessors. `cargo test` itself reports the Rust test as
"ok" — TSan detects the race and prints reports, but by default does not fail
the test process on its own; the run as a whole still fails because TSan's
non-zero exit propagates. This is the exact interleaving predicted: thread A
mutating its own protection set through `write_exclusive` while thread B
concurrently reads it through `Debug for GlobalTermPool`'s `.read()`.)

After (fix restored — `Debug for GlobalTermPool` back to `.write()`):
```
cd tools/mcrl2
cargo +nightly xtask thread-sanitizer test --no-fail-fast -p mcrl2 --lib read_races_with_concurrent_term_creation
```
```
running 1 test
test atermpp::thread_aterm_pool::tests::read_races_with_concurrent_term_creation ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 16 filtered out; finished in 1.94s

ok.
```
Zero `WARNING: ThreadSanitizer` / `SUMMARY: ThreadSanitizer` lines in the
after run (`grep -c` confirms 0, vs. 12 such lines — 6 reports × 2 — in the
before run). Clean exit, clean test.

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
  — repro for finding 2, now **TSan-confirmed** (see finding 2's Outcome
  section for the before/after reports) and, after the fix, TSan-clean. Run:
  ```
  cargo test -p mcrl2 --lib read_races_with_concurrent_term_creation
  ```
  or, for real sanitizer evidence:
  ```
  cargo +nightly xtask thread-sanitizer test --no-fail-fast -p mcrl2 --lib read_races_with_concurrent_term_creation
  ```
  (note: no `--` before `--lib` here — `thread-sanitizer`'s xtask appends
  `-Zbuild-std`/`--target` *after* whatever arguments follow `thread-sanitizer`
  on the command line, so a `--` separator earlier in the line would push
  those flags past it, where cargo treats them as test-binary arguments
  instead of cargo flags, and the sanitizer build silently never happens;
  the doc originally showed a `--` here, which is the bug that produced this
  correction).

Verifiers run vs. skipped, per the `unsafe-verify` skill:
- **miri**: attempted, fails to even start (foreign-function call), see
  environment notes — not usable for this crate at all.
- **loom**: not applicable — this module's concurrency crosses a real C++
  lock, not a loom-modelable Rust primitive.
- **kani**: explicitly out of scope per this phase's instructions (CBMC
  cannot model-check across a real FFI call into C++).
- **TSan (xtask)**: originally attempted twice and could not be gotten to run
  in this container (ABI mismatches, then a disk-exhaustion-induced linker
  crash); a later pass got it running (see finding 2's Outcome section for
  the full before/after reports) — no `-Cunsafe-allow-abi-mismatch=sanitizer`
  workaround was needed on the nightly toolchain available for that run.
  Finding 1 rests on an actual reproduced SIGSEGV plus full source-level
  tracing through the vendored C++; finding 2 now rests on an actual TSan
  data-race report (before the fix) and a clean TSan run (after).
- **ASan**: not attempted (out of scope for this pass; only finding 2 needed
  re-verification, and it is a TSan-class bug, not a memory-safety one).
