# Two constraint-solver bugs found while fuzzing richer generated specs (both fixed)

Found while extending the random generators to draw on struct/container/function
sorts (a `Function`-sorted value generates as a `lambda`, and richer
struct/container sorts generate `Set`/`Bag` literals needing several elements).
Both are real, reproduced with a two-line spec, root-caused precisely, and are
now fixed and shipped.

## 1. Function sorts had no subtyping at all

- **Location**: `crates/typecheck/src/inference/unification.rs`
  (`Unifier::strict_related_sorts`), `crates/typecheck/src/inference/resolved_sort.rs`
  (`SortInterner::is_materializable`), `crates/typecheck/src/lowering/mcrl2_lowering.rs`
  (`Lowering::coerce`).
- **Scenario**: a `Sub` constraint between two function sorts (`D -> S` and
  `D -> T`, same domain, `S` a subsort of `T`) could never succeed: `Unifier
  ::strict_related_sorts` returned no related sorts for a `Function` head, so
  the widening search had nothing to try beyond exact structural equality. A
  lambda body whose own natural (unwidened) sort differed from what the
  lambda's declared/expected range required had nothing to widen against and
  failed to type-check, even though the identical expression works fine
  outside a lambda -- and a **named** function value (a bare `map` reference,
  not a literal lambda) couldn't widen its range *at all*, in any context,
  since there was no body to push a fix into.
- **Minimal repros**:
  - `sort S = struct c0; map f: (Nat -> Bag(S)); eqn f = lambda x: Nat . {c0: 1};`
    -- `{c0: 1}`'s natural sort is `FBag(S)`, needs to widen to `Bag(S)`.
  - `map inc: Nat -> Nat; eqn inc = lambda y: Nat. y + 1;` -- `y + 1`'s natural
    sort is `Pos`, needs to widen to `Nat`.
  - `map g: Nat -> Nat; h: Nat -> Int; var x: Nat; eqn g(x) = x; h = g;` -- `g`
    is not a lambda at all, so no per-body fix could ever reach this case.
- **Fix**: function sorts now support real, if narrow, subtyping: `D -> S` is
  a subsort of `D -> T` exactly when the domain `D` is identical and `S` is a
  subsort of `T` (the domain itself never widens/narrows -- that would be
  contravariant and isn't needed by anything this fixes). This is wired
  through the same three places every other kind of widening already goes
  through:
  - `Unifier::strict_related_sorts`'s `Function` case now recurses into the
    range's own related sorts and rewraps each one in the same domain, for
    both a `Resolved` function sort and a still-structural `InferSort::Function`
    node (so the search works whether or not the range is fully resolved
    yet).
  - `SortInterner::is_materializable` gained the matching `(Function,
    Function)` case (`from_domain == to_domain && is_materializable(from_range,
    to_range)`), since `coerce()` gates on it before building anything.
  - `Lowering::coerce`'s new `(Function, Function)` case builds the only thing
    that actually **can** be built here: an eta-expansion. `term` of sort
    `D -> S` becomes `lambda d: D. coerce(term(d), S, T)` of sort `D -> T` --
    unlike a number (`Pos2Nat`, ...) or a container (`SetFSet2Set`, ...),
    there is no builtin mCRL2 operator that coerces a whole function *value*
    directly; wrapping it in a fresh lambda that applies it and coerces the
    result is the only term-level construction available, and it works for
    *any* function-sorted expression, not just a literal lambda -- which is
    exactly what fixes the named-function-reference case.

  With this in place, `ConstraintGenerator::visit`'s `Lambda` case needs no
  special-casing at all any more: it just builds `domain -> body_sort`
  directly (the pre-existing, simplest possible code), and lets whatever `Sub`
  a use site imposes on the *whole* lambda widen through the mechanism above.
  This is a **simplification**, not just a fix: two earlier, narrower attempts
  at this file's Lambda case (wrapping the body in a fresh `Sub`-constrained
  variable, one unconditionally and one scoped to specific body shapes) are
  both gone, replaced by the one general mechanism above, which covers
  everything they did plus the named-function case they couldn't reach.

- **A discovered, and reverted, cosmetic side effect**: making function sorts
  widen also gives the solver a *second* way to satisfy some `Sub`s that
  previously had only one route -- e.g. for `inc = lambda y: Nat. y + 1;`,
  the solver can now either (a) leave the lambda's own sort at its natural
  `Nat -> Pos` and eta-expand a wrapper coercing the *whole value* once the
  declared `Nat -> Nat` is known, or (b) resolve `y + 1`'s own `+` overload at
  `Nat # Nat -> Nat` directly. mCRL2's own checker does (b); merc's solver
  is a strictly ascending-distance, lexicographic (earliest-constraint-first)
  search (see `Solver::dominated`'s slice comparison and `solve_widening`'s
  own doc comment) that happens to reach a same-total-distance (a) first once
  it exists as an option, since committing to the lambda's own free variable
  at zero cost immediately succeeds and the search never backtracks to try a
  costlier local choice merely because it might be cheaper somewhere later.
  A first attempt to fix this by making a function-level widening artificially
  more expensive (`Solver::solve_widening` pushing an extra flat penalty when
  the widened sort is a function) was tried and **found to be a no-op**: the
  penalty is only ever compared once the search already knows *some*
  completion exists from the current (cheaper, unpenalized) branch, and by
  then `solve_widening`'s own "first success is final for this `Sub`" rule
  (a real invariant under this lexicographic ordering, not a shortcut -- see
  that function's doc comment) has already committed and returned, without
  ever reaching the penalized branch to compare against. The penalty was
  reverted rather than kept as dead code. The result is a real, structural
  divergence from the oracle in `inc`'s specific lowered *term* shape (an
  eta-expanded wrapper around the original lambda, rather than the single,
  simpler lambda the toolset builds) on top of the already-accepted
  "Expected-sort propagation" sort divergence -- both are semantically sound,
  neither is a soundness bug, and `test_round_trip_lambda_and_higher_order`
  already excludes exactly the one section (`Equations`) where this shows up
  (see below).

### The oracle divergence, and why it is not a bug

`test_round_trip_lambda_and_higher_order`'s `inc = lambda y: Nat. y + 1;`
lowers to a *structurally different but equally well-typed* term than the
real mCRL2 toolset produces, for the reasons above. This is the **same,
already-documented "Expected-sort propagation" divergence** as
`test_round_trip_positive_literal_argument`/
`test_round_trip_where_clause_with_positive_binding` a few tests down in the
same file (see `docs/developer/typechecking/data-specification.md`, "Known
divergences from the mCRL2 toolset") -- this session's fix just made a lambda
body the third place that same divergence is reachable from, plus (as
explained above) a second, independent difference in how the coercion is
*shaped* once one is needed. The test was converted from `assert_round_trips`
to `assert_sections_round_trip` excluding `Section::Equations`, matching the
existing precedent exactly (`apply`'s own equation, which has no arithmetic,
still conforms and is still checked).

## 2. `solve_join`'s fast path never backtracked

- **Location**: `crates/typecheck/src/inference/inference.rs`, `Solver::solve_join`.
- **Scenario**: when 2+ `Sub` constraints target the same free variable (e.g. every element of a `Set`/`Bag` literal), `merge_shared_subs` merges them into one `Join` constraint computing their least-upper-bound directly, replacing the old order-sensitive two-`Sub` encoding (see the `Join` struct's own doc comment, which already describes the general class of bug this replaced). The fast path in `solve_join` computed `lub` from the sources' mutual join, `unify`d `join.target` to it, and called `self.solve(index + 1)` -- but if that later solving failed, `solve_join` just returned `false` with **no fallback** to `solve_join_seq` (the per-source sequential search it already has, sitting right below, used for the "not materializable"/"can't join" cases). A `target` that needs to be *wider* than every source's own mutual join (because a *later* constraint -- e.g. an enclosing container -- requires it) had no way to be found.
- **Minimal repro**: `map v: Set(Bag(Nat)); eqn v = { {1: 1}, {2: 1} };` -- both elements' natural type is `FBag(Nat)`, joining trivially to `FBag(Nat)`; the declared `Set(Bag(Nat))` needs `Bag(Nat)`, one step wider than the join.
- **Fix**: snapshot before the fast-path `unify`+`solve`, and on failure `rollback_to` the snapshot and fall through to `solve_join_seq` (already written, already correct) instead of returning `false` directly.
- **Status**: fixed and shipped. Verified against the full `merc_typecheck` suite and the `mcrl2` oracle-conformance suite.

## A third bug found once both of the above were fixed

Once findings 1 and 2 above no longer blocked the generators, running the
newly-enabled fuzz tests surfaced one more, purely in the **generators**, not
the type checker: `crates/syntax/src/random_pbes.rs`'s `random_param_value`
called `random_integer_data_expression` directly for a `Pos`/`Nat`/`Int`
predicate-variable parameter alike. That generator can produce `0` or a
`Subtract`, neither of which is a valid `Pos` value, and a `Subtract` can go
negative, which is not a valid `Nat` value either (`m - 2` where `m: Nat`
naturally has sort `Int`, not `Nat` -- both sorts are arbitrary-precision, so
this is never an overflow/fixed-width "out of range" issue, only a "outside
this sort's own value set" one) -- the exact same hazard
`random_value_expression`'s own `Pos`/`Nat` cases already guard against.
Fixed by having `random_param_value` delegate to `random_value_expression`
uniformly instead of special-casing the three numeric sorts. The same latent
issue existed in `crates/syntax/src/random_lps.rs`'s process-variable
parameter values (there was no fuzz test exercising it before this session,
only a print/parse round-trip); fixed the same way.

## Practical impact

`crates/syntax/src/random_value_expression.rs` (a general random-value
generator for an arbitrary declared sort), `random_pbes_with_data_specification`,
`random_pres_with_data_specification`, and
`make_process_specification_with_data_specification` all generate
`Function`-sorted values (lambdas) freely and type-check cleanly; their fuzz
tests run unconditionally (no `#[ignore]`). Permanent regression coverage for
finding 1 lives in `crates/typecheck/tests/inference_test.rs`
(`test_lambda_body_widens_a_finite_container_literal`,
`test_lambda_body_widens_a_primitive`,
`test_function_sorted_list_element_widens`,
`test_named_function_reference_widens`).
