# Unbounded recursion → stack overflow (SIGABRT), and what fixing it actually involves

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

## Where this leaves the original finding

- The original finding (3, now-confirmed-4, checking/resolution-phase recursive walks) is
  unfixed. A stack-size mitigation at the public entry point is a real, verified option (it
  does make the checking phase itself succeed at the depth the existing tests probe), but:
  - it needs tuning per checker/build profile (100 MiB was insufficient in debug; 1 GiB
    sufficed for this one case, but that number was not derived rigorously, just found to
    work once, and was only validated for `modal`, not `process`/`pres`);
  - the existing `stack_depth_probe` tests call the raw recursive function directly and would
    need to be rewritten to go through a wrapped public entry point instead, changing what
    they isolate;
  - it does not remove the *class* of bug, only raises the depth at which it recurs (still a
    finite, real ceiling, just a much bigger one) — a true fix is the bounded/iterative
    rewrite the tests' own doc comments gesture at ("a bounded-recursion walk would handle it
    trivially").
- A *second* finding (recursive-`Drop` stack overflow on deeply `Box`-chained ASTs, discovered
  via the above prototype, not previously documented) is confirmed and unfixed, and is broader
  than modal formulas — see above.

Neither is shipped. No production code was changed by this investigation; the prototype
(`checking.rs`'s `run_with_deep_stack`, its use in `modal_specification.rs`) was reverted after
confirming both points above, and both are recorded here for whoever picks this up next.
