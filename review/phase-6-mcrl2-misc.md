# Phase 6 review: mcrl2 crate miscellany + xtask

Scope: `tools/mcrl2/crates/mcrl2/src/{control_flow,data,global_lock,lib,log}.rs` and
`tools/mcrl2/crates/xtask/`. None of these files contain `unsafe` code (confirmed by grep before
this phase started), so this is pure logic review, not an unsafe/FFI review. Out of scope (already
covered by earlier phases): `tools/mcrl2/crates/mcrl2/src/{atermpp/**,data_expression,pbes,
pbes_expression,lps,visitor}.rs` (phase 1b), and `cfg_lps.rs`/`cfg_srf.rs` which layer on top of
`control_flow.rs` (phase 4).

**Verdict**: `control_flow.rs` (the substantial file in scope) is otherwise sound and already
has unusually thorough regression coverage (`crates/merc_pbes/tests/cfg_srf_analysis_test.rs`,
`crates/merc_lps/tests/cfg_lps_test.rs`) pinning down the CFP-classification edge cases a reviewer
would normally reach for first (disjunctive guards, sinks that reset unconditionally, equality
written either way, non-constant writes with a free variable, two independent CFPs). One real
defect was found and demonstrated by direct execution: `ControlFlowGraph::new` collapses two
semantically different summand behaviours ("this summand never touches parameter `d`" vs. "this
summand overwrites `d` with a non-constant expression") into the identical `CfgEdge`, contradicting
`CfgEdge::target`'s own doc comment. It is not exploitable through either of the two current
consumers (`merc_lps::CfgLinearProcessSpecification`, `merc_pbes::CfgPbesSrfLps`) today, for
reasons explained below, but it is a real, demonstrated bug in the public `ControlFlowGraph` API
and a latent risk for the next consumer or for a `live` predicate that is ever slightly
over-permissive. `data.rs`, `global_lock.rs`, `lib.rs`, `log.rs`, and `crates/xtask` contain no
non-trivial logic (thin FFI wrappers, a mutex, re-exports, a `match` and a `println!`) and no
defects were found in them.

## Finding 1 — `ControlFlowGraph::new` reports "overwritten with an unknown value" identically to "left unchanged", contradicting `CfgEdge::target`'s doc — CONFIRMED (API contract), not reachable through current callers

`tools/mcrl2/crates/mcrl2/src/control_flow.rs:279-330` (`SummandAnalysis`, `analyse_summand`) and
`:117-137` (the `edges` construction loop in `ControlFlowGraph::new`).

`SummandAnalysis::target` distinguishes three cases per parameter, by design (its own doc
comment): the key is *absent* when the summand leaves the parameter unchanged; the value is
`Some(c)` when the summand assigns it the closed value `c`; the value is `Some(None)` — i.e. the
key is *present* but the value is `None` — when the summand assigns it a **non-constant**
expression. `is_control_flow_parameter` correctly uses this three-way distinction to disqualify a
parameter whenever a *live* summand hits the third case (`target.is_none()` after `Some(target) =
analysis.target.get(&j)`, `control_flow.rs:362-366`).

But when `ControlFlowGraph::new` builds the public `CfgEdge<V>` for each summand
(`control_flow.rs:121-134`), it collapses the second and third case back together:

```rust
let target = analysis
    .target
    .get(&j)                       // None (unchanged) | Some(None) (non-constant) | Some(Some(c))
    .and_then(|value| value.as_ref())   // None | None | Some(c)
    .map(|term| intern_term(&mut intern, term));
```

Both "the key is absent" (truly unchanged, a self-loop) and "the key is present with value `None`"
(assigned a non-constant expression, i.e. genuinely unknown after firing) produce `target: None`
in the resulting `CfgEdge`. But `CfgEdge::target`'s doc comment (`control_flow.rs:50-53`) says:

> `None` when the summand does not change it, so the edge arrives back at `source` (a self-loop).

A summand that overwrites `d` with an unrelated free variable is not a self-loop: `d`'s value after
firing need not equal its value before firing. A consumer reading only `ControlFlowGraph::edges()`
(the only public accessor for this information) cannot tell the two situations apart.

This can only surface for a summand excluded by the caller's `live` predicate: for any *live*
summand, `is_control_flow_parameter` already requires a non-constant target to disqualify the
parameter entirely (so the parameter would not appear in `control_flow_parameters` and no edge for
it would ever be built for any summand). Both current callers happen not to expose this in
practice: `merc_lps::CfgLinearProcessSpecification` always passes `live = |_| true` (every summand
is live, so the disqualifying case is impossible per the paragraph above), and
`merc_pbes::CfgPbesSrfLps` passes a real `live` (per-equation reachability) but never exposes
`ControlFlowGraph::edges()` outside the crate, and only ever reads `edge.source` in its own
`prepare()`, never `edge.target` (`crates/merc_pbes/src/cfg_srf.rs:261-264`,
`crates/merc_lps/src/cfg_lps.rs:258-261`) — so today's dead-equation summands, even though they
can legitimately hit this case, never have it observed. This is why the bug has not caused a wrong
exploration result so far; it is nonetheless a real, directly demonstrated defect in the module's
public contract, not a hypothetical one.

**Evidence** (direct unit test against `mcrl2::ControlFlowGraph`, bypassing both wrapper crates):

```
cd tools/mcrl2
cargo test -p mcrl2 --test control_flow_edge_test
```

```
running 1 test
test unchanged_edge_is_distinguishable_from_non_constant_write_edge ... FAILED

thread '...' panicked at crates/mcrl2/tests/control_flow_edge_test.rs:154:5:
assertion `left != right` failed: a summand that overwrites `d` with a non-constant expression
(`d := e`) must not be reported identically to a summand that truly leaves `d` unchanged: both
currently normalise to `target: None`, silently reporting the non-constant write as a self-loop
  left: CfgEdge { position: 0, source: Some(0), target: None }
 right: CfgEdge { position: 0, source: Some(0), target: None }

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
```

The test builds a 3-summand LPS (`proc P(d: Nat, e: Nat) = (d==1) -> a.P(e,e) + (d==1) -> b.P(d,e)
+ (d==2) -> c.P(3,e)`) and calls `ControlFlowGraph::new` directly with only summand 2 marked
`live` — enough for `d` to be a genuine CFP (read via `d==2`, only ever written a constant by the
one live summand) while summands 0 (`d := e`, non-constant) and 1 (`d` untouched) stay dead and
therefore exempt from the "writes must be constant" requirement. `edges(0)` and `edges(1)` come
back byte-for-byte identical `CfgEdge`s even though summand 0's actual post-state for `d` is
unconstrained and summand 1's is pinned to the pre-state value.

Why this test would pass once fixed: any fix that keeps the third state observable in `CfgEdge`
(e.g. widening `target` to an enum distinguishing `Unchanged`/`Constant(V)`/`Unknown`, or exposing
`SummandAnalysis`-level detail) would make `edges(0)` and `edges(1)` differ on exactly that
dimension, satisfying the `assert_ne!`.

Direction of a fix (not applied, per review scope): give `CfgEdge::target` a third state (e.g.
`enum CfgTarget<V> { Unchanged, Constant(V), Unknown }` in place of `Option<V>`), and update both
current consumers, which only match on `Some(v)`/`None` today and would need one extra arm treating
`Unknown` as "no constraint" (same as their current `None` handling) to stay behaviourally
unchanged.

### Outcome: FIXED

Applied exactly the suggested direction: `CfgEdge::target` is now `CfgTarget<V>`, a new public enum
with `Unchanged`, `Constant(V)`, and `Unknown` variants (`control_flow.rs:57-75`), re-exported from
`crates/mcrl2/src/lib.rs` alongside `CfgEdge`. `ControlFlowGraph::new`'s edge-construction loop
(`control_flow.rs:121-126`) now matches `analysis.target.get(&j)`'s three states explicitly instead
of collapsing the last two through `.and_then(Option::as_ref)`:

```rust
let target = match analysis.target.get(&j) {
    None => CfgTarget::Unchanged,
    Some(None) => CfgTarget::Unknown,
    Some(Some(term)) => CfgTarget::Constant(intern_term(&mut intern, term)),
};
```

The inclusion test (`source.is_some() || matches!(target, CfgTarget::Constant(_))`) is the exact
same predicate as before (`target.is_some()` only matched the `Constant` case even previously), so
which summand/parameter pairs get an edge at all is unchanged — only the `target` value inside an
already-included edge is now correct. Updated `CfgEdge::target`'s and the struct-level doc comment
to describe the three cases instead of documenting a `None` that meant two different things.

Checked both real consumers for call sites touching `.target`: neither
`crates/merc_lps/src/cfg_lps.rs` nor `crates/merc_pbes/src/cfg_srf.rs` reads `edge.target` at all
(`grep -n "\.target\b" crates/merc_lps/src/cfg_lps.rs crates/merc_pbes/src/cfg_srf.rs` — no
matches; both only read `edge.source` at the line numbers this review already cited), so neither
needed a code change for the new representation — confirming the review's own blast-radius
analysis that today's two consumers are unaffected by this type change.

Regression test kept, now green instead of failing:

```
cd tools/mcrl2
cargo test -p mcrl2 --test control_flow_edge_test
```
```
running 1 test
test unchanged_edge_is_distinguishable_from_non_constant_write_edge ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.25s
```

Full verification:
- `cargo nextest run -p mcrl2 --no-fail-fast -- --include-ignored` (from `tools/mcrl2`): all tests
  pass, no regressions.
- `cargo test -p merc_lps --test cfg_lps_test` (from `tools/mcrl2`): all tests pass unchanged.
- `cargo test -p merc_pbes --test cfg_srf_analysis_test` (from `tools/mcrl2`): all tests pass
  unchanged.
- `cargo clippy -p mcrl2 -p merc_lps -p merc_pbes --all-targets` (from `tools/mcrl2`): clean.
- `cargo +nightly fmt --all -- --check` (repo root): clean.

## Checked and found correct

- **The "first conjunct wins" behaviour in `as_parameter_equality`/`analyse_summand`**
  (`control_flow.rs:304-311`, `.entry(index).or_insert(value)`): when a guard has two `d == c`
  conjuncts on the same parameter with different `c`, only the first is recorded as the source.
  Traced this through to both consumers' `prepare()` (`cfg_lps.rs:258-261`,
  `cfg_srf.rs:261-264`) and to `enumerate()`/`enumerate_raw_with_current_assignments`
  (`crates/merc_lps/src/explore_explicit.rs:796-822`): the control-flow graph is only ever used to
  *drop* candidate summands before the real guard is evaluated by the mCRL2 backend, never to add
  one — so recording any one of several necessary `d == c` conjuncts is sound (failing that one
  conjunct is always a valid reason to prune; not failing it never causes a transition to be
  fabricated, since the real guard is still evaluated for every retained candidate). No
  under-approximation is possible from this path.
- Empty-input edge cases: zero summands, zero parameters, and summands with no write
  assignments and/or no guard conjuncts all fall through the same code paths without panicking
  (`(0..0).filter(...)` degenerates to an empty `Vec`, `Vec::with_capacity(0)` for `edges`,
  `collect_conjuncts` on a non-`&&` expression pushes exactly the one conjunct).
- `CfgDisplay::fmt`'s indexing (`self.parameters[j]` for `j` drawn from
  `graph.control_flow_parameters()`) cannot go out of bounds: `control_flow_parameters` is built
  by filtering `0..parameters.len()`, and `CfgDisplay` is always constructed with the exact same
  `parameters` slice inside `ControlFlowGraph::new`.
- The identity-assignment skip (`lhs_arg.copy() == rhs_arg.copy()`, `control_flow.rs:319-321`) is
  sound given this codebase's aterm invariant (structural equality is pointer equality via
  hash-consing), and is redundant-but-harmless with `CfgSummand::write_assignments`'s documented
  precondition that it already carries only non-identity assignments.
- `data.rs`, `global_lock.rs`, `lib.rs`, `log.rs`: no branches, loops, or non-trivial arithmetic to
  attack. `DataSpecification::spec_ref()`'s `.expect("...is never null")` is an assumption about
  the external `mcrl2-sys` FFI crate (not part of this phase's scope, and not vendored in this
  repository), so it is left as an unverified claim rather than a finding.
- `tools/mcrl2/crates/xtask`: `add_target_flag`'s `cfg!`/`#[cfg(...)]` dispatch and `main.rs`'s
  argument matching are straight-line and exhaustively guarded (`match task.as_deref()` has a
  catch-all); nothing to demonstrate a wrong result with.

## Tests added

- `tools/mcrl2/crates/mcrl2/tests/control_flow_edge_test.rs` —
  `unchanged_edge_is_distinguishable_from_non_constant_write_edge` (Finding 1's regression test).
  Run with:
  ```
  cd tools/mcrl2
  cargo test -p mcrl2 --test control_flow_edge_test
  ```
