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

**Update: this is now fixed, for all eight node types at once — see "The recursive-`Drop` fix"
below.**

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

## Every hand-written checker and variable resolution are now migrated onto `Traverse`

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
this migration just fixed) was no longer masking it by overflowing first. At the time, the test
worked around this the same way `crates/syntax/src/traverse.rs`'s own `stack_depth_probe` tests did,
by forgetting the formula rather than letting it drop; that workaround is gone now that the
recursive-`Drop` bug itself is fixed too — see "The recursive-`Drop` fix" below.

`process::check::collect_scope`/`check_process_expr` and `pres::check::collect_scope`/
`check_pres_expr` are migrated too, each onto a single `Traverse::try_visit` walk — simpler than
`modal::check` needed, since neither has a fixpoint-variable-style scoped stack (`visit_scoped`) or
a `RegFrm`/`ActFrm`-style node-type crossing (`visit_mixed`) to deal with: every arm either does its
own per-node work against a field that isn't itself a same-type child (a `DataExpr`
weight/condition/time/constant, an `ActionName` list, a `PropVarInst`) or is empty, and the
traversal's own descent (`Traverse::push_children` for `ProcessExpr`/`PresExpr`) reaches every
operand in the same order the original recursive calls did. `process::check::stack_depth_probe`
now passes (it already existed and used to SIGABRT); `pres::check` had no `stack_depth_probe` test
at all before this — the original finding's own framing called this checker's exposure "structurally
the same pattern (untested)" — so one was added alongside the migration rather than left
retroactively unverified.

`resolution::variable_resolution::resolve_in_state_frm` (and its `resolve_in_act_frm` cousin) — the
fourth and last of the originally-flagged SIGABRT instances — is migrated too, closing the gap this
section used to describe as unattempted. It needed a genuinely different addition from the three
checkers above, not just the same mechanical swap, because unlike a checker it *rewrites* the tree
(`Id` → `Resolved`) rather than only reading it: `visit_scoped` (the checkers' scoping mechanism) is
read-only (`&self`), and there was no mutating counterpart. `Traverse::apply_subtree_scoped`/
`apply_scoped` (`crates/syntax/src/traverse.rs`) are that counterpart, and their own doc comments
cover the one real design wrinkle this needed: `exit` cannot be handed the node it is un-scoping the
way `visit_subtree_scoped`'s can, since holding a reference to it across the window where its
children are then reached through a *fresh* `&mut` borrow of that same node is exactly the aliasing
this trait's other mutating methods avoid by construction, and there is no `unsafe` way around it in
a `#![forbid(unsafe_code)]` crate. `resolve_in_state_frm`'s local `Undo` enum, pushed onto `state`
once per node by `enter` and popped by `exit`, is the pattern this forces on a caller whose `exit`
needs to know what it's undoing.

The other piece this section used to flag — two independent scoped stacks needed at two different
node types in the same walk (`StateFrm`'s own `scope`/`state_vars`, and separately `ActFrm`'s own
`scope` around its *own* `Quantifier`, reached by crossing through `RegFrm` from inside a
`Modality`) — turned out not to need two *nested traversal calls* wired together after all: `scope`
is a plain `&mut Scope`/borrowed field either way, so `resolve_in_act_frm`'s own `apply_scoped` call
simply borrows the *same* `Scope` the outer `resolve_in_state_frm` walk owns (passed down through
the untouched, plain-recursive `resolve_in_reg_frm`, exactly the way `check_reg_formula` already
threads `scope` through unchanged) rather than needing a separate one — two independent walks
sharing one piece of mutable state through an ordinary borrow, not one gap needing a new mechanism.
`resolve_in_reg_frm` itself stays a small hand-written recursive dispatch: `RegFrm` carries no
binder and its own nesting (`.`/`+`/`*` chains within a single modality) was never part of the
original SIGABRT finding, unlike `StateFrm`'s and `ActFrm`'s own potentially-pathological nesting.

Two new regression tests (`test_action_formula_quantifier_resolves_to_its_own_binder_and_can_see_an_
outer_state_formula_binder`, `test_action_formula_quantifier_shadows_an_outer_state_formula_binder_
of_the_same_name`) cover what no existing test did: an `ActFrm`'s own `Quantifier` binder correctly
seeing an enclosing `StateFrm` binder, and correctly shadowing one of the same name. A
`stack_depth_probe` test (100,000-deep, matching the other three checkers') is new too — this was
the one of the four original instances that never had one — and, like the others', proves the
separate recursive-`Drop` bug is fixed at the same time by letting its tree drop normally. The full
`merc_syntax`/`merc_typecheck` suites (1395/1395) and, since this pass sits ahead of every modal
formula real-world `.mcf` files exercise, `tools/mcrl2`'s `merc_pbes` test suite (137/137, including
several real academic-example `.mcf` files with quantifiers and fixpoints run through the full
LTS→PBES pipeline) all pass.

## The recursive-`Drop` fix

The [second bug](#2-a-second-different-bug-sits-directly-underneath-the-first-recursive-drop) this
document found — the compiler's own default, field-by-field drop glue recursing through a
`Box`-chained AST one node at a time on the way down, independent of and not touched by anything
above — is now fixed, for all eight `Traverse` node types at once, in
`crates/syntax/src/spanned.rs`.

**Why this couldn't just be `impl Drop for StateFrm { ... }` eight times.** `StateFrm` and the
other seven are type aliases for `Spanned<XKind>`, and Rust's `E0366` ("`Drop` impls cannot be
specialized") flatly refuses a `Drop` impl for one specific instantiation of a generic struct —
only `impl<T> Drop for Spanned<T>`, covering every `T` uniformly, is legal. Making that one impl's
*behavior* still depend on which `T` it's instantiated with needed a trait: `TakeRecursiveChildren`,
with a default no-op method, implemented for real (iterative-detach) logic only by the eight
`*Kind` enums that actually nest through their own type, and trivially (the default) by every other
`T` `Spanned` is used with (`String`, `AssignmentData`, `EqnSpecData`, `PropVarInstData` — checked
by grep across the whole workspace to confirm the list is closed and none live outside
`crates/syntax`). `Spanned<T>`'s own struct definition then had to grow the bound
(`Spanned<T: TakeRecursiveChildren>`, another consequence of `E0367`: a conditional `Drop` impl's
bound must match a bound already on the struct, so `impl<T: TakeRecursiveChildren> Drop for
Spanned<T>` isn't legal on an unbounded struct either) — the one place this ripples outward, since
now *anything* named as `Spanned<T>` anywhere needs that bound satisfied.

**How the iterative detach itself works, reusing `Traverse`'s own machinery.** Each `*Kind` enum's
`TakeRecursiveChildren` impl is generated by the same `define_traversal!` macro invocation that
already generates its `Traverse` impl in `crates/syntax/src/traverse.rs`, reusing the exact same
`$child`/`$mut_child` match arms `push_children_mut` uses — the `recurse` closure just does
something different with each child (`stack.push(std::mem::take(child).into_node())`, replacing it
in place with a cheap `Default` value and moving the real one onto a `Vec`, instead of collecting a
reference) — so there is one source of truth per node type for "what are this node's same-type
children", not two to keep in sync. `Spanned<T>::drop` then drains that `Vec` in a loop, calling
`take_recursive_children` again on each popped value *before* it's allowed to drop implicitly at
the end of the loop body — by the time any single value's fields are actually freed, none of them
still holds a deep subtree, only ever the cheap placeholder value it was just replaced with.

Getting a cheap, non-recursive placeholder value out of `T::default()` needed each of the eight
`*Kind` enums to implement `Default` — six via `#[derive(Default)]` with `#[default]` on an
existing unit-ish leaf variant (`StateFrmKind::True`, `ActFrmKind::True`, `PbesExprKind::True`,
`PresExprKind::True`, `ProcessExprKind::Delta`, `DataExprKind::EmptyList`), and two
(`RegFrmKind`, `SortExpressionKind`) by hand: `#[default]` only accepts a unit variant, and neither
enum has one (every variant carries at least one field), so `RegFrmKind::default()` returns
`Action(ActFrm::default())` and `SortExpressionKind::default()` returns
`Reference(String::default())`, written out manually instead.

**Re-entrant `Drop::drop` is expected, and still bounded.** Nothing stops the natural drop of a
popped, already-detached value from calling `<Spanned<T> as Drop>::drop` on it again (Rust has no
way to skip a type's own `Drop` when an owned value goes out of scope) — but by then
`take_recursive_children` has already been called on it this same iteration, so that re-entrant
call finds only the cheap placeholder, extracts nothing new, and returns in O(1). The *number* of
these re-entrant calls is proportional to the tree's size (harmless, since each is O(1) and they
don't nest inside each other beyond a small constant depth), never to its *depth* — which is the
property that actually matters for the native call stack, and the two million-deep
`stack_depth_probe` tests in `crates/syntax/src/traverse.rs` (`visit`/`apply_mut`) now prove it by
letting their formula actually drop instead of leaking it with `mem::forget`, as do the scoped/mixed
probes added alongside them and `modal::check`'s own 100,000-deep probe.

**Why not `unsafe`.** `crates/syntax` is `#![forbid(unsafe_code)]`, which rules out the more
obvious `ManuallyDrop` + `ptr::read` idiom for taking a field out of a `Drop` type. `Default` +
`std::mem::take` gives the same thing safely, at the cost of needing `Default` on every `T` this
touches.

**The other consequence: `Spanned<T>::into_node()`/`into_parts()`.** Implementing `Drop` also
means Rust no longer allows `let Spanned { node, span } = value;`-style destructuring of an *owned*
`Spanned<T>` anywhere in the codebase (moving only some of a `Drop` type's fields out is rejected
outright) — about a dozen call sites across `crates/syntax` and `crates/typecheck` did exactly
this (mostly `identifier.node` after parsing an `Id`, or matching `expr.node` by value to
destructure it). `Spanned::into_node`/`into_parts` (also built on `mem::take`, same reasoning as
above) replace each of them; `crates/typecheck/src/process/disambiguation.rs`'s `flatten_at_chain`
is the one non-mechanical case, since its fallback arm needed to reconstruct the original
`DataExpr` from the split `(node, span)` pair rather than just discarding the span.
