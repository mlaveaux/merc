# Two confirmed constraint-solver bugs blocking richer generated specs

Found while extending the random generators to draw on struct/container/function
sorts (a `Function`-sorted value generates as a `lambda`, and richer
struct/container sorts generate `Set`/`Bag` literals needing several elements).
Both are real, reproduce with a two-line spec, and root-caused precisely.
Fixes were designed and verified against the specific repro, but **not
merged** -- see "Why not fixed" below for each.

## 1. Lambda bodies get no widening against their expected range

- **Location**: `crates/typecheck/src/inference/inference.rs`,
  `ConstraintGenerator::visit`'s `DataExprKind::Lambda` arm.
- **Scenario**: `let function_sort = self.with_binder_scope(variables, |this, sorts| { let body_sort = this.visit(body)?; ...; Ok(this.unifier.function(parameters, body_sort)) })?;` uses `body_sort` directly as the lambda's range, with no `Sub` constraint letting it widen. Contrast `Application`'s arguments a few lines above, which route through a fresh variable specifically so the argument may upcast. A lambda body whose own natural (unwidened) sort differs from what the lambda's declared/expected range requires has nothing to widen against and fails to type-check, even though the identical expression works fine outside a lambda.
- **Minimal repro**: `sort S = struct c0; map f: (Nat -> Bag(S)); eqn f = lambda x: Nat . {c0: 1};` -- `{c0: 1}`'s natural sort is `FBag(S)`, needs to widen to `Bag(S)`. Fails: `has no valid sort assignment`. The same body outside a lambda (`map f: Bag(S); eqn f = {c0: 1};`) type-checks fine.
- **Designed fix**: bind the body through a fresh `range` variable with a `Constraint::Sub(SubConstraint { lhs: body_sort, rhs: range })`, then use `range` (not `body_sort`) as the lambda's range -- mirroring the `Application` argument pattern exactly.
- **Why not fixed**: verified against the repro and the full `merc_typecheck` test suite cleanly (all 496 tests across every test binary), including regenerating two golden snapshot diffs that a from-first-principles read of `lowering::mcrl2_lowering::Lowerer::coerce` proved are cosmetic (a numeral literal is lowered directly at its context's target sort regardless of which resting point the solver's own bookkeeping settled on -- see that function's doc comment). **However**, `tools/mcrl2/crates/mcrl2/tests/lowering_conformance.rs::test_round_trip_lambda_and_higher_order` -- which cross-checks the lowered term against the real mCRL2 reference toolset via FFI, not just an internal snapshot -- then failed: `inc = lambda y: Nat. y + 1;` lowers to a structurally different term than the oracle produces.

  Dug further (per explicit instruction not to just work around it): this is not merely a different-but-valid overload choice. Dumping the actual lowered `Mcrl2DataSpecification` for `map inc: Nat -> Nat; eqn inc = lambda y: Nat. y + 1;` with the fix applied shows the standard-sort machinery generating a full battery of system equations (`==`, `!=`, `<`, `<=`, `>`, `>=`, `less_total`, `if`) for a **spurious function sort `(Nat # Pos) -> Pos`** that has no business existing for this specification at all -- alongside the correct ones for `Nat -> Nat`. The fresh `range` variable this fix introduces is evidently being picked up by the "inferred-only function sort" collection pass in `crates/typecheck/src/lowering/instantiate.rs` (the same mechanism `function_update_inferred_sort_test.rs` exercises and documents as fragile -- see the pre-existing, already-broken `test_function_update_on_lambda_only_sort_gets_ground_equations`) before it resolves to its final value, so a transient/intermediate sort gets misidentified as a real inferred function sort and needlessly instantiated. This is a genuine defect introduced by this fix, not an acceptable divergence -- reverted a second time rather than shipped. A real fix needs to either keep the fresh `range` variable from being visible to that collection pass until fully resolved, or take a different approach entirely (e.g. constrain the range more directly against the lambda's already-known expected sort, when one exists, instead of via a fully free fresh variable).

## 2. `solve_join`'s fast path never backtracks

- **Location**: `crates/typecheck/src/inference/inference.rs`, `Solver::solve_join`.
- **Scenario**: when 2+ `Sub` constraints target the same free variable (e.g. every element of a `Set`/`Bag` literal), `merge_shared_subs` merges them into one `Join` constraint computing their least-upper-bound directly, replacing the old order-sensitive two-`Sub` encoding (see the `Join` struct's own doc comment, which already describes the general class of bug this replaced). The fast path in `solve_join` computes `lub` from the sources' mutual join, `unify`s `join.target` to it, and calls `self.solve(index + 1)` -- but if that later solving fails, `solve_join` just returns `false` with **no fallback** to `solve_join_seq` (the per-source sequential search it already has, sitting right below, used for the "not materializable"/"can't join" cases). A `target` that needs to be *wider* than every source's own mutual join (because a *later* constraint -- e.g. an enclosing container -- requires it) has no way to be found.
- **Minimal repro**: `map v: Set(Bag(Nat)); eqn v = { {1: 1}, {2: 1} };` -- both elements' natural type is `FBag(Nat)`, joining trivially to `FBag(Nat)`; the declared `Set(Bag(Nat))` needs `Bag(Nat)`, one step wider than the join. Fails: `has no valid sort assignment`.
- **Designed fix**: snapshot before the fast-path `unify`+`solve`, and on failure `rollback_to` the snapshot and fall through to `solve_join_seq` (already written, already correct) instead of returning `false` directly.
- **Why not fixed**: this fix alone (without the Lambda fix) was verified clean against the *entire* `merc_typecheck` test suite including the oracle-conformance suite in `tools/mcrl2`, with one caveat -- it also produces the same class of cosmetic golden-snapshot diff as finding 1 (traced to the same `coerce()` literal-typing mechanism, same conclusion: benign). It was reverted together with finding 1 rather than shipped alone, purely for lack of time in this session to isolate and re-verify it independently after finding 1's oracle regression; there is no known problem with it on its own. **This one is likely safe to ship as a standalone fix** if someone has time to re-verify in isolation (repeat: apply only this fix, run `cargo test -p merc_typecheck --lib --tests`, regenerate/inspect the two `game_of_goose*` snapshot diffs against `coerce()`'s reasoning above, then run `cargo test -p mcrl2 --test lowering_conformance`).

## Practical impact

Both bugs make `crates/syntax/src/random_value_expression.rs` (a general
random-value generator for an arbitrary declared sort, added alongside this
finding) fail when the target sort involves a `Function` sort (finding 1) or
a multi-element `Set`/`Bag` literal nested inside another container
(finding 2, **fixed and shipped** -- see above). Finding 1 (Lambda-body
widening) remains unfixed after two attempted fixes, both reverted for
concrete, demonstrated reasons (arithmetic-overload divergence from the
mCRL2 oracle, then a spurious function-sort instantiation bug of the fix's
own making) -- not for being merely different from the reference toolset.
Per explicit instruction, `random_value_expression` still generates
`Function`-sorted values (lambdas) rather than avoiding that sort kind, so
the PBES/PRES/process fuzz tests built on it will surface this known,
already-documented bug on some fraction of runs rather than reliably
passing; that is treated as a fuzzer correctly reporting a real, tracked
defect, not something to hide by narrowing what the generator produces.
