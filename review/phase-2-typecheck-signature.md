# Phase 2 review: `crates/typecheck` signature/checking pipeline

**Verdict:** The two worst-scored, untested files in the whole repository — `modal/check.rs` and `process/check.rs` — share the same real defect: their recursive-descent walk (`check_state_formula`/`collect_scope`/`check_reg_formula`/`check_action_formula` in `modal/check.rs`; `check_process_expr`/`collect_scope` in `process/check.rs`) has no depth guard, so a formula/process tree of ordinary (not pathological-by-mCRL2-standards) nesting depth crashes the whole process with `SIGABRT` instead of returning a typecheck error — confirmed even under the project's own CI stack budget (`RUST_MIN_STACK=104857600`, see the `check` skill). Everything else I looked at in these four files — overload/ambiguity resolution, `val(...)` real/bool fallback, action-table deduplication, the comm/rename sort-compatibility backtracking, `standard_sorts.rs`'s generated-equation text, `data_specification.rs`'s memoization — held up under direct reading, cross-checking against the grammar, and probing with constructed inputs; I found no wrong-result bug there.

## Findings

### 1. Unbounded recursion in `check_state_formula` (`modal/check.rs`) and `check_process_expr` (`process/check.rs`) crashes the process on deeply nested input — CONFIRMED

`crates/typecheck/src/modal/check.rs:173-247` (`check_state_formula`) and `crates/typecheck/src/process/check.rs:152-235` (`check_process_expr`), together with their sibling walks in the same two files (`collect_scope`, `collect_scope_regfrm`, `collect_scope_actfrm`, `check_reg_formula`, `check_action_formula` in `modal/check.rs`; `collect_scope` in `process/check.rs`), are ordinary recursive-descent functions with no depth limit and no manual stack (heap-allocated worklist). Each level of `!!!!...true` or `hide({}, hide({}, ...))` adds one native stack frame. Neither file bounds how deep that nesting may go before it is checked — parsing succeeds (or, for a large-enough input, the parser itself overflows first, a separate concern in `crates/syntax`, out of this review's scope), and `from_untyped_with`'s only defense against a pathological input is the OS stack.

I isolated each function's own recursion from the parser and from the resolution pass (`resolve_modal_variables`/`resolve_process_variables`, also out of scope) by building the tree directly with the syntax-tree constructors and calling the checking function itself, bypassing both:

- `crates/typecheck/src/modal/check.rs` (`stack_depth_probe` module, added): 100,000 nested `StateFrmKind::Unary { op: Negation, .. }` nodes, wrapping a bare `True`, checked via `check_state_formula` directly.
- `crates/typecheck/src/process/check.rs` (`stack_depth_probe` module, added): 100,000 nested `ProcessExprKind::Hide { actions: [], .. }` nodes, wrapping a bare `Delta`, checked via `check_process_expr` directly.

Both abort the process with `SIGABRT` (a stack overflow, not a panic that could be caught) — first reproduced with the default stack size, then reproduced again under `RUST_MIN_STACK=104857600` (100 MiB), the exact stack size the `check` skill says CI itself configures ("CI additionally runs the same suite in release mode with `RUST_MIN_STACK=104857600`"), specifically because other deep-recursion-prone code in this codebase needs it. Even that generous, CI-matching budget is not enough for a 100,000-deep tree in either file:

```
$ RUST_MIN_STACK=104857600 cargo nextest run -p merc_typecheck stack_depth_probe --no-fail-fast
SIGABRT [   0.961s] (1/2) merc_typecheck process::check::stack_depth_probe::deeply_nested_hide_does_not_overflow_the_stack
stderr ───
thread 'process::check::stack_depth_probe::deeply_nested_hide_does_not_overflow_the_stack' (32566) has overflowed its stack
fatal runtime error: stack overflow, aborting
(test aborted with signal 6: SIGABRT)
SIGABRT [   1.073s] (2/2) merc_typecheck modal::check::stack_depth_probe::deeply_nested_negation_does_not_overflow_the_stack
stderr ───
thread 'modal::check::stack_depth_probe::deeply_nested_negation_does_not_overflow_the_stack' (32567) has overflowed its stack
fatal runtime error: stack overflow, aborting
(test aborted with signal 6: SIGABRT)
Summary [   1.074s] 2 tests run: 0 passed, 2 failed, 866 skipped
```

Each test would pass once fixed: `assert!(result.is_ok())` on a 100,000-deep, otherwise-trivial tree is a formula/process any bounded-recursion (explicit worklist/stack) or depth-limited walk would accept in milliseconds; the assertion only fails today because the process aborts before it can even be evaluated.

Why this is a real, not merely theoretical, risk for exactly these two files: both are explicitly the ones an LSP-style consumer drives interactively (`TypingInfo` for hover/go-to-definition is threaded through every node these functions visit), and both are reachable from data the *type checker* controls the acceptance of, not just from hand-written source — a machine-generated `.mcf` property (model checkers routinely emit large disjunctions/conjunctions and deep fixpoint nesting) or a large generated process term (flattened `hide`/`block`/`rename` chains from a code generator) are the realistic inputs that would hit this, not adversarial fuzzing. A crash aborts the whole process (not just that one request), which is a much worse failure mode for an LSP/batch tool than a clean `TooDeeplyNested` error.

Note on scope: I confirmed via a direct AST-construction probe that `UntypedStateFrmSpec::parse` (the parser, `crates/syntax`, explicitly out of this review's scope) *also* overflows on a 200,000-deep textual input, before ever reaching `modal/check.rs`. That is a separate defect in a different crate and I am not claiming it here. The point of building the tree directly (skipping `parse` and the resolution pass) in the two tests above is precisely to show that `check_state_formula`/`check_process_expr` themselves — the files this review covers — have the identical unbounded-recursion defect independent of the parser's own limit, so fixing only the parser's recursion would not fix these two files.

Direction of fix: convert the checking walk to an explicit worklist/stack on the heap (as e.g. `resolve_single_candidate`'s flat iteration already does for overload candidates), or wrap the two public entry points (`check_modal_specification`, `check_process_specification`) in a bounded-stack thread the way some compilers spawn checking on a thread with an explicit, generous `stack_size` and turn a stack-overflow-adjacent depth into a reported error before it happens (e.g. a cheap upfront depth count with a `TooDeeplyNested` error variant) — either avoids the current all-or-nothing "however big the OS stack happens to be" behavior.

## Checked and found correct

- **Overload/ambiguity resolution** (`resolve_single_candidate`, `check_action` and `check_action_or_process`/`check_instantiation`, both files): each candidate is checked into its own scratch `TypingInfo`, merged into the caller's only on the unique success — confirmed by reading and by probing an actually-ambiguous case (`act a: Nat; act a: Pos; form <a(1)>true;` under `FormulaType::Bool`) that correctly reports `ModalError::AmbiguousAction { count: 2, .. }` rather than silently picking one or merging both.
- **`check_val_expr`'s `Real`-then-`Bool` fallback** (`modal/check.rs:250-285`) for state-level `val(...)`: matches every case in `modal_specification_test.rs` and my own probes (a `Bool`-only value under `Real`, an ambiguous-looking-but-not-actually-ambiguous two-overload case, quantifiers/fixpoints referencing scoped variables inside `val(...)`).
- **Regular- and action-formula operators not exercised by the existing test file** (`RegFrmKind::Iteration`/`Plus`/`Sequence`/`Choice`, i.e. `<a*>`, `<a+>`, `<a.b>`, `<a+b>`; `ActFrmKind::Quantifier`, i.e. `<exists x: Nat . a(x)>true`): probed directly, all check correctly with no crash and no wrong acceptance/rejection.
- **`combined_sort_matches`'s unchecked `from_indices[0]` index** (`process/check.rs:636`): safe. `check_rename_sorts` always passes a length-1 slice; `check_comm_sorts`/`try_from_overloads` only ever reaches it through `comm.from.actions`, which the grammar (`crates/syntax/mcrl2_grammar.pest:276`, `CommExpr = { Id ~ "|" ~ MultActId ~ "->" ~ Id }`, `MultActId` itself requiring ≥1 `Id`) guarantees has at least 2 entries — `from_indices` is never empty at that call site.
- **`ActionTable::build`'s deduplication of an exact restatement** (`checking.rs:145-179`, e.g. `act a: Nat; a: Nat;`): collapses to one candidate rather than leaving two identical, spuriously-ambiguous ones — confirmed by the existing test and does not regress `AmbiguousAction`/`AmbiguousActionOrProcess` reporting for genuinely different overloads.
- **The flat, non-popping `scope: Vec<(VarId, ResolvedSortId, Span)>` both files build via `collect_scope`** (every binder anywhere in the body/formula, appended but never removed as the walk exits a binder's subtree): not a shadowing/scoping bug. `Scope`'s own doc comment (`checking.rs:24-27`) states the design: it is keyed by each declaration's unique `VarId`, assigned by the (out-of-scope) resolution pass *before* checking runs, and an out-of-scope name reference fails to resolve to any `VarId` at that earlier stage, independent of this Vec's contents — confirmed against `test_bound_variable_is_out_of_scope_outside_it`/`test_sum_bound_variable_is_out_of_scope_outside_the_sum` and by tracing how `check_expression_against` looks entries up by `VarId`, not by position or name.
- **`check_comm_sorts`/`try_from_overloads`/`check_rename_sorts`'s backtracking** (`process/check.rs:497-611`) accepts the *first* satisfying overload combination without checking whether a second, different one would also satisfy it (no `Ambiguous*` error for `comm`/`rename`, unlike `check_action`/`check_action_or_process`). This is not flagged as a defect: unlike those two, `check_comm_sorts`/`check_rename_sorts` carry no `TypingInfo`/`ResolvedName` output for a hidden alternative combination to misattribute — there is nothing here a second candidate could get "wrong" the way an unreported ambiguous overload pick would be for hover/go-to-definition.
- **`standard_sorts.rs`'s generated-equation text generators** (`function_update_text`'s `less`/`structured_sort_equations`'s `lexicographic`, both doing an unchecked `arity - 1`/`a.len() - 1`): the zero-arity case that would underflow is syntactically unreachable — `ConstrDeclList = { ConstrDecl ~ ("|" ~ ConstrDecl)* }` (`mcrl2_grammar.pest:120`) requires ≥1 constructor for a `struct`, and a function sort's domain is likewise always ≥1 sort by construction (`flatten_function_domain_rec`) — so every caller already guards the empty case (`if constructor.args.is_empty() { "false" } else { lexicographic(...) }`) before it would matter.
- **`data_specification.rs`'s `Arc`-based memoization** (`typing_info()`, `equation_typing_info()`): already covered by the file's own 3 pointer-identity tests; I did not find a path that reads a stale cache entry after a mutation, and the two struct-sort/function-update generators it drives (`typecheck_templates`, `typecheck_function_update_template`) correctly key their `template_typings` cache by `TemplateId` including the generic function-update's own arity.

## Tests added

Both are minimal, hand-constructed-AST regression tests placed as `#[cfg(test)] mod stack_depth_probe` inside the file whose own function they isolate, per the working files' own convention of unit-testing private helpers in-module (`modal/check.rs` and `process/check.rs` had no test file at all, and both defects are in `pub(super)`/private functions not reachable from the integration `tests/` directory without going through the parser, which would confound the isolation this test needs):

- `crates/typecheck/src/modal/check.rs` — `modal::check::stack_depth_probe::deeply_nested_negation_does_not_overflow_the_stack`
- `crates/typecheck/src/process/check.rs` — `process::check::stack_depth_probe::deeply_nested_hide_does_not_overflow_the_stack`

Run with (each test's own process is aborted by the stack overflow; `cargo nextest run` isolates that to just that one test rather than taking the whole binary down, which plain `cargo test`/`cargo test --lib` does not — confirmed both ways):

```
cargo nextest run -p merc_typecheck stack_depth_probe --no-fail-fast
# or, matching CI's own stack budget exactly:
RUST_MIN_STACK=104857600 cargo nextest run -p merc_typecheck stack_depth_probe --no-fail-fast
```

Both report `SIGABRT`/"has overflowed its stack" on the current tree; both would report `ok` once either function's recursion is bounded (explicit worklist, or a depth check that returns a typecheck error instead of recursing further).
