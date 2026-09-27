# Unbounded recursion → stack overflow (SIGABRT): `Traverse` itself is now fixed; the checkers that don't use it yet are not

The original finding was narrow and precise: `crates/typecheck/src/modal/check.rs`'s
`check_state_formula`/`collect_scope` and `crates/typecheck/src/process/check.rs`'s
`check_process_expr` are plain recursive-descent walks over their own AST with no depth
bound, and structurally the same pattern exists (untested) in `crates/typecheck/src/pres/check.rs`'s
`check_pres_expr`. Each has a `stack_depth_probe` test (`modal::check`, `process::check`)
that builds a 100,000-deep tree *directly* (bypassing the parser and any resolution pass,
specifically to isolate the recursive-descent walk's own contribution) and asserts the
walk still succeeds. Both currently SIGABRT.

Asked to look into fixing this, a prototype (a `run_with_deep_stack` helper in
`crates/typecheck/src/checking.rs`, running the check on a `std::thread::scope` worker
with a large stack, wired into `ModalSpecification::from_untyped_with`) surfaced two things
that widen the finding considerably. Both are documented here rather than shipped, per the
explicit choice to stop and record rather than keep expanding scope.

## 1. The finding's own "isolated" framing undersells the real pipeline

`resolve_modal_variables` (`crates/typecheck/src/resolution/variable_resolution.rs`'s
`resolve_in_state_frm`) runs *before* `check_modal_specification` in
`ModalSpecification::from_untyped_with`, and is exactly the same shape: a plain
self-recursive walk over the same `StateFrm` tree, no depth bound. A prototype that fed a
100,000-deep formula through the real public entry point (`ModalSpecification::from_untyped`,
not the isolated `check_state_formula` call the existing test makes) overflowed *before*
reaching the checking phase at all -- confirmed by the aborting thread being the original,
unwrapped test thread rather than the wrapped worker. Wrapping only `check_modal_specification`
(as the finding's own scope implies) leaves the real, end-to-end pipeline unprotected;
`resolve_modal_variables` needs the same treatment, and almost certainly so does the parser
itself for deeply nested input (the original test's own comment already calls the parser "a
separate, out-of-scope concern" for the same reason -- it also isn't bounded).

Once `resolve_modal_variables` and `check_modal_specification` were run inside the same wrapped
worker thread, a 100 MiB stack (matching what CI's `RUST_MIN_STACK=104857600` already gives
release-mode runs, so the same order of magnitude the project already treats as sufficient
for deep-recursion cases) was not enough for the checking phase specifically. A synthetic
benchmark recursing to the same depth with a deliberately small (256-byte) frame succeeded
comfortably at 100 MiB, which means the real `check_state_formula`/`collect_scope`/
`resolve_in_state_frm` match arms have meaningfully larger frames than that -- unsurprising in
an unoptimized debug build (no inlining, no dead-branch elimination across a large `match`),
but it means "give it CI's stack size" isn't the right number; 1 GiB was needed before the
checking-phase recursion itself stopped overflowing at 100,000 deep in a debug build.

## 2. A second, different bug sits directly underneath the first: recursive `Drop`

Even with the checking phase alone made to succeed (via the 1 GiB worker), the *same test
process* then aborted anyway -- this time back on the original, unwrapped thread, and with no
error printed by the checked call itself (it had already returned `Ok`). The cause: dropping
a 100,000-deep chain of `Box<StateFrm>` (built directly by the test, then held in the returned
`ModalSpecification`/local variable until the test function returns) recurses through the
default, compiler-generated `Drop` glue one `Box` at a time, at the same unbounded depth --
this is the well-known Rust footgun where a deeply nested `Box`-chained recursive data
structure overflows the stack on drop, completely independent of whatever algorithm walked it
earlier. A stack-size wrapper around the *checking* call cannot fix this: the value is handed
back across the thread boundary (via `.join()`) and dropped on the *caller's* thread, whose
stack size this crate does not control and — for a real caller (a CLI tool's `main`, a test
harness thread, an embedder) — has no way to.

This is a distinct bug class from the original finding (destructor recursion, not checking
recursion), and it is not specific to `StateFrm`: any sufficiently deep `Box`-chained AST in
this codebase (a `DataExpr`, a `ProcessExpr`, a `PresExpr`, ...) has the same default `Drop`
glue and the same exposure. Fixing it for real means either an explicit iterative `Drop` impl
(pop children into a work-list rather than recursing) on each such type, or restructuring the
representation (e.g. an arena/index-based AST instead of `Box` chains) -- neither of which is
a small change, and neither is scoped to modal formulas specifically.

## Where this left the original finding, before the real fix

- A stack-size mitigation at the public entry point was a real, verified option (it did make the
  checking phase itself succeed at the depth the existing tests probe), but needed tuning per
  checker/build profile, didn't make the existing (deliberately isolated) `stack_depth_probe`
  tests pass as written, and only raised the depth at which the bug recurs rather than removing
  the class of bug. It was not shipped.
- A *second* finding (recursive-`Drop` stack overflow on deeply `Box`-chained ASTs, discovered
  via the above prototype, not previously documented) was confirmed and is still unfixed — see
  above. It is broader than modal formulas and not addressed by anything below.

## The real fix: `Traverse`'s own engine, made genuinely stack-safe

Directed to the actual right answer instead: `crates/syntax/src/traverse.rs`'s `Traverse` trait
is the one general recursive-tree-walk abstraction already shared by every AST in this codebase
(`SortExpression`, `DataExpr`, `ProcessExpr`, `StateFrm`, `RegFrm`, `ActFrm`, `PbesExpr`,
`PresExpr`) — and its own doc comment already named the fix: "an explicit worklist would replace
the call stack." That's now done for the pre-order half of the trait (`visit_subtree`/
`apply_subtree`, and everything built on them: `visit`, `try_visit`, `visit_with`,
`visit_children`, `apply`, `apply_mut`, `apply_with`, `apply_children`). The trait's two
"required, node-shape-aware" methods changed from `visit_children`/`apply_children` (which used
to recurse straight back into `visit_subtree`/`apply_subtree` per child) to `push_children`/
`push_children_mut`, which only *enqueue* a node's direct children onto an explicit,
heap-allocated `Vec<(&Node, Context)>`/`Vec<(&mut Node, Context)>` stack; a single shared `drive`
loop (for the mutable case) and an inlined equivalent (for the shared-reference case, kept
separate only because Rust has no clean way to be generic over `&`/`&mut` here) then pops from
that stack in a loop, calls the callback, and pushes the result's children — reproducing the
exact left-to-right, depth-first, pre-order visitation order native recursion gives, just backed
by the heap instead of the call stack. Every one of the 8 `define_traversal!` node types picked
this up automatically, in one place, with zero call-site changes anywhere in the codebase — the
public API (`visit`, `apply_mut`, etc.) is unchanged in signature and behavior; only how it's
implemented underneath changed. All 18 pre-existing unit tests (context threading, `Prune`,
`Break`, `Replace`-doesn't-descend, the `Box`-behind-`FunctionUpdate` case, ...) pass unmodified,
and two new ones (`traverse::stack_depth_probe`) build a plain, directly-constructed million-deep
`StateFrm` (ten times the depth that SIGABRTs the hand-written checkers below) and confirm both
`visit` and `apply_mut` complete it without overflowing.

**`transform_children`/`try_transform`/`transform` (bottom-up rewriting) is not fixed and remains
recursive.** This is a harder, different problem, not just more of the same trick: a bottom-up
rewrite needs a node's children fully processed *before* the node itself is touched again, which
for an owned, `Box`/`Vec`-based tree (as opposed to an arena addressed by index) means holding a
live mutable borrow of the already-processed children at the same time as the borrow needed to
reach the parent again — the borrow checker rejects this, and it's why the top-down half above
has no such conflict (a node's borrow is dropped before its children are ever reached, since
nothing needs to come back to the parent afterward). Making bottom-up rewriting iterative too
needs a genuinely different representation (an arena, or owned nodes moved in and out of a
worklist behind a placeholder value), not this same worklist trick. The three current
`.transform`/`.try_transform` call sites (`crates/typecheck/src/ir/desugar.rs`,
`crates/typecheck/src/ir/lower.rs`, `crates/typecheck/src/resolution/name_resolution.rs`) are
therefore still not stack-safe for a pathologically deep tree, and are the next thing to either
fix (harder) or explicitly accept as a narrower, separately-documented residual gap.

## The two capability gaps that were blocking a migration are now closed in `Traverse` itself

The previous version of this document stopped at identifying two obstacles to migrating the
hand-written checkers onto `Traverse` and left the API addition an act. Both are now implemented
in `crates/syntax/src/traverse.rs`, independently of and without touching the checkers themselves:

- **The enter/exit hook.** `check_state_formula` threads genuinely *scoped* mutable state
  (`state_vars`, pushed when a `mu`/`nu` binder is entered and popped once its body is done being
  checked), which needs a callback on both sides of a node's children, not just the one pre-order
  callback `Visit`/`Step::Into(context)` offered. `Traverse::visit_subtree_scoped`/`visit_scoped`
  add that second callback (`exit`), called right after a node's whole subtree finishes — using
  the same explicit-stack technique as the rest of the trait (an `Enter`/`Exit` frame per node
  rather than a plain node), so it's stack-safe by the same argument as the fix above, and unwinds
  every still-open `exit` (innermost first) if the walk stops early on a `Break` or an error, so a
  caller's scoped state is never left corrupted. The one real wrinkle: `enter` and `exit` are two
  separate `FnMut`s, and two closures cannot both capture the *same* variable mutably at once — so
  the scoped state itself is threaded as an explicit `&mut S` parameter (`state`) rather than
  captured by either closure, exactly like `context` is threaded, just mutably and without the
  `Copy` requirement. `traverse::tests::test_visit_scoped_pushes_and_pops_fixpoint_variables_like_a_real_checker`
  reproduces `check_state_formula`'s exact `state_vars` push/pop pattern end to end (nested `mu`
  bindings, a sibling that must not see an already-exited inner binder's variable) and
  `test_visit_scoped_unwinds_open_scopes_on_break` covers the early-exit unwind specifically.
- **Crossing node types.** `collect_scope`/`collect_scope_regfrm`/`collect_scope_actfrm` are three
  hand-written functions whose only job is to walk a `StateFrm`, across into the `RegFrm` of a
  `Modality` and across again into the `ActFrm` of an `Action` — a crossing every other method on
  `Traverse` deliberately refuses to make on its own (see the trait's own doc comment on why).
  `Traverse::visit_mixed`/`try_visit_mixed`, backed by a new `MixedNode` enum (one variant per
  traversable type) and `push_mixed_children` (a new, empty-by-default trait method that a node
  type overrides to name its foreign-type children — currently only `StateFrm`, into `RegFrm`, and
  `RegFrm`, into `ActFrm`), do that crossing in one walk instead of three hand-wired functions.
  `traverse::tests::test_visit_mixed_crosses_from_state_formula_into_its_regular_and_action_formulas`
  walks `[a . b*]X` and confirms the visit order crosses from the `StateFrm` root into the whole
  `RegFrm`/`ActFrm` subtree of the modality and back, pre-order, left to right.

Both additions get the same million-deep-tree stack-safety probe as the original fix
(`stack_depth_probe::visit_scoped_a_million_deep_negation_does_not_overflow_the_stack`,
`visit_mixed_a_million_deep_negation_does_not_overflow_the_stack`), since each is its own,
independent explicit-stack loop rather than a thin wrapper over `visit_subtree`.

`visit_mixed` is deliberately narrower than "reach every node of every type": `push_mixed_children`
is only overridden for the one crossing `collect_scope`'s trio needed
(`StateFrm` → `RegFrm` → `ActFrm`). Left unwired, and still exactly the kind of gap this document
exists to name rather than quietly leave undocumented: the `DataExpr` reachable from
`ActFrm::DataExprVal`/`MultAct`/`At`'s `operand`, from `StateFrm::Id`'s arguments and
`DataValExpr`/`DataValExprLeftMult`/`DataValExprRightMult`, and from the equivalent spots in
`PbesExpr`/`PresExpr`/`ProcessExpr`; and the `SortExpression` bound by an `IdDecl`/quantifier
variable anywhere. None of these block `collect_scope`'s own migration (it doesn't currently
recurse into any of them either), so wiring them was left for whichever future migration actually
needs a given one, rather than guessed at speculatively here.

## `modal::check` is now migrated; `process::check`/`pres::check` and variable resolution are not

`modal::check::collect_scope`/`collect_scope_regfrm`/`collect_scope_actfrm` are now one function
(`collect_scope`) built on `try_visit_mixed`, and `check_state_formula`/`check_fixed_point` are now
`check_state_formula` (fallible state now local to it, no longer threaded in from
`check_modal_specification`) built on `visit_scoped`, with `check_reg_formula`/`check_action_formula`
similarly collapsed into one `try_visit_mixed`-based `check_reg_formula`. Each hand-written
`_regfrm`/`_actfrm` recursive cousin is gone; only the per-node work remains, at the same left-to-right
checking order the original recursive-descent version had (the one place order needed active
preserving: `check_state_formula`'s `Modality` arm calls `check_reg_formula` on the modality's regular
formula *inside* `enter`, before returning `Step::Into`, so it still fully finishes before
`visit_scoped`'s own descent reaches the modality's `StateFrm` operand — mirroring the original's
"check the regular formula, then recurse into the operand" order exactly).

`modal::check::stack_depth_probe::deeply_nested_negation_does_not_overflow_the_stack` now passes.
Getting it there also surfaced a second thing worth recording precisely: after the migration it
*still* SIGABRT'd once, on the same test, not from `check_state_formula`'s own walk (confirmed by
`std::mem::forget`-ing the test's formula immediately after a successful `check_state_formula`
call: the SIGABRT moved to the end of the test regardless) — it was the recursive-`Drop` bug from
this document's own [earlier section](#2-a-second-different-bug-sits-directly-underneath-the-first-recursive-drop)
firing on the same 100,000-deep `Box<StateFrm>` chain once the *first* bug (the traversal recursion
this migration just fixed) was no longer masking it by overflowing first. The test now forgets its
formula for the same reason `crates/syntax/src/traverse.rs`'s own `stack_depth_probe` tests do,
which makes it, retroactively, a second confirmation of that bug rather than only the first one.

What is still not migrated: `resolution::variable_resolution::resolve_in_state_frm`,
`process::check::check_process_expr`, and `pres::check::check_pres_expr` are still hand-written
recursive-descent matches, not `Traverse` callbacks. `check_process_expr`/`check_pres_expr` don't
have `modal::check`'s node-type-crossing shape (no `visit_mixed` need) but do thread several more
parameters (`data`, `tables`, `scope`, `typing`, ...) than a single `context`/`state` slot has
obvious room for, same as `modal::check` did — migrating them is real, separate, follow-up work,
not fundamentally blocked by anything left in `Traverse`, just not done. The `stack_depth_probe`
test in `process::check` still SIGABRTs, unchanged.
