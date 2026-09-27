# Phase 3 — Remaining algorithmic crates

Coverage: `crates/symbolic`, `crates/reduction`, `crates/sabre` (automaton
construction), `tools/lts`, `tools/mcrl2/crates/merc_pbes` (`graph_symmetry`).
The `vpg`/typecheck-unification/`sabre` automaton agent was cut off by a
session rate limit before delivering findings; not covered here.

## Findings

### 1. `merc-lts convert` silently corrupted the tau label on AutMcrl2 output — FIXED

- **Location**: `tools/lts/src/main.rs`, `handle_convert`'s `GenericLts::AutMcrl2`
  and `GenericLts::Lts` match arms.
- **Scenario**: both arms matched `LtsFormat::Aut | LtsFormat::AutMcrl2` together
  and always called `write_aut` (the Aldebaran dialect, tau label `"i"`),
  regardless of which output format was requested. Converting to AutMcrl2
  output (`--output-format aut-mcrl2`) silently rewrote every `"tau"` label to
  `"i"`, corrupting round-trips through the mCRL2 dialect.
- **Evidence**: `tools/lts/tests/convert_cli.rs::convert_aut_mcrl2_to_aut_mcrl2_preserves_tau_label`
  failed before the fix (`assert` on the output containing `"tau"`), passes after.
- **Fix**: split the two format cases; `LtsFormat::AutMcrl2` now calls
  `write_mcrl2_aut`. Verified with `cargo test -p merc-lts`, clippy and fmt clean.

### 2. `quotient_lts_block` (branching) mishandles a block with no bottom state — FIXED

- **Location**: `crates/reduction/src/quotient.rs`, `quotient_lts_block::<_, true>`'s
  bottom-state search (the `debug_assert!(!diverges(lts, candidate), ...)` guard).
- **Scenario**: the search assumed every block has a genuine bottom state, which
  holds for partitions produced by the crate's own `reduce_lts` (tau-SCCs are
  always collapsed first — `tau_cycle_elimination_and_reorder` runs before
  `branching_bisim_sigref_impl`, so the resulting block's tau-subgraph is a
  subgraph of a DAG and always has a sink) but was undocumented and unchecked
  for `quotient_lts_block` itself, a `pub fn` callable with any caller-supplied
  `BlockPartition` (e.g. from `merc_refinement`). On a block containing a
  tau-cycle, the cycle-detection was a `debug_assert!`: it panicked in debug
  builds and was compiled out in release builds, where the search silently
  picked a non-bottom-state representative and dropped that block's non-tau
  transitions from the quotient. A second, independent issue in the same
  search: even the bottom-detection walk itself only followed one arbitrarily
  chosen tau successor per state, so it could report "no bottom state" simply
  because the edge it happened to follow led into a cycle, even when another
  edge from the same state led to a genuine bottom state elsewhere in the
  block.
- **Root cause**: a block can legitimately have no bottom state under
  branching bisimulation. `{0, 1, 2}` with `0 -tau-> 1`, `1 -tau-> 2`,
  `2 -tau-> 1`, `2 -x-> 3` is a genuine branching-bisimulation-stable block
  (by the branching condition, `1 ~ 2` because `2`'s `-x->` is weakly matched
  by `1 =tau=> 2 -x-> 3`, and `2 ~ 1` symmetrically via the inert
  `2 -tau-> 1`), yet every state in it has an outgoing same-block tau
  transition — there is no single state whose direct transitions capture the
  whole block's behaviour, since only state `2` performs `x` directly. This
  cannot arise from the crate's own `reduce_lts` pipeline (which always
  tau-SCC-collapses first, see above), but is fully reachable through the
  public API with a hand-built `BlockPartition`.
- **Fix**: the bottom-state search now explores *all* outgoing tau edges of
  every visited state via a worklist (not one arbitrary successor at a time),
  so it cannot be fooled by picking the "wrong" edge when a real bottom state
  is reachable via another. When no bottom state is reachable at all — the
  visited states form one or more tau-cycles with no exit — every visited
  state becomes a representative instead of one arbitrary (non-bottom) state,
  so their transitions (e.g. state 2's `x`-transition) are unioned into the
  quotient instead of being dropped. The former `debug_assert!(!diverges(..))`
  is now a real, always-true invariant check on the fallback path rather than
  an assertion of "this never happens".
- **Evidence**: `crates/reduction/src/quotient.rs::test_quotient_lts_block_branching_uses_true_bottom_state`,
  un-`#[ignore]`d. Hand-built LTS with a 2-state tau-cycle reached via a tail
  state, one visible transition only reachable from the cycle's far state.
  Confirmed the test fails without the fix (reverted the fix locally, kept
  the test un-ignored: panics at `crates/reduction/src/quotient.rs:224`,
  `"The states of the given LTS should be non-divergent."`) and passes with
  it restored:
  ```
  $ cargo test -p merc_reduction --lib quotient::tests::test_quotient_lts_block_branching_uses_true_bottom_state
  test quotient::tests::test_quotient_lts_block_branching_uses_true_bottom_state ... ok
  ```
- **Status**: FIXED. Full crate suite including previously-ignored tests:
  `cargo nextest run -p merc_reduction --no-fail-fast -- --include-ignored`
  → `46 tests run: 46 passed (8 slow), 0 skipped`. Downstream: `cargo test -p
  merc_lts` → 19 passed + 1 doctest. No dedicated quotienting tests exist in
  `tools/lts/tests` or `tools/rewrite` (`tools/rewrite` has no `tests/`
  directory at all; `grep -rn quotient tools/lts/tests` finds nothing —
  `tools/lts/src/main.rs` is the only non-test hit). `cargo clippy -p
  merc_reduction --all-targets` clean, `cargo +nightly fmt -p merc_reduction
  -- --check` clean.

### 3. Two test-quality fixes (not bugs in reviewed production code)

- `tools/mcrl2/crates/merc_pbes/src/graph_symmetry.rs::probe_empty_pbes` was a
  print-only test with no assertion; replaced with
  `empty_pbes_is_rejected_by_the_parser_before_reaching_build_sdg`, which
  asserts `Pbes::from_text` actually rejects a zero-equation PBES (confirming
  `unified_parameters`'s empty-vector branch is unreachable via the text parser).
- `crates/sabre/tests/automaton_construction_tests.rs::test_multiple_nested_rules_build_automaton`
  asserted an under-specified expected value (`h(k(a))`, one rewrite step short
  of normal form, for a 4-rule spec whose normal form is `b`). Fixed the
  expectation; `SetAutomaton` construction itself was not at fault.

## New coverage (no defect found)

- `crates/symbolic/src/symbolic_lps_explore.rs::test_detect_deadlocks_finds_the_grid_corner`:
  exercises `ReachabilityOptions::detect_deadlocks` through the real
  `SymbolicLpsGroup`/`TransitionGroup::learn_successors` adapter (previously only
  covered by `crate::ldd::symbolic_explore`'s hand-built groups), across every
  `SummandGrouping`/cache/`ExplorationStrategy` combination. All combinations
  agree on the expected reachable-state and deadlock count.
- `crates/symbolic/src/util.rs::test_random_bdd_renaming_varied_substitutions`:
  `variable_rename`/`variable_rename_reverse`'s existing random tests only ever
  exercised one fixed substitution shape (`[(0,1),(2,3)]`); this varies the
  number of entries and the gaps between them, stressing
  `variable_rename_edge`'s cache-key soundness (same node reached via different
  remaining-substitution suffixes). 100 iterations, passes against an oracle
  built from `BDDFunction::substitute`.

Both new symbolic tests pass single-threaded; running the full `merc_symbolic`
lib suite with the default (parallel) test harness triggered an OOM kill in
this container (~14GB RSS, 15GB available) from several oxidd BDD/LDD managers
live at once — a container memory limit, not a defect in the new tests or the
reviewed code.

## Verification

```
cargo test -p merc_reduction --lib          # 38 passed, 1 ignored (finding #2, pre-fix)
cargo nextest run -p merc_reduction --no-fail-fast -- --include-ignored   # 46 passed, 0 skipped (post-fix; 39 lib + 7 ltsconvert_test)
cargo test -p merc_symbolic --lib -- --test-threads=1   # 75 passed
cargo test -p merc_sabre --test automaton_construction_tests   # 3 passed
cargo test -p merc-lts                       # 5 passed (finding #1's regression test)
cd tools/mcrl2 && cargo test -p merc_pbes --lib empty_pbes_is_rejected   # 1 passed
cargo clippy -p merc_syntax -p merc_typecheck -p merc-lts --all-targets
cargo +nightly fmt -p merc_aterm -p merc-lts -- --check

# Finding #2's fix, added afterwards:
cargo test -p merc_reduction --lib quotient::tests::test_quotient_lts_block_branching_uses_true_bottom_state
  # ok, both with the fix applied and (confirmed separately, fix reverted) panicking without it
cargo test -p merc_lts                       # 19 passed + 1 doctest
cargo clippy -p merc_reduction --all-targets # clean
cargo +nightly fmt -p merc_reduction -- --check   # clean
```
