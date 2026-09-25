# Phase 2 adversarial review — `crates/typecheck` inference/resolution pipeline

**Scope:** `inference/inference.rs`, `inference/resolved_sort.rs`, `resolution/variable_resolution.rs`, `lowering/instantiate.rs`.

## Verdict

The change under review is not a diff but the standing state of these four files; one real, demonstrable correctness gap was found and confirmed with a failing test: `extend_system_with_inferred_sorts` never re-discovers a **function** sort that only ever appears as an inferred node sort (as opposed to a container sort, which it does re-discover). A specification can type-check and lower successfully while producing a `@func_update(...)` term that has no matching rewrite equation anywhere in the generated data specification, i.e. it is permanently stuck under rewriting. Everything else examined in these four files — the constraint solver's branch-and-bound pruning, the subtype/widening lattice, the scope-stack based variable resolution, and the template-instantiation worklist/dedup machinery — held up under fairly aggressive scrutiny (see "Checked and found correct").

## Findings

### 1. `instantiate.rs:522-533` (`extend_system_with_inferred_sorts`) — inferred function sorts never get ground `@func_update` equations — CONFIRMED

**Scenario.** `extend_system_with_inferred_sorts`'s job (per its own doc comment) is to catch a sort that Phase-3 inference produces but that never appears written down anywhere in the specification's own syntax — the standard example being the element sort of an enumeration literal like `[1, 2]`. It does this for container sorts:

```rust
// instantiate.rs:524-533
let mut container_worklist = Vec::new();
for typing in ctx.equation_typing.values().filter_map(|typing| typing.as_ref().ok()) {
    for &id in &typing.sorts {
        if matches!(ctx.sorts.get(id), ResolvedSort::Container { .. })
            && let Some(sort) = resolved_sort_to_syntax(ctx, spec, id)
        {
            container_worklist.push(sort);
        }
    }
}
```

The filter is `ResolvedSort::Container { .. }` only. `ResolvedSort::Function` is never collected here, and the later comparison-operator pass (`comparison_worklist`, unfiltered) only feeds `comparison_operator_equations`, never `standard_sort`/`function_update` — so nothing in this function ever generates `@func_update`/`@func_update_stable`/`@is_not_an_update`/`@if_always_else` equations for a function sort discovered purely through inference.

That gap is only safe if every function sort that can ever be the target of `[key -> value]` update syntax is *also* discoverable syntactically (a declared `map`/`cons` sort, or a lambda's own binder sort). It isn't: `system_check.rs`'s `DataExprKind::FunctionUpdate` arm (line 203) recurses into an arbitrary sub-expression with no restriction to a bare mapping name, and a `lambda`'s own *result* sort (e.g. `Pos -> Bool` for `lambda n: Pos. true`) is never written anywhere as a `SortExpression` — `collect_system_sorts_in_expr`'s `Lambda` arm collects only the *binder's* sort, never the lambda's own function-typed sort. So `expr[key -> value]` where `expr` is an anonymous lambda (or any other expression whose function sort is purely inferred, e.g. the join of two differently-typed lambda branches under an `if`) type-checks, lowers to a `@func_update(...)` application, and never gets a rewrite rule.

**Evidence.**

```
cargo nextest run -p merc_typecheck test_function_update_on_lambda_only_sort_gets_ground_equations
```
```
FAIL [0.705s] merc_typecheck::function_update_inferred_sort_test test_function_update_on_lambda_only_sort_gets_ground_equations
thread '...' panicked at crates/typecheck/tests/function_update_inferred_sort_test.rs:36:5:
expected a ground @func_update equation for the Pos -> Bool sort used by
`(lambda n: Pos. true)[1 -> false]`, but none was generated — the resulting
@func_update(...) term in the equation's right-hand side has no rewrite rule
and can never reduce
```

Dumping the lowered term confirms it is really produced and really orphaned:
```
func_update mappings: []
func_update equations: []
USE SITE: b = ==(@func_update(Binder(Lambda, [DataVarId(n, SortId(Pos))], OpIdNoIndex(true, SortId(Bool))), @c1, false), ...)
```
i.e. `DataSpecification::from_untyped` accepts the specification (no `InferenceError`), and `lower_data_specification()` embeds a live `@func_update(...)` call site with zero matching equations anywhere in the mappings/equations lists.

**Why the test would pass once fixed.** Broadening `extend_system_with_inferred_sorts`'s inference-driven worklist to also collect `ResolvedSort::Function` node sorts (routed through the same `standard_sort`/`merge_generated` machinery already used for the syntactic pass, since `resolved_sort_to_syntax` already produces a `FlattenedFunction` for any `ResolvedSort::Function`) would generate the missing `@func_update` block for `Pos -> Bool`, at which point `lowered.equations()` would contain an equation whose `lhs()` mentions `func_update`, and the assertion passes.

**Direction of a fix:** in the `container_worklist`/`container_seen` construction of `extend_system_with_inferred_sorts`, also collect `ResolvedSort::Function` node sorts and route them through `standard_sort` the same way declared function sorts are.

## Checked and found correct

- **Solver branch-and-bound pruning (`inference.rs`, `Solver::dominated`/`leaf`)**: verified by hand that the measure vector's length at any given constraint index is a fixed function of that index (`Sub`/`Lit` always contribute exactly 1 component, `Join` exactly `sources.len()`, `Disjunction`/`Comprehension` exactly 0), independent of which branch is taken — so `dominated()`'s prefix slice never panics and pruning a strictly-worse prefix can never discard a solution that would later tie or beat the incumbent (lexicographic comparison on equal-length vectors is decided by the first differing element, and appending more elements cannot undo that). Ambiguity detection (`Candidate::duplicate`) is correctly reset whenever a strictly better solution supersedes a tied one.
- **`merge_shared_subs` (`inference.rs`)**: grouping `Sub` constraints by their free-variable union-find root, replacing each group of ≥2 with one `Join` at the last member's position, and rebuilding the constraint list in original order — traced through and found sound; groups of size 1 are left untouched, and the two-`Sub`-vs-`Join` measure conventions were cross-checked to agree (`solve_join`'s fast path only ever takes the `head` component of `widening_distance` when `is_materializable` holds for every source, which is exactly when `interior` is guaranteed 0).
- **`SortInterner::partial_cmp`/`join`/`meet`/`widening_distance`/`is_materializable` (`resolved_sort.rs`)**: function-sort contravariance, container finiteness widening (`FSet <= Set`), and the materializability gate (which deliberately excludes function-sort coercions and element-changing container coercions, since neither is actually buildable by lowering) were checked against their extensive existing unit tests and by hand for a handful of additional cases (double-flip contravariance, arity mismatches). `widening_distance`'s `Function` branch is unreachable from production code (its only caller, `Solver::solve_join`'s fast path, is gated on `is_materializable`, which never returns true for a `Function` pair) — a minor dead-code observation, not a functional defect.
- **`variable_resolution.rs`'s scope stack (`NameStack`/`Scope`/`FixpointScope`)**: every binder gets a fresh `VarId`/`StateVarId` from a monotonically-increasing allocator, so `push`/`pop`-by-count is safe even with same-named shadowing (including within one `whr` block, `mu X. nu X. X`-style fixpoint shadowing, and nested lambda/quantifier/comprehension binders reusing a name) — no case found where the wrong binder is looked up or where a scope entry leaks past its construct's boundary. `resolve_in_act_frm` itself, despite being flagged for complexity, is a straightforward 7-arm match with no missing case found against `ActFrmKind`'s variants.
- **`instantiate.rs`'s worklist/dedup (`expand_sorts`, `merge_generated`, `seen` sets)**: `SortExpression`/`Spanned<T>` deliberately ignores `Span` in `Eq`/`Hash`/`Ord` (`crates/syntax/src/spanned.rs:11-14`), so structurally-identical sorts from different source positions do dedupe correctly; the single-argument `FlattenedFunction` → `Function` normalization in `collect_system_sorts` (needed so a re-scanned generated template and a user's own single-arg `map` declaration land in the same worklist representation) was traced and found consistent — this is the piece adjacent to Finding 1 that does work correctly for *syntactically declared* function sorts, which is exactly why the gap is specific to the inference-only path.
- **`instantiate_equation_block`/`specialize_template_typing`**: the `debug_assert_eq!` coverage check in `instantiate_system_equations` and the block-index/local-index bookkeeping were checked against `merge_generated`'s range bookkeeping; no off-by-one found.

## Tests added

- `/home/user/merc/crates/typecheck/tests/function_update_inferred_sort_test.rs` — `test_function_update_on_lambda_only_sort_gets_ground_equations`, demonstrating Finding 1. Run with:
  ```
  cargo nextest run -p merc_typecheck test_function_update_on_lambda_only_sort_gets_ground_equations
  ```
  or `cargo test -p merc_typecheck --test function_update_inferred_sort_test`.

## Notes on the environment (not findings against this review's scope)

- `cargo nextest run -p merc_typecheck` (full crate, parallel) also shows `modal::check::stack_depth_probe::deeply_nested_negation_does_not_overflow_the_stack` and `process::check::stack_depth_probe::deeply_nested_hide_does_not_overflow_the_stack` aborting with `SIGABRT` (stack overflow), and `git status` shows `crates/typecheck/src/process/check.rs` modified and `review/phase-2-typecheck-signature.md` present — both are inside the `process/`/`signature/` areas this task explicitly excludes ("a different agent owns those"), and are very likely mid-edit from a concurrent reviewer in this shared checkout, not from anything in this review's scope. Not investigated further.
- `lowering::mcrl2_lowering::tests::test_empty_list_sort_is_embedded` and `test_empty_set_lowers` show as `FAIL` in the full parallel run but pass cleanly in isolation (`cargo nextest run -p merc_typecheck -E 'test(test_empty_list_sort_is_embedded) + test(test_empty_set_lowers)'` → 2/2 passed) — this looks like parallel-run flakiness/resource contention in this sandbox rather than a real defect, and `mcrl2_lowering.rs` is outside this review's four assigned files regardless, so it was not pursued further.
