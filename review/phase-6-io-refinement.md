# Phase 6 — `crates/io` and `crates/refinement`

Coverage: `crates/io` (`bitstream`, `dumpfiles`, `format`, `line_iterator`,
`progress`, `traced_command`) and `crates/refinement` (`antichain`,
`counterexample_constructor`, `failures_refinement`, `impossible_futures`,
`refinement`). This phase exists to close a real scope gap: Phase 5's
coverage-gap audit found that `crates/io` and `crates/refinement` were listed
as covered by Phase 3 in the status table, but neither crate has any findings
or mention in `phase-3-algorithmic-core.md` or
`phase-3-vpg-typecheck-unification-automaton.md` — they were never actually
reviewed. This phase reviews them for real.

Both crates carry `#![forbid(unsafe_code)]`, confirmed by `grep -rn unsafe
crates/io/src crates/refinement/src` (only the `forbid` attribute itself
matches). So this is pure logic/IO review; the `unsafe-verify` skill does not
apply.

## Verdict

Both crates are sound at the level that matters most: `crates/io`'s
serialization round-trips correctly (including on malformed/truncated input,
which errors cleanly rather than panicking), and `crates/refinement`'s
`refines()` produces a boolean verdict that is robust — verified with
thousands of randomized cases, including divergence-heavy and self-loop-heavy
LTSs — to the choice of `ExplorationStrategy`, the `preprocess` flag, and
whether a counterexample is requested, and is reflexive (an LTS always refines
an identical copy of itself) under all five `RefinementType`s.

One real defect was found and confirmed: `crates/refinement`'s counterexample
construction violates its own documented contract by embedding literal `tau`
transitions in a "weak trace" counterexample, where the type's own doc comment
promises only the visible trace (`<tau*.a0.tau*.a1...>true` — implicit taus).
This is currently masked for the one production consumer
(`crates/syntax::generate_refinement_formula`, which happens to filter tau out
of the trace itself before building a formula), so it has not caused an
observed wrong end-user result — but it is a genuine, demonstrated violation
of `CounterExample`'s documented contract that any other consumer of the raw
`CounterExample` value (a debug printer, a test, a future tool, direct
`Vec<L>` comparison) would see as a wrong trace.

## Findings

### 1. `CounterExample::WeakTrace`/`Divergence`/`StableFailures`/`ImpossibleFutures` embed literal tau steps, contradicting the type's own doc comment — CONFIRMED

- **Location**: `crates/refinement/src/failures_refinement.rs:187` (`is_refinement_generic`'s
  main loop), which calls `counter_example.add_edge(impl_transition.label, ce)`
  for **every** outgoing transition explored on the impl side — including
  hidden/tau transitions — regardless of `weak_transition`. Only the *spec*
  side abstracts tau away via `tau_closure`; the *impl* side's raw transition
  labels (tau included) are recorded verbatim into the counterexample tree,
  and `CounterExampleConstructor::reconstruct_trace`
  (`crates/refinement/src/counterexample_constructor.rs:63`) returns them
  unfiltered.
- **Scenario**: `crates/refinement/src/counterexample_constructor.rs:9-19`
  documents `CounterExample::WeakTrace` as representing the formula
  `<tau*.a0.tau*.a1. ... .a_n.tau*>true` — i.e. a sequence of *visible*
  actions with taus implicit/elidable between them. But when the witnessing
  execution path happens to include a genuine tau step (as it will whenever
  the counterexample state is reached via a `-tau->` transition), that tau
  label ends up as a literal entry in the returned `Vec<L>`. For an
  implementation `0 -i-> 1 -a-> 2` (where `"i"` is the Aldebaran tau label)
  that performs the weak trace `"a"` a specification consisting of a single
  deadlocked state cannot match, `refines(..., RefinementType::Weaktrace, ...,
  true)` correctly returns `false`, but the returned
  `CounterExample::WeakTrace` trace is `["τ", "a"]`, not `["a"]`.
- **Evidence**:
  ```
  $ cargo test -p merc_refinement --test counterexample_content -- --nocapture
  thread '...' panicked at crates/refinement/tests/counterexample_content.rs:44:13:
  assertion `left == right` failed: the weak-trace counter example must report only the visible trace
  (the leading tau is implicit, per CounterExample::WeakTrace's own doc comment: `<tau*.a0.tau*.a1...>true`),
  but got ["τ", "a"]
    left: ["τ", "a"]
   right: ["a"]
  test weak_trace_counterexample_reports_the_visible_trace_not_raw_impl_steps ... FAILED
  ```
  This would pass once the trace returned by `is_refinement_generic`/
  `reconstruct_trace` for the weak variants has hidden labels filtered out
  (or is filtered at the point `CounterExample::WeakTrace`/`Divergence`/
  `StableFailures`/`ImpossibleFutures` are constructed in
  `crates/refinement/src/refinement.rs`), matching what `CounterExample::Trace`
  already gets correctly (strong trace legitimately treats tau as an ordinary
  action, so it is *not* filtered there — confirmed by reading
  `crates/syntax/src/counterexample_formula.rs`'s `generate_refinement_formula`,
  whose `CounterExample::Trace` arm does not filter, while its
  `weaktrace_formula` helper does: `.filter(|l| !l.is_tau_label())`).
- **Impact assessment (verified, not assumed)**: I checked every call site of
  `merc_refinement::refines`/`CounterExample` in the workspace
  (`grep -rn "CounterExample::" tools/ crates/`); the only match outside
  `crates/refinement` itself is `crates/syntax::generate_refinement_formula`,
  and its `weaktrace_formula` helper already strips tau labels
  (`crates/syntax/src/counterexample_formula.rs:193`) before building the
  modal formula, and the filtered result is a correct weak trace (the raw
  labels form a real execution path of the implementation, so removing tau
  from that literal path yields a genuine weak-observation trace). So today
  this does **not** produce a wrong formula for the one real caller — the
  defect is confirmed at the `CounterExample` API boundary, and is a latent
  risk for the next caller that trusts the doc comment rather than
  independently re-deriving that it must filter tau itself.
- **Status**: FIXED. The fix is at the source, in `is_refinement_generic`
  itself (`crates/refinement/src/failures_refinement.rs`): an impl-side
  transition is now recorded as a literal edge in the counter-example tree
  only when it is *not* an unobservable tau step under weak semantics
  (`is_unobservable_tau = weak_transition && merged_lts.is_hidden_label(...)`).
  For such a tau step, exploration still proceeds (the new `(impl', spec)`
  pair is still pushed to `working` and still tracked by the antichain), but
  no new counter-example tree node is created — the pair reuses the current
  node `ce` as its parent, so the tau step contributes nothing to the
  reconstructed trace. `CounterExample::Trace` (`weak_transition == false`)
  is untouched: strong trace refinement still records tau as an ordinary,
  literal action, per the original design. This also fixes the divergence
  path (`InnerCe::Diverges` failures reached via a preceding tau step) since
  it goes through the same tree-construction code, and the impossible-futures
  path (`crates/refinement/src/impossible_futures.rs`), which calls the same
  `is_refinement_generic` with `weak_transition = true` for both the outer
  and the inner (`is_weak_trace_refinement_ce`) checks.
  - **Regression test**: `cargo test -p merc_refinement --test
    counterexample_content -- --nocapture` now passes:
    `weak_trace_counterexample_reports_the_visible_trace_not_raw_impl_steps ... ok`
    (previously failed with `["τ", "a"]`, now returns `["a"]`).
  - **No regressions**: `cargo nextest run -p merc_refinement --no-fail-fast --
    --include-ignored` — 13/13 tests pass, including
    `refines_is_reflexive_on_random_ltss` and
    `refines_verdict_is_independent_of_preprocess_and_strategy` (the
    reflexivity/preprocess-strategy-independence property tests added by this
    phase) and all five `test_mcrl2_ltscompare_*` variants.
  - **Consumer unaffected/simplified**: `crates/syntax::generate_refinement_formula`
    needed no change. Its `weaktrace_formula` helper's defensive
    `.filter(|l| !l.is_tau_label())` (`crates/syntax/src/counterexample_formula.rs:193`)
    is now provably redundant for every trace it will ever see from
    `merc_refinement` (those traces can no longer contain a tau label at all),
    but it is left in place as harmless defense-in-depth rather than removed.
    `cargo test -p merc_syntax --lib --bins --test example_test --test
    grammar_test --test roundtrip_test` passes (21+41 tests). The pre-existing
    `multi_action_test` failures (`multi_action_eq_distinguishes_*`) are the
    already-tracked, unrelated `MultiAction::eq` defect from
    `review/phase-2-syntax.md` and are untouched by this change (verified they
    fail identically before this fix, since this fix does not touch
    `crates/syntax` at all — confirmed via `git diff --stat`, which shows only
    `crates/refinement/src/failures_refinement.rs` changed).
  - **Lint/format**: `cargo clippy -p merc_refinement -p merc_syntax
    --all-targets` shows no new warnings (the two pre-existing warnings in
    `crates/syntax/src/random_value_expression.rs` and
    `crates/syntax/src/traverse.rs` are unrelated and unchanged by this fix);
    `cargo +nightly fmt --all -- --check` shows no diff for
    `failures_refinement.rs` (unrelated pre-existing formatting diffs remain
    in other, untouched files elsewhere in the workspace).

## Checked and found correct

- **`refines()` verdict is reflexive.** `refines(lts.clone(), lts.clone(),
  refinement, strategy, preprocess, false, ..).0` is `true` for all five
  `RefinementType`s, both `preprocess` values and both `ExplorationStrategy`
  values, across 200 randomized LTSs (1–11 states, 1–4 labels). Test:
  `crates/refinement/tests/randomized_refinement_invariants.rs::refines_is_reflexive_on_random_ltss`.
- **`refines()` verdict is independent of `preprocess` and `ExplorationStrategy`.**
  Both are documented as pure optimisations (preprocessing "can lead to
  significant performance improvements" but should not change the outcome;
  BFS vs. DFS only affects which counterexample is found). Checked across 200
  randomized (impl, spec) pairs (spec random, impl a random mutation of spec)
  for all five refinement types: all four `(preprocess, strategy)`
  combinations always agree on the boolean verdict. Test:
  `crates/refinement/tests/randomized_refinement_invariants.rs::refines_verdict_is_independent_of_preprocess_and_strategy`.
  I additionally ran this same invariant, outside the committed suite, at
  higher stress (2000 iterations up to 40 states/8 labels/40 mutations in
  release mode, and separately 3000 iterations biased toward divergence-heavy,
  self-loop-heavy 1–2-label LTSs to specifically target the tau-cycle/
  self-loop edge cases this phase was asked to focus on) with no discrepancy
  found.
- **`quotient_lts_block`'s known "no bottom state" bug (Phase 3, `crates/reduction`)
  does not reach `crates/refinement` in practice.** `crates/refinement::refines`'s
  `preprocess=true` path calls `quotient_lts_block::<_, false>` for
  `RefinementType::Trace` (`BRANCHING=false`, so the pathological bottom-state
  search in `quotient_lts_block` — which is only compiled into the
  `BRANCHING=true` path — never runs), and calls `quotient_lts_block::<_,
  true>` for the weak variants only after `branching_bisim_sigref` has already
  called `tau_cycle_elimination_and_reorder` on the LTS (confirmed by reading
  `crates/reduction/src/signature_refinement.rs:104-121`), which is exactly
  the tau-SCC-collapsing precondition Phase 3 identified as making every block
  have a genuine bottom state. So although `crates/refinement` is one of the
  `pub fn quotient_lts_block` callers Phase 3 called out as at-risk in the
  abstract, tracing the actual call path shows it is not exposed to that bug.
  (I verified this instead of assuming it, per the review skill's "verify,
  don't assume" step — my first hypothesis, before reading
  `signature_refinement.rs`, was that this phase had found a second instance
  of the Phase 3 bug; it did not survive checking the actual call chain.)
- **`Antichain`/`AC` insert-and-dominate logic.** `contains_superset`/
  `contains_subset`'s subset directions match their doc comments; `insert`'s
  retain-and-replace logic (remove stored supersets of the new value, reject
  the new value if a stored subset already dominates it) matches the
  antichain invariant, and `check_consistency` (used by
  `test_random_antichain`, 100 iterations of 50 random inserts each) never
  fails.
- **`refusals_contained_in`'s optimised check vs. the naive powerset-based
  definition.** Both `refusals_contained_in` (the O(n²) enabled-set check)
  and `refusals_contained_in_naive` (literal `refusals(s) ⊆ refusals(spec)`
  over the powerset of each maximal refusal set) are cross-checked by
  `debug_assert!` on every call in the existing test suite (`cargo test`
  compiles them in dev profile), and neither assertion has ever fired across
  all of this phase's randomized runs.
- **`bitstream::{BitStreamReader, BitStreamWriter}` round-trip fidelity and
  error handling.** `write_bits`/`read_bits` correctly reject `> 64` bits
  (existing test); the underlying `bitstream-io` crate's `write_unsigned_counted`
  independently validates that a value fits the requested bit width (read the
  vendored source at
  `~/.cargo/registry/src/.../bitstream-io-4.10.0/src/write.rs`) — writing a
  value that does not fit the given `number_of_bits` fails cleanly rather than
  silently truncating. `read_string` on a length that exceeds the remaining
  stream data errors rather than panicking (new test:
  `crates/io/tests/truncated_stream.rs::read_string_on_truncated_stream_errors_instead_of_panicking`),
  as does `read_integer` on a completely empty stream
  (`read_integer_on_empty_stream_errors_instead_of_panicking`). Existing tests
  already cover invalid-UTF-8 rejection, unicode round-tripping and edge-case
  strings (empty, multi-byte, 10,000 chars).
- **`format::{BytesFormatter, LargeFormatter}`.** Verified the comma-insertion
  arithmetic by hand for several digit-count boundaries (3, 4, 6 digits) and
  confirmed it matches the existing `test_large_formatter_*` tests; unit
  selection is monotonic in magnitude (`test_bytes_formatter_random_unit_selection`,
  1000 iterations) with no boundary off-by-one.
- **`progress::TimeProgress`.** The compare-exchange in `print` correctly
  ensures at most one caller "claims" a given interval under concurrent calls
  (reasoned through the interleaving: a stale `last` read only lets the CAS
  succeed for the thread that has not yet been beaten to the update).
  `saturating_sub` prevents underflow if `elapsed < last` (cannot currently
  happen since `Instant::elapsed` is monotonic, but is defensive regardless).
- **`line_iterator::LineIterator`.** Empty input, single line with no
  trailing newline, `\n`- and `\r\n`-terminated lines, and consecutive empty
  lines all produce the expected `String`s; invalid UTF-8 surfaces as an
  `io::Error` via `error()` rather than a panic (by inspection of
  `BufRead::read_line`'s documented behaviour, exercised transitively by the
  existing tests).

## Tests added

- `crates/refinement/tests/randomized_refinement_invariants.rs` —
  `refines_is_reflexive_on_random_ltss`,
  `refines_verdict_is_independent_of_preprocess_and_strategy`. Both pass;
  regression coverage closing the gap this phase exists to close (no prior
  test exercised `refines()` against randomly generated LTSs without the
  external `MCRL2_PATH` oracle, so none of this ran in a normal CI checkout).
  Run with `cargo test -p merc_refinement --test randomized_refinement_invariants`.
- `crates/refinement/tests/counterexample_content.rs` —
  `weak_trace_counterexample_reports_the_visible_trace_not_raw_impl_steps`.
  **Fails on the current tree** (see Finding 1); this is the regression test
  for that defect. Run with
  `cargo test -p merc_refinement --test counterexample_content -- --nocapture`.
- `crates/io/tests/truncated_stream.rs` —
  `read_string_on_truncated_stream_errors_instead_of_panicking`,
  `read_integer_on_empty_stream_errors_instead_of_panicking`. Both pass;
  closes the malformed/truncated-input coverage gap this phase was scoped to
  check. Run with `cargo test -p merc_io --test truncated_stream`.

All of the above (except the deliberately failing regression test) pass
`cargo fmt -p merc_io -p merc_refinement -- --check` and
`cargo clippy -p merc_io -p merc_refinement --tests --no-deps` with no
warnings.
