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

## What is still not fixed: the hand-written checkers don't use `Traverse` yet

`Traverse` being stack-safe only helps code that actually uses it. The original finding's own
functions — `modal::check::check_state_formula`/`collect_scope`,
`resolution::variable_resolution::resolve_in_state_frm`, `process::check::check_process_expr`,
and `pres::check::check_pres_expr` — are hand-written recursive-descent matches, not `Traverse`
callbacks, and migrating them is real, separate, follow-up work, not a mechanical consequence of
the engine fix above. The two `stack_depth_probe` tests in `modal::check`/`process::check` still
SIGABRT unchanged. The obstacle worth flagging before starting that migration: these checkers
thread genuinely *scoped* mutable state (`check_state_formula`'s `state_vars`, pushed when a
`mu`/`nu` binder is entered and popped when its body is done being checked), which needs an
enter-*and*-exit hook around each node's children; `Traverse`'s current context-threading model
(`Visit`'s `Step::Into(context)`) only offers a single pre-order visit per node with no
corresponding "children are done, un-scope now" callback. Reusing `Traverse` for these will need
either a real API addition (a post-order/exit hook alongside the existing pre-order one) or
re-expressing the scoped state as an immutable, `Copy` value threaded purely through `context`
(the pattern `merc_utilities::traversal::Visit`'s own doc comment already recommends — "state
that grows along a path ... belongs in the callback itself (push on entry, truncate on exit) or
in a `Copy` slice" — but "push on entry, truncate on exit" itself presumes the enter/exit hook
that doesn't exist yet).
