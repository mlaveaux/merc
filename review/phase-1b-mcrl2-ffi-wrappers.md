# Phase 1b review: mcrl2 FFI higher-level wrappers and tool binaries

Scope: `tools/mcrl2/crates/mcrl2/src/{data_expression,pbes,pbes_expression,lps,visitor}.rs`,
`tools/mcrl2/lps/src/main.rs`, `tools/mcrl2/pbes/src/main.rs`,
`tools/mcrl2/crates/merc_pbes/src/{explore_srf,explore_pbes,quotient_lps}.rs`,
`tools/mcrl2/crates/merc_lps/src/explore_explicit.rs`. Kani does not apply (real C++ FFI, not
model-checkable); miri does not apply either (every test touches `THREAD_TERM_POOL`, whose
constructor calls real FFI immediately). Evidence is real `cargo test`/`cargo test --release`
runs against the actual mCRL2 C++ backend, which does build and run in this container.

**Verdict**: not sound. Three `unsafe impl Send` blocks let 100% safe code crash the process
with a C++-side assertion failure; one public method (`DataApplication::sort()`) silently
returns the wrong term type. Both confirmed independently (re-ran the reviewing agent's repro
commands against the unmodified tree before any fix landed). Everything else the agent examined
(GC-reachability contracts on `from_address` constructors, `PbesFlattenStack`'s `Send`,
`{PbesSrfLps,PbesLps,ExplicitLinearProcessSpecification}`'s `Sync`, `QuotientLps`/
`QuotientSummand`'s `Sync`, the raw-pointer scratch buffers, the `catch_unwind`/`abort()`
trampoline for C→Rust callbacks) held up on inspection and is not restated in full detail here —
see the "Checked and found correct" summary below.

## Finding 1 — three `unsafe impl Send` blocks contradict their own field's documented
thread-affinity; cross-thread use crashes the process — CONFIRMED, CRITICAL

- `tools/mcrl2/crates/merc_pbes/src/explore_srf.rs:175` `unsafe impl Send for PbesSrfContext {}`
- `tools/mcrl2/crates/merc_pbes/src/explore_pbes.rs:267` `unsafe impl Send for PbesContext {}`
- `tools/mcrl2/crates/merc_lps/src/explore_explicit.rs:608` `unsafe impl Send for ExplicitContext {}`

Each struct embeds a handle whose own type is correctly documented (and marked) `!Send`:
`PbesSrfContext.context: LearnSuccessorsContext` ("must be created and destroyed on the same
thread", `crates/mcrl2/src/lps.rs`), `PbesContext.rewrite: PbesRewriteContext` ("Not `Send`: the
underlying C++ rewriter is single-threaded", `crates/mcrl2/src/pbes.rs`), and
`ExplicitContext.context: LearnSuccessorsContext` (same as the first). The three `unsafe impl
Send` blocks override that, so safe code can create a context on one OS thread and use it on
another — exactly what the inner type's own documentation forbids.

Reproduced independently (before any fix, against the unmodified tree): a test per explorer
family creates a context on the main thread via the public `LPS::create_context()`, moves it
into a `std::thread::scope` worker (which only compiles because of the `unsafe impl Send`), and
calls `LPS::prepare`/`Summand::enumerate` on the worker thread. All three abort the process
identically:

```
3rd-party/mCRL2/libraries/utilities/include/mcrl2/utilities/detail/hashtable.h:145:
mcrl2::utilities::hashtable<...>::erase: Assertion `false && "hashtable::erase
called for a key that is not present"' failed.
signal: 6, SIGABRT
```

```
cd tools/mcrl2
cargo test -p merc_pbes --test pbes_srf_context_send_soundness_test   # SIGABRT, confirmed
cargo test -p merc_pbes --test pbes_context_send_soundness_test       # SIGABRT, confirmed
cargo test -p merc_lps  --test explicit_context_send_soundness_test   # SIGABRT, confirmed
```

The panicking assertion is inside atermpp's per-thread pointer hashtable — the C++ enumerator
registers/erases term pointers against the hashtable of the thread that constructed it, and using
it from a different thread erases a key from a hashtable it was never inserted into from that
thread's point of view.

`QuotientContext<P>` (`crates/merc_pbes/src/quotient_lps.rs`) has no `unsafe impl` of its own but
auto-derives `Send` from `<P::Summand as Summand>::Context`, so it inherited the same bug
transitively whenever `P` was `PbesSrfLps`/`PbesLps`; no separate finding, same root cause.

Why the live code never crashed in production before this review: `merc_explore::explore_parallel`
(`crates/explore/src/explore.rs`) calls `lps.create_context()` *inside* the `rayon::broadcast`
closure, so every call site created and dropped the context on the same physical worker thread —
the type's `Send` bound was never actually exercised by real callers. But `Send` is a type-level,
unconditional claim advertised as safe for *any* caller, not just the current ones.

### Outcome: FIXED

Removed all three `unsafe impl Send` blocks. Checked whether anything actually required the bound:
`crates/explore/src/explore.rs`'s `explore_parallel` had a `<P::Summand as Summand>::Context:
Send` bound in its `where` clause, but its body creates `context` *inside* the `rayon::broadcast`
closure and never moves it out — the bound was structurally unnecessary. Removed it there and at
every downstream pass-through call site that only propagated it without needing it independently
(`crates/explore/tests/explore_test.rs`, `tools/mcrl2/crates/merc_lps/src/explore_explicit.rs`'s
`run_explore_explicit_parallel`, `tools/mcrl2/crates/merc_pbes/src/explore_common.rs`'s
`explore_pbes_parallel_impl`, `tools/mcrl2/pbes/src/main.rs`'s `quotient_explore` — traced each
one's call chain to confirm it only reaches `explore_parallel`, never anything else that would
need `Send`).

Verified: `cargo check -p merc_explore --all-targets` (root workspace) and
`cargo check -p mcrl2 -p merc_pbes -p merc_lps --all-targets --bin merc-pbes` (`tools/mcrl2`
workspace) both clean.

The three original runtime crash-repro tests could no longer *compile* once the impls were
removed (the whole point — the bug is now rejected at compile time, not merely detected at
runtime). Converted each into a `trybuild` compile-fail regression test (the repo already uses
this pattern in `crates/aterm/tests/build_tests.rs`): `tools/mcrl2/crates/merc_pbes/tests/
build_tests.rs` (`pbes_srf_context_not_send.rs`, `pbes_context_not_send.rs`) and
`tools/mcrl2/crates/merc_lps/tests/build_tests.rs` (`explicit_context_not_send.rs`). Each fixture
creates a context and tries to move it into `std::thread::spawn`; `trybuild` asserts this fails to
compile. Added `trybuild` to `tools/mcrl2`'s workspace dependencies (it wasn't there yet) and as a
dev-dependency of both crates.

## Finding 2 — `DataApplication::sort()` / `DataApplicationRef::sort()` returns the function
symbol, not the application's result sort — CONFIRMED

`tools/mcrl2/crates/mcrl2/src/data_expression.rs:296-301`:

```rust
/// Returns the sort of a data application.
pub fn sort(&self) -> SortExpressionRef<'_> {
    // SAFETY: `arg(0)` is a direct subterm of `self.term`, so `self.term`
    // is a valid parent witness for widening the borrow to `&self`.
    unsafe { self.term.arg(0).upgrade(&self.term) }.into()
}
```

Byte-for-byte identical to `data_function_symbol()` two methods above it — both return `arg(0)`
(the applied head symbol, e.g. `f` in `f(x)`), just reinterpreted through a different marker type.
`arg(0)` of a data application is never a `sort_expression`; the `mcrl2_term` macro's own
`debug_assert!(is_sort_expression(&term), ...)` inside `SortExpressionRef::from` exists precisely
to catch this tag mismatch, and does — in debug builds only.

Confirmed independently — debug build (the internal `debug_assert!` fires, proving the mismatch):

```
cd tools/mcrl2 && cargo test -p mcrl2 --test data_application_sort_test
```
```
test data_application_head_symbol_is_the_function_not_a_sort ... ok
test data_application_sort_panics_on_type_confusion_in_debug - should panic ... ok
```

Release build (the `debug_assert!` compiles out, so the wrong value is returned silently instead
of panicking — verified this too):

```
cargo test --release -p mcrl2 --test data_application_sort_test
```
```
test data_application_sort_silently_returns_function_symbol_not_result_sort_in_release ... ok
```
That test asserts `appl.sort().name() == "f"` (the function's own name, not its Nat result sort) —
holds.

Why it hasn't caused visible damage: `DataExpression::data_sort()` — the plausible call site — does
not handle the application case at all and panics with `"data_sort not implemented for {}"`
instead of delegating to `DataApplication::sort()`; grepped the whole `tools/mcrl2` tree
(including `graph_symmetry.rs`/`symmetry.rs`) and found zero call sites for
`DataApplication::sort()`/`DataApplicationRef::sort()`. Dead code with a plausible-sounding doc
comment and no caller — a live trap for the first person who reaches for it.

### Outcome: DEFERRED (documented, not fixed)

Direction of a fix: compute the actual result sort by decomposing the head function symbol's
(possibly curried) arrow sort by the number of applied arguments, rather than reusing
`data_function_symbol()`'s implementation verbatim. Not applied in this pass — zero current
callers means zero user-facing impact today, and a correct fix needs more research into how this
binding's sort-arrow decomposition API works than was budgeted here. The regression test
(`tools/mcrl2/crates/mcrl2/tests/data_application_sort_test.rs`) stays in the tree exactly as
written (documenting the bug is live); once fixed, its release-mode test should be replaced with
one asserting `sort.pretty_print() == "Nat"` (noted in the test's own doc comment already).

## Checked and found correct (agent's findings, not independently re-verified line by line)

- `DataExpressionRef::from_address` / `PbesExpressionRef::from_address`: GC-reachability
  contracts, traced through every call site in scope (`explore_srf.rs`, `explore_pbes.rs`,
  `explore_explicit.rs`) — each inserts the address into a `Protected`-backed set before any
  further FFI call that could trigger collection.
- `PbesFlattenStack`'s `unsafe impl Send` (`visitor.rs:348`): sound — the only read path
  (`PbesFlattenIter::next`) is only reachable via `PbesFlattenIter::new`, which clears the stack
  before pushing, so no address pushed on one thread can be read back on another.
- `{PbesSrfLps, PbesLps, ExplicitLinearProcessSpecification}: Sync`: sound — no field is mutated
  through `&self` after construction except the `Protected`-backed concurrent map, which provides
  its own genuine thread-safe interior mutability. A distinct claim from Finding 1 (sharing `&Self`
  for concurrent reads vs. moving a mutable per-thread C++ object).
- `QuotientLps<P>`/`QuotientSummand<P>`: `Sync` sound as actually used, but fragile — depends on
  no method exposing an independent `Arc<P>` clone to a caller; flagged as a design fragility
  worth a comment, not a defect.
- The `catch_unwind`/`std::process::abort()` trampoline pattern for C→Rust callbacks
  (`lps.rs::enumerate_raw_inner` and the explorer `enumerate` methods): correctly prevents a Rust
  panic from unwinding into a C++ frame.
- `PbesRewriteContext::rewrite_formula` (`pbes.rs:307`): a minor observation, not a demonstrated
  bug — the `unsafe` boundary is drawn around the wrong argument (`formula`, already safe/owned)
  rather than the real hidden precondition (σ from the preceding `set_assignments` call must
  still be live). Worth tightening the doc comment when someone next touches this function; text
  for the contract is in the original hand-back, not reproduced here.

## Tests added

- `tools/mcrl2/crates/mcrl2/tests/data_application_sort_test.rs` — Finding 2 (kept as-is, bug
  still live).
- `tools/mcrl2/crates/merc_pbes/tests/build_tests.rs` + `tests/input/{pbes_srf_context_not_send,
  pbes_context_not_send}.rs` — Finding 1, replacing the two runtime crash tests (now
  compile-fail).
- `tools/mcrl2/crates/merc_lps/tests/build_tests.rs` + `tests/input/explicit_context_not_send.rs`
  — Finding 1, replacing the one runtime crash test.

## Note on this report's provenance

The reviewing agent's session ended before it wrote this file to disk (only its hand-back message
carried the findings). Reconstructed here from that message, with the Finding 1 fix, its
verification, and the trybuild conversion added by the coordinating session.
