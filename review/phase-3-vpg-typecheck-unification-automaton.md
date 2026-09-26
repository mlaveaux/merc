# Phase 3 — vpg (parity game solving), typecheck unification, sabre set_automaton

Coverage: `crates/vpg` (all of `parity_games/`, `symbolic/`, `variability_zielonka.rs`,
`zielonka.rs`, `priority_promotion.rs`, `submap.rs`), `crates/typecheck/src/inference/unification.rs`
(the crate's only "unif"-named module — `ResolvedSortId` widening in
`crates/typecheck/src/inference/resolved_sort.rs` was skimmed but not exhaustively
verified), and `crates/sabre/src/set_automaton/` (`automaton.rs`, `match_goal.rs`,
`display.rs`).

A previous agent's abandoned lead ("minus/minus_function semantics" in
`variability_zielonka.rs`, and a flaky-looking `..._optimised_left --release`
nextest run) was re-investigated from scratch below (Finding 1). The flake was
not reproduced (11 seeds × 100 iterations, release and debug); the semantics
lead did turn up a real inconsistency, but not the one that would explain a
release-only flake.

## Findings

### 1. `zielonka_family_optimised`'s `C_restricted` inverts the `alternative_solving` condition used everywhere else in the file — CONFIRMED (as a code defect), verified NOT to change results

- **Location**: `crates/vpg/src/variability_zielonka.rs:391-398`.
- **The inconsistency**: every other place in this file that has to pick between
  "the universe is all BDD assignments" and "the universe is the game's actual
  configuration space" uses the same condition:
  - `solve_variability_zielonka`'s initial `V` (line 62): `if alternative_solving { true_bdd } else { game.configuration() }`
  - `attractor`'s `minus_edge` call (line 562-566): `if self.alternative_solving { true_bdd } else { configuration() }`
  - `C1_restricted` in the very same function, 30 lines below (line 421-428): `if self.alternative_solving { true_bdd } else { configuration() }`

  But `C_restricted` (line 391-398) has the condition **inverted**:
  ```rust
  let C_restricted = minus(
      &if !self.alternative_solving {
          self.true_bdd.clone()
      } else {
          self.game.configuration().clone()
      },
      &C,
  )?;
  ```
  This is the only one of the four call sites in the file that negates
  `alternative_solving` before branching.
- **Why it doesn't (currently) produce a wrong result**: `C_restricted` feeds
  into exactly one place, `omega1_not_x_restricted`, which is used only for
  the `if omega1_not_x_restricted.is_all_empty_set()` branch decision (it is
  never used again — `C1_restricted`, not `C_restricted`, drives the actual
  `alpha1`/`gamma_restricted` computation in the `else` branch). Algebraically,
  `f.minus_function(universe.minus(C)) == f ∩ universe^c ∪ f ∩ C`, which
  collapses to `f ∩ C` whenever `f ⊆ universe`. `omega1_not_x` (`f` here) is
  always a subset of `true_bdd` trivially, so with `alternative_solving =
  false` (`!false` picks `true_bdd`, the "wrong" choice happens to coincide
  with a value that always contains `f`) the bug is masked. With
  `alternative_solving = true` the wrongly-chosen universe is
  `game.configuration()`, a proper subset of `true_bdd` in general, and
  `omega1_not_x` genuinely ranges outside it in this solving mode (by design —
  see the type's own doc comment on `alternative_solving`) — so the branch
  decision *can* differ from a correctly-computed one. But taking the `else`
  branch instead of the `if` shortcut still runs the algorithm's general
  (always-correct) recursive step, just without the intended short-circuit.
- **No test in the tree exercises this combination at all**: `grep` for
  `FamilyOptimisedLeft` combined with `alternative_solving = true` turned up
  nothing (`tools/vpg/src/main.rs` always passes `false`; the only existing
  test, `test_random_variability_parity_game_solve_optimised_left`, also only
  passes `false`). I added
  `test_random_variability_parity_game_solve_optimised_left_alternative_solving_agrees_with_family`
  in `crates/vpg/src/variability_zielonka.rs` to close that gap; it compares
  `FamilyOptimisedLeft` against `Family` with `alternative_solving = true` and
  passed on every seed tried (750+ random games total: the default random
  seed plus explicit seeds 11-20, 101, 202, 303, each running 50-100
  iterations). This matches the algebraic argument above: the bug degrades
  the "left-optimised" algorithm to always taking its general-case branch
  under `alternative_solving = true` (defeating the point of the optimization
  in that mode) without producing a wrong final partition.
- **Status**: CONFIRMED as a real inconsistency with the codebase's own
  established invariant (verified by direct comparison against the other
  three call sites), but I could not make it produce a wrong end-to-end
  result, and have a specific algebraic reason to expect it can't for this
  particular use site. Not reported as a correctness bug. Fix direction: drop
  the `!` in the `C_restricted` condition to match `C1_restricted`/the other
  two sites; harmless as far as I can tell, but keeps the "left-optimised"
  variant's complexity guarantee meaningful once anything calls it with
  `alternative_solving = true`.
- The originally-reported flaky `test_random_variability_parity_game_solve_optimised_left
  --release` run was **not reproduced**: ran with `MERC_SEED` 1 through 20 in
  `--release` (100 iterations each, 2000 games total) with no failure. That
  test only ever exercises `alternative_solving = false`, which the analysis
  above shows is unaffected by this bug, so it is very unlikely this bug was
  the cause of that one earlier failure; more likely a one-off CI flake
  unrelated to this code (not reproduced, so not investigated further).

## Checked and found correct

- **`crates/typecheck/src/inference/unification.rs`** (the whole file, all
  ~440 non-test lines): read end-to-end. `UnifyValue::unify_values`'s
  `debug_assert!` (at most one side of a merge is bound) holds because the
  only call to `unify_var_var` follows `shallow_normalize` on both sides,
  which strips exactly the case where a side is bound. The occurs check
  (`Unifier::occurs`) correctly detects cycles through a variable merged
  (but not yet bound) with the one being bound to a structure containing it
  — verified this is exercised by the existing
  `test_occurs_check_rejects_cyclic_sort`'s second case (merge-then-detect)
  and confirmed by re-reading `shallow_normalize`/`table.unioned` together.
  `Unifier::free_root`'s claim ("two nodes eagerly unified to the same free
  variable return the same root") holds because `table.find` is computed on
  the union-find key itself, not on the arena id, so it is insensitive to
  which of several merged arena nodes was used to reach it.
  `resolve`/`resolve_or_default`/`strict_super_sorts`/`strict_sub_sorts` were
  traced against their existing unit tests (container head-only widening,
  function contravariance, number-generality ordering) and match.
  `bind`'s documented "table may retain partial bindings on failure, callers
  must snapshot/rollback" contract was checked against the `Function`-`Function`
  and `Resolved`-`Function` arms, which do commit early-argument bindings
  before a later argument can fail — consistent with the documented contract,
  not a bug, provided callers honor it (did not audit every call site in
  `inference.rs` for snapshot discipline; flagging as unverified rather than
  claiming it as fact for the whole crate).
- **`crates/vpg/src/submap.rs`**: `minus`/`or`/`and_function`/`minus_function`
  and their `non_empty_count` invariant maintenance. Verified `imp_strict_edge`'s
  semantics independently against the vendored oxidd source
  (`oxidd-rules-bdd/src/complement_edge/apply_rec.rs:916`, which rewrites
  `ImpStrict(f, g)` as `!f & g` for quantification), confirming
  `Submap::minus`'s `imp_strict_edge(other, self)` computes `self \ other` as
  intended (matches `merc_symbolic::minus`'s own `rhs.imp_strict(lhs)` pattern
  in `crates/symbolic/src/bdd/cube_iter.rs`).
- **`crates/vpg/src/zielonka.rs`**: `x_and_not_x`/`combine`/`x_and_not_x_strategy`/
  `combine_with_strategy` player-indexed tuple helpers — all four correctly
  swap on `Player::Odd` and are consistent with each other (`combine` undoes
  exactly what `x_and_not_x` does).
- **`crates/vpg/src/parity_games/parity_game.rs`**: the `make_total` sink/self-loop
  construction's `Priority::new(owner.index(vertex_idx).opponent().to_index())`
  looked like a `VertexIndex`/`Priority`/player-index mix-up at first read, but
  is a deliberate (and correct) encoding: a forced self-loop's priority parity
  is set to make the *opponent* of the stuck vertex's owner win the infinite
  play, using `to_index()`'s 0/1 values as the priority's parity directly.
  Traced through `Player::from_priority` and confirmed consistent.
- **`crates/vpg/src/parity_games/variability_predecessors.rs`**: standard
  CSR-style incoming-edge index construction (count, prefix-sum, place,
  restore offsets, sentinel) — mirrors `ParityGame`'s own outgoing-edge
  construction and has no off-by-one in the offset restoration.
- **`crates/sabre/src/set_automaton/automaton.rs`**: `compute_derivative`,
  `classify_match_goal`, `build_set_automaton_destinations`,
  `add_fresh_match_goals`, and `MatchAnnouncement::symbols_seen`'s bookkeeping
  (which `SabreRewriter::apply_rewrite_rule` uses to compute `prune_point =
  leaf_index - announcement.symbols_seen` — a `usize` subtraction that would
  panic on underflow in debug and silently wrap in release if `symbols_seen`
  were ever wrong). This crate had **no randomized test coverage at all**
  before this review (`grep -rl random_test crates/sabre/` was empty). Added
  `crates/sabre/tests/random_arithmetic_rewrite_test.rs`, a from-scratch
  randomized cross-check (Peano arithmetic + booleans, rules chosen so
  `plus`/`mult` share a `zero`/`s(x)` head split and `eq` has all four head
  combinations, exercising the same GCP/partition/fresh-goal machinery as the
  hand-written automaton tests) comparing `SabreRewriter` and `NaiveRewriter`
  against an independent Rust evaluator. 200 iterations × 2 term shapes, run
  in both `--release` and debug (`debug_assert!`s active) with no failures —
  no evidence of a `symbols_seen`/pruning bug in the class of terms this
  generates (depth ≤ 3, values ≤ ~50).
- **`crates/sabre/src/set_automaton/automaton.rs`**: `is_supported_rule` /
  `variables_occur_in_lhs` / `is_supported_term` — traced against the file's
  own three unit tests (unbound rhs variable, bound rhs variable, unbound
  condition variable) and the logic matches.

## Not conclusively resolved

- `crates/typecheck/src/inference/resolved_sort.rs::widening_distance` (flagged
  by repowise as one of the highest-`weighted_deficit` files in the whole repo,
  `nested_complexity`, `widening_distance nests 4 levels deep`) computes an
  overload-resolution distance metric with head/interior components,
  contravariant function-domain handling, and container/number widening. I
  traced its logic against `number_generality`/`generic_op_partial_cmp` and it
  is internally consistent, but verifying it picks the *correct* overload in
  every ambiguous-overload scenario would require reconstructing the mCRL2
  overload-resolution specification this is implementing, which was outside
  this review's time budget. Not reporting a finding here since I have no
  concrete wrong-result scenario, only unverified residual risk in a
  known-complex file — flagging for a future pass rather than padding this
  report with a guess.

## Tests added

- `crates/vpg/src/variability_zielonka.rs`:
  `test_random_variability_parity_game_solve_optimised_left_alternative_solving_agrees_with_family`
  (new `#[merc_test]` in the existing `tests` module) — the first test in the
  tree to exercise `VpgSolver::FamilyOptimisedLeft` with `alternative_solving
  = true`; documents and guards Finding 1.
- `crates/sabre/tests/random_arithmetic_rewrite_test.rs` (new file) — two
  randomized tests,
  `test_random_arithmetic_terms_normalize_to_their_evaluated_value` and
  `test_random_boolean_terms_normalize_to_their_evaluated_value`, the first
  randomized regression coverage for `crates/sabre/src/set_automaton`'s
  construction against an independent oracle.

## Verification

```
# vpg (release; debug also run for the new test, see Finding 1)
cargo test -p merc_vpg --release --lib -- --test-threads=1     # 37 passed
cargo test -p merc_vpg --release -- --test-threads=1           # all integration tests + doctest passed
for seed in 1..20 101 202 303; do MERC_SEED=$seed cargo test -p merc_vpg --release --lib \
  variability_zielonka::tests::test_random_variability_parity_game_solve_optimised_left \
  variability_zielonka::tests::..._alternative_solving_agrees_with_family; done   # all ok

# sabre
cargo test -p merc_sabre --release --test random_arithmetic_rewrite_test    # 2 passed
cargo test -p merc_sabre --test random_arithmetic_rewrite_test              # 2 passed (debug, debug_assert active)

# lint/format on touched files
cargo clippy -p merc_vpg -p merc_sabre --all-targets     # no new warnings in touched files
cargo +nightly fmt -p merc_vpg -p merc_sabre -- --check  # clean for touched files
                                                          # (one pre-existing, unrelated diff in
                                                          # crates/sabre/src/utilities/term_stack.rs,
                                                          # not touched by this review)
```

Note: `cargo test -p merc_vpg --release` with the default (parallel) harness
gets SIGKILL'd (OOM) in this container from multiple concurrent oxidd BDD
managers — same container-memory-limit artifact noted in
`review/phase-3-algorithmic-core.md` for `merc_symbolic`, not a defect. Use
`--test-threads=1`.
