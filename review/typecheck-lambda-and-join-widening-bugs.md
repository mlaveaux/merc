# Two constraint-solver bugs found while fuzzing richer generated specs (both fixed)

Found while extending the random generators to draw on struct/container/function
sorts (a `Function`-sorted value generates as a `lambda`, and richer
struct/container sorts generate `Set`/`Bag` literals needing several elements).
Both are real, reproduced with a two-line spec, root-caused precisely, and are
now fixed and shipped.

## 1. Lambda bodies got no widening against their expected range

- **Location**: `crates/typecheck/src/inference/inference.rs`,
  `ConstraintGenerator::visit`'s `DataExprKind::Lambda` arm, plus
  `crates/typecheck/src/lowering/mcrl2_lowering.rs`'s `Lowering::lower_lambda`.
- **Scenario**: the lambda's range was bound directly to `body_sort`, with no
  `Sub` constraint letting it widen. Contrast `Application`'s arguments a few
  lines above, which route through a fresh variable specifically so the
  argument may upcast. A lambda body whose own natural (unwidened) sort
  differs from what the lambda's declared/expected range requires had nothing
  to widen against and failed to type-check, even though the identical
  expression works fine outside a lambda.
- **Minimal repro**: `sort S = struct c0; map f: (Nat -> Bag(S)); eqn f = lambda x: Nat . {c0: 1};`
  -- `{c0: 1}`'s natural sort is `FBag(S)`, needs to widen to `Bag(S)`.
- **Fix, part 1 (inference)**: the body is bound through a fresh `range`
  variable with `Constraint::Sub(SubConstraint { lhs: body_sort, rhs: range })`,
  then `range` (not `body_sort`) is used as the lambda's range -- mirroring the
  `Application` argument pattern exactly, and applied unconditionally to every
  lambda (not scoped to a particular body shape).
- **Fix, part 2 (lowering)**: a *function value* has no term-level coercion to
  a different function sort (unlike a number or a container, there is no
  wrapper `coerce()` can build between two `Function` sorts -- see
  `Unifier::strict_related_sorts`'s `Function` case, which deliberately
  returns no related sorts for exactly this reason). So the widening the
  fresh `range` variable records has to be materialized one level down, at
  the body itself, which *is* a plain, coercible value. `lower_lambda` now
  looks up the lambda's own resolved range and calls
  `self.coerce(body_id, body_term, body_sort, range)` on the lowered body
  before wrapping it in the `Binder(Lambda, ...)`.
- **First two attempts were reverted, and why they were wrong**: an earlier
  version of fix part 1 alone (with no corresponding lowering change) passed
  Phase-3 inference but then hit a **Phase-4 lowering panic**
  (`user equation '...' passed Phase-3 inference but failed Phase-4
  lowering`) for exactly the kind of body that needs widening, because
  nothing ever inserted the coercion the solver had accepted as valid. A
  later attempt to fix this the *other* way -- letting a function *sort*
  itself widen covariantly in its range, via `Unifier::strict_related_sorts`
  -- type-checked and looked principled, but produced the exact same Phase-4
  panic for a different reason: `Lowerer::coerce` still had (and still has)
  no case for `(ResolvedSort::Function, ResolvedSort::Function)`, so accepting
  such a `Sub` at the inference level just moved the "how do I lower this"
  problem one level up without answering it. That is why the widening has to
  happen at the body (a real term), not at the function sort (an abstraction
  with no representation of "the same function, but wider").
- **Verification**: the full `merc_typecheck` test suite (`--lib --tests`,
  including `--include-ignored`) passes except the three pre-existing,
  independently-tracked failures already in this table (the two
  `stack_depth_probe` SIGABRTs and
  `function_update_inferred_sort_test::test_function_update_on_lambda_only_sort_gets_ground_equations`).
  The full `mcrl2 --test lowering_conformance` oracle-conformance suite
  passes 28/28, with one test updated rather than left alone (see below).

### The one remaining oracle divergence, and why it is not a bug

`test_round_trip_lambda_and_higher_order`'s `inc = lambda y: Nat. y + 1;`
lowers to a *structurally different but equally well-typed* term than the
real mCRL2 toolset produces: the body `y + 1` has no expected sort of its own
during its own inference, so both checkers pick the exact overload
`+: Nat # Pos -> Pos`. The toolset then pushes the lambda's declared range
(`Nat`) down into resolving `+`'s overload itself, ending up with
`+: Nat # Nat -> Nat` (retyping the literal `1` at `Nat`); merc instead types
the body at its own minimal sort (`Pos`) and widens the *result* once
(`Pos2Nat`/`@cNat`) via the fix above. This is the **same, already-documented
"Expected-sort propagation" divergence** as
`test_round_trip_positive_literal_argument`/
`test_round_trip_where_clause_with_positive_binding` a few tests down in the
same file (see `docs/developer/typechecking/data-specification.md`, "Known
divergences from the mCRL2 toolset") -- this session's fix just made a lambda
body the third place that same divergence is reachable from, rather than
introducing a new one. The test was converted from `assert_round_trips` to
`assert_sections_round_trip` excluding `Section::Equations`, matching the
existing precedent exactly (`apply`'s own equation, which has no arithmetic,
still conforms and is still checked).

## 2. `solve_join`'s fast path never backtracked

- **Location**: `crates/typecheck/src/inference/inference.rs`, `Solver::solve_join`.
- **Scenario**: when 2+ `Sub` constraints target the same free variable (e.g. every element of a `Set`/`Bag` literal), `merge_shared_subs` merges them into one `Join` constraint computing their least-upper-bound directly, replacing the old order-sensitive two-`Sub` encoding (see the `Join` struct's own doc comment, which already describes the general class of bug this replaced). The fast path in `solve_join` computed `lub` from the sources' mutual join, `unify`d `join.target` to it, and called `self.solve(index + 1)` -- but if that later solving failed, `solve_join` just returned `false` with **no fallback** to `solve_join_seq` (the per-source sequential search it already has, sitting right below, used for the "not materializable"/"can't join" cases). A `target` that needs to be *wider* than every source's own mutual join (because a *later* constraint -- e.g. an enclosing container -- requires it) had no way to be found.
- **Minimal repro**: `map v: Set(Bag(Nat)); eqn v = { {1: 1}, {2: 1} };` -- both elements' natural type is `FBag(Nat)`, joining trivially to `FBag(Nat)`; the declared `Set(Bag(Nat))` needs `Bag(Nat)`, one step wider than the join.
- **Fix**: snapshot before the fast-path `unify`+`solve`, and on failure `rollback_to` the snapshot and fall through to `solve_join_seq` (already written, already correct) instead of returning `false` directly.
- **Status**: fixed and shipped (this was already the case before this entry was last updated -- see git history). Verified against the full `merc_typecheck` suite and the `mcrl2` oracle-conformance suite.

## A third bug found once both of the above were fixed

Once findings 1 and 2 above no longer blocked the generators, running the
newly-enabled fuzz tests (`random_value_expression_type_checks_for_every_generated_sort`,
`random_pbes_with_data_specification_type_checks`) surfaced one more, purely
in the **generators**, not the type checker: `crates/syntax/src/random_pbes.rs`'s
`random_param_value` called `random_integer_data_expression` directly for a
`Pos`/`Nat`/`Int` predicate-variable parameter alike. That generator can
produce `0` or a `Subtract`, neither of which is a valid `Pos` value, and a
`Subtract` can go negative, which is not a valid `Nat` value either (`m - 2`
where `m: Nat` naturally has sort `Int`, not `Nat`) -- the exact same hazard
`random_value_expression`'s own `Pos`/`Nat` cases already guard against.
Fixed by having `random_param_value` delegate to `random_value_expression`
uniformly instead of special-casing the three numeric sorts.

## Practical impact

`crates/syntax/src/random_value_expression.rs` (a general random-value
generator for an arbitrary declared sort) and `random_pbes_with_data_specification`
now both generate `Function`-sorted values (lambdas) freely and type-check
cleanly; their fuzz tests (`random_value_expression_test.rs`,
`random_pbes_with_data_specification_test.rs`) run unconditionally (no
`#[ignore]`). Permanent regression coverage for finding 1 and its
container/list-nesting variants lives in `crates/typecheck/tests/inference_test.rs`
(`test_lambda_body_widens_a_finite_container_literal`,
`test_lambda_body_widens_a_primitive`,
`test_function_sorted_list_element_widens_its_lambda_body`).
