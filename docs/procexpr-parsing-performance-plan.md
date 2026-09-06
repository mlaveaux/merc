# `ProcExpr`/`DataExpr` parsing performance — plan

This is a design and implementation plan for removing the O(n²) (and, on large
inputs, stack-overflow) parsing behaviour in
[mcrl2_grammar.pest](../crates/syntax/mcrl2_grammar.pest), which currently
keeps two real-world example specs
([MLV.mcrl2](../examples/mCRL2/industrial/MLV/MLV.mcrl2) and
[garage-ver.mcrl2](../examples/mCRL2/industrial/garage/garage-ver.mcrl2))
disabled in
[example_test.rs](../crates/syntax/tests/example_test.rs).

**Constraint, unchanged throughout:** every input that currently parses
successfully must keep producing the *identical* AST — same tree shape, same
spans. This is a pure performance change.

**Status (updated).** Phase 1 is implemented and merged. The "reuse pest's own
generated parsing functions" design sketched below in §3.3 turned out to be
**infeasible**: `pest_generator` 2.9.0 emits every per-rule matcher inside a
function-local module scoped to the one generated `Parser::parse` function,
unreachable from any other code — confirmed by reading its actual codegen,
not assumed. §3.2's marker-insertion idea was implemented instead (as
[condition_marker.rs](../crates/syntax/src/condition_marker.rs)) and is now
the real Phase 2: a linear pre-scan inserts a private-use-area marker before
every condition it can identify as genuine, `ProcExprIfPrefix`/`ProcExprIfThen`
require that marker, and [with_offset_corrections](../crates/utilities/src/span.rs)
undoes the span shift.

This is **substantially working**: every one of the 398 existing
`merc_syntax` tests still passes byte-for-byte (verified via the snapshot
suite, not just "compiles"), and along the way this surfaced and fixed
several real scanner bugs, each now covered by a regression test in
`condition_marker.rs` — most notably that `Action`/`ProcExprId` argument
lists that are empty or assignment-shorthand (`f()`, `f(x=y)`) are *not*
valid `DataExprApplication`, so a chain cannot reach through them in either
direction, and that a parenthesized operand whose content is itself
process-only (not a plain `DataExpr`) cannot be `DataExprBrackets` either —
getting either of these wrong let a stale chain "skip over" intervening
content and wrongly attach to a `->` much further away, silently producing a
different (though still hard-failing, per the marker's uniqueness — never a
*silently wrong* AST) parse than intended.

**§1b found, and now fixed.** Bisecting `MLV.mcrl2` (binary search over
line-prefixes of its *marked* text, each attempt pest-parsed directly with a
3s thread-timeout) isolated the hang to byte-identical certainty: it was not
a scanner coverage gap. The `ConfigurationMemory` process (source lines
821-930) is a `+`-chain of ~40 terms, every one shaped `sum v: Nat . (cond)
-> action(v) . Call(field = v)` — i.e. every single term genuinely **is** a
condition, and `mark_process_conditions` correctly marked every one of them
(confirmed: the scanner found all 90 marks across the file in 2.5ms). The
hang survived anyway. Root-caused to a **second, independent
exponential-time source**, §1b below, that the original marker fix did not
touch and was never designed to touch — confirmed **not a regression from
this session's Phase 1 merge** by diffing against `HEAD`: the original,
unmodified grammar already tried `ProcExprIfThen` (mandatory `<>`) as a
`ProcExprPrefix` alternative before falling back to bare `ProcExprIf`, the
same failure shape as today's merged `ProcExprIfPrefix`'s optional tail —
Phase 1 restructured this into one rule, it did not change its complexity.

§1b is now fixed with a second marker, `ELSE_MARKER`, computed by the same
scanner in the same linear pass (see §1b for the mechanism). Both
`MLV.mcrl2` and `garage-ver.mcrl2` now parse successfully — 53ms and 96ms
respectively — and their `#[test_case]`s in `example_test.rs` are re-enabled
with freshly generated snapshots. All 503 `merc_syntax` tests
(`--include-ignored`) pass, including 30 in `condition_marker.rs` (5 new,
covering §1b specifically: a bare condition gets no `ELSE_MARKER`, a
chain of many bare conditions gets none of them, the nearest-enclosing-if
rule is respected, and `ELSE_MARKER` scope does not cross a closing
bracket).

## 1. Root cause (verified empirically, not just suspected)

`ProcExprPrefix` includes `ProcExprIfPrefix`/`ProcExprIfThen`, which start by
parsing a full `DataExpr` looking for a trailing `->`:

```
ProcExprIfPrefix = { DataExpr ~ "->" ~ (ProcExprNoIf ~ "<>")? }
ProcExprIfThen    = { DataExpr ~ "->" ~ ProcExprNoIf ~ "<>" }
```

`DataExprInfix` includes `+`, `.` (`DataExprAt`) and `||` (`DataExprDisj`) —
the *same tokens* `ProcExprInfix` uses for choice/sequence/parallel. Because
pest's `*`/`+` repetition is possessive (it never backtracks into a shorter
match once committed), `DataExpr`'s own infix loop greedily walks straight
through a `+`/`.`/`||`-separated process chain (each operand, e.g. `a1(1)`,
is itself a syntactically valid `DataExprPrimary`/`DataExprApplication`) —
mis-parsing it as one giant data expression — before failing to find `->` and
backtracking out entirely.

The expensive part: `ProcExprPrefix*` is retried at **every operand
position** of the enclosing `(ProcExprInfix ~ ProcExprPrefix* ~
ProcExprPrimary ~ ...)*` loop. This retry is *semantically required* — `a +
(b -> P <> Q)` is legal mCRL2 (confirmed: it's exactly the shape the
`WMS.mcrl2` regression edit exercises, `... <> (nested if) + Monitor(...)`)
— so the if-check genuinely must be attempted at each of the n operand
positions. At position *i* in an *n*-term chain, a doomed attempt costs
O(n−i); summed over all *i* that's O(n²), and at n≈1600 with `.`/`||` chains
it stack-overflows and aborts the process instead of just being slow.

## 1b. A second, independent root cause: the optional `<>`-tail itself

This is distinct from §1 above and **not fixed by the condition-marker
approach**, because it strikes even at positions where the condition is
genuine and correctly marked. Found by bisecting `MLV.mcrl2` down to its
`ConfigurationMemory` process, then reproduced in isolation with a clean
synthetic file to rule out any remaining scanner involvement:

```
proc P(v: Nat) =
  ⟨MARKER⟩(v == 0) -> a0(v).P(v)
  + ⟨MARKER⟩(v == 1) -> a1(v).P(v)
  + ⟨MARKER⟩(v == 2) -> a2(v).P(v)
  ...  // n terms, every condition already correctly marked, no fallback needed
;
init P(0);
```

Fed directly to `Mcrl2Parser::parse(Rule::MCRL2Spec, ...)` — bypassing
`condition_marker` entirely, so this measures the `.pest` grammar alone with
perfect marker placement already given to it "for free":

| n   | result                        |
|-----|-------------------------------|
| 10  | 165 ms                        |
| 20  | > 3 s (timeout)                |
| 40  | > 3 s (timeout)                |
| ... | > 3 s (timeout)                |
| 640 | stack overflow, process abort |

This is not merely quadratic — n=10→20 (2×) goes from 165ms to over 3
seconds (>18×), and by n=640 it overflows the stack, matching an
exponential-recursion shape, not a polynomial one.

**Mechanism**: `ProcExprIfPrefix = { ProcExprConditionMarker ~ DataExpr ~
"->" ~ (ProcExprNoIf ~ "<>")? }`. Once the condition and `->` match, the
grammar unconditionally *also* attempts `ProcExprNoIf ~ "<>"` to see whether
this `if` has an explicit `else`. `ProcExprNoIf`'s own repetition is
possessive and greedy exactly like `DataExpr`'s (§1) — it walks forward
through the *rest of the `+`/`.`/`||` chain*, and **at every subsequent
operand position it encounters, it again tries `ProcExprPrefix`, which again
tries `ProcExprIfPrefix`, which again attempts its own optional
`ProcExprNoIf ~ "<>"` tail** — recursively, before ever getting to see
whether a `<>` follows. For a chain of *n* if-without-`<>` terms this
produces the same shape as naive recursive Fibonacci (`T(n) = T(n-1) +
T(n-2) + ... `), i.e. genuinely exponential, not just O(n²).

Confirmed **not a regression introduced by this session's Phase 1 merge**:
`git diff HEAD` shows the pre-existing, unmodified grammar had `ProcExprIf`
= `{ DataExpr ~ "->" }` and a separate `ProcExprIfThen = { DataExpr ~ "->" ~
ProcExprNoIf ~ "<>" }`, with `ProcExprPrefix` trying `ProcExprIfThen`
*before* falling back to `ProcExprIf` — the exact same "eagerly try the full
then-plus-`<>` shape, backtrack the whole thing on failure" structure, just
spread across two rules instead of one rule's optional group. Phase 1's
merge (§2 below) changed *which rule* encodes this, not its complexity.

**Why §3.3's reachability-table design (as originally sketched) does not
fix this**: that design only gates *whether `ProcExprIfPrefix` is attempted
at all* at a given position (replacing "walk to find `->`" with an O(1)
table lookup) — it says nothing about what happens *after* the condition and
`->` are found, which is exactly where this second blowup lives. A complete
fix needs a **second** precomputed table — call it `has_else: Vec<bool>`,
answering "does the `<>` that would terminate *this* if's then-branch, under
mCRL2's standard nearest-enclosing-if binding rule, actually exist reachably
before the enclosing chain's own terminator?" — computed by a linear,
process-expression-aware scan (mirroring `condition_marker`'s technique, but
walking `ProcExpr` shapes instead of `DataExpr` shapes, and needing to
track nested if/then/else nesting depth to resolve "which `<>` belongs to
which `if`" the same way the grammar's `ProcExprNoIfInfix`/`ProcExprIfThen`
split already does structurally). Positions where `has_else` is `false` must
skip the `(ProcExprNoIf ~ "<>")?` attempt entirely — relying on the
*outer* `ProcExpr`'s own `ProcExprPrimary`/`ProcExprInfix` loop to parse the
then-branch as ordinary chain continuation instead (this is, in effect, what
the original two-rule `ProcExprIf`/`ProcExprIfThen` design did for the
"just a bare condition, no `<>`" case — `ProcExprIf` never attempted to
consume a then-branch at all).

**Implemented** as sketched above, reusing the existing `condition_marker`
infrastructure rather than a separate pass: `mark_process_conditions`'s
single left-to-right scan now also maintains a `pending_ifs: Vec<usize>`
stack, local to each `scan_expr_region` call/recursion (which is exactly the
scoping needed — see below). On every `->` found, the position right after
it is pushed; on every bare `<>` token found, the top of the stack is popped
and resolved as having a match — the standard nearest-enclosing-if
(dangling-else) rule, and exactly what a left-to-right scan naturally
produces (PEG's own greedy, depth-first matching would reach that same `<>`
from the *innermost* still-open condition's own tail-attempt first, being
nested deeper in the parse). Entries never popped by the time their
enclosing `scan_expr_region` call returns (region ends at `;`, `,`, a
closing bracket, or EOF) are simply dropped — correctly leaving those
conditions marked as having no `<>`, with no extra code needed, since a `<>`
inside a nested `(...)` can only close an `if` inside those same parens, and
every such nested region already gets a *fresh* `scan_expr_region`
call/stack.

A new codepoint, `ELSE_MARKER` (`'\u{E001}'`, same Private-Use-Area
rationale and same UTF-8 length as `CONDITION_MARKER` — checked with a
`const` assertion so both can share one insertion pass and one
`with_offset_corrections` call), is inserted at each resolved position.
`mcrl2_grammar.pest` requires it before ever attempting the `<>`-tail in
both `ProcExprIfPrefix` and `ProcExprIfThen` (the latter needs it too: it is
reached, in a genuine then-branch scan, at exactly the same textual
position, so an ungated `ProcExprIfThen` would reintroduce the same
recursive blow-up whenever it is tried against a bare condition):

```
ProcExprIfPrefix = { ProcExprConditionMarker ~ DataExpr ~ "->" ~ (ProcExprElseMarker ~ ProcExprNoIf ~ "<>")? }
ProcExprIfThen    = { ProcExprConditionMarker ~ DataExpr ~ "->" ~ ProcExprElseMarker ~ ProcExprNoIf ~ "<>" }
```

Both markers are silent (`_{ }`), so this changes nothing in the produced
`Pairs`/AST for anything that already parsed — `consume.rs` and
`precedence.rs` needed zero changes, same as the original `CONDITION_MARKER`
addition. If `has_else`-equivalent resolution is ever wrong for some input
(a bug, not a fundamental gap), the same safety property holds as for
`CONDITION_MARKER`: the marker's uniqueness in the grammar means a
wrongly-placed or wrongly-omitted `ELSE_MARKER` can only produce a hard
parse error, never a silently different tree, and
`UntypedProcessSpecification::parse`'s existing marked-then-fallback
structure is unchanged.

**Result**: the synthetic if-without-`<>` chain above, re-measured through
the *full* pipeline (`mark_process_conditions` → grammar), now scales
linearly — n=10 → 1.5ms, n=1280 → 127ms, n=2560 → 252ms (within measurement
noise of exactly 2× per doubling). `MLV.mcrl2` parses in 53ms and
`garage-ver.mcrl2` in 96ms (both previously did not finish). All 503
`merc_syntax` tests pass with `--include-ignored`, including both files'
newly re-enabled snapshot tests.

Nothing else in the grammar has this shape (checked and ruled out:
`DataExprPrimary`'s `{`-alternatives, `Action`/`ProcExprId` ordering,
`RegFrm`/`ActFrm`/`StateFrm`/`PbesExpr`/`PresExpr` — all bounded by an
unambiguous delimiter or cheap keyword gate, so they cost only a constant
factor, not a re-triggerable O(n) scan).

## 2. Phase 1 — safe left-factoring (done)

Merged `ProcExprIfThen`/`ProcExprIf` into one rule with an optional tail, so
the condition `DataExpr` is parsed once per attempt instead of twice:

```
ProcExprIfPrefix = { DataExpr ~ "->" ~ (ProcExprNoIf ~ "<>")? }
```

`ProcExprIfThen` itself is kept as a separate rule (still `DataExpr ~ "->" ~
ProcExprNoIf ~ "<>"` with the mandatory tail) because it's also referenced
from `ProcExprNoIfInfix`, where the tail must be mandatory to avoid the
classic dangling-`->`-without-`<>` ambiguity inside a `ProcExprNoIf` context.
Touched: [mcrl2_grammar.pest](../crates/syntax/mcrl2_grammar.pest),
[precedence.rs](../crates/syntax/src/precedence.rs) (Pratt-parser op
registration + `map_prefix` arm), [consume.rs](../crates/syntax/src/consume.rs)
(merged `ProcExprIf`/`ProcExprIfThen` handler). Verified span-safe and
AST-identical via the full `merc_syntax` test suite (snapshot, roundtrip,
grammar, unit tests) plus downstream `merc_typecheck`.

This **halves the constant factor** of each failed attempt but does not
change the asymptotic behaviour: it's a prerequisite for Phase 2 (one call
site to convert instead of two), not a fix on its own.

**Verified insufficient on its own** (2026-09-04): with Phase 1 alone,
`WMS.mcrl2` (the user's edited copy, with the extra `+ Monitor(l1,l2,b);`
branch) parses in 23ms — it was never actually the trigger for the reported
hang. `garage-ver.mcrl2` and `MLV.mcrl2` both still exceed a 45s timeout;
`garage-ver.mcrl2` was run to a 300s budget and killed by `timeout` without
finishing, with RSS climbing from ~59MB to ~400MB — a genuine runaway, not
just slow. Phase 2 is required to re-enable these two tests.

## 3. Phase 2 — eliminate the O(n²) (rethought design)

### 3.1 Why a purely declarative `.pest`-only fix cannot work

This is worth stating precisely, because it constrains every option below.
The cost is inherent to *unbounded lookahead repeated at n independent
positions*: at each operand position we must answer "does a `->` exist,
reachable through the shared-token chain, before some position-specific stop
token?" — and that answer can only be produced by scanning forward from that
position. Pest has no memoization (no packrat caching of `(rule, position)`
results), so nothing expressible purely inside `mcrl2_grammar.pest` — no
lookahead predicate, no rule reordering, no left-factoring — can make that
per-position query cheaper than the scan itself, because the .pest file has
no way to reuse work done answering the query at a *different* position.
Genuinely O(n) requires information computed **once for the whole file** and
then consulted in O(1) at each of the n positions — i.e. memoization in some
form has to enter the picture. The only design freedom is *how* that
memoized information gets in front of the parser without duplicating
`DataExpr`'s grammar or corrupting spans.

Two designs were considered:

### 3.2 Rejected: text-marker insertion

Idea: precompute, in one linear bracket-aware scan, which operand positions
have a reachable `->`; rewrite the source text to insert a sentinel
character (e.g. a Private-Use-Area codepoint) immediately before each
genuine condition, and require that sentinel at the front of
`ProcExprIfPrefix` in the grammar. A missing sentinel makes the rule fail in
O(1) *before* touching `DataExpr`, for every position that would otherwise
have failed expensively.

This works for the complexity problem, but **inserting bytes shifts every
span downstream of the insertion point**. Since `.as_span()` offsets are
used elsewhere in the codebase (IDE hover, `TypingInfo::at_offset`, error
messages), every span produced after an insertion would need compensating
arithmetic, applied consistently at every call site that reads a span from
the affected subtree. That's a correctness-critical, easy-to-get-subtly-wrong
change spread across the tree-walking code, for exactly the part of the
system the "identical AST" constraint is protecting. Rejected in favour of
3.3, which avoids touching the source text at all.

### 3.3 Recommended: precomputed reachability table + a hand-written
`ProcExpr`/`ProcExprNoIf` driver reusing pest's own per-rule functions

Key fact checked directly against `pest_generator` 2.9.0's own codegen
(`generator.rs:301-361`, in the crate's cargo-registry source, not part of
this repo): for **every** rule in the grammar —
including silent (`_{ }`) ones like `ProcExprPrefix`/`ProcExprPrimary` —
`pest_derive` generates a `pub fn <RuleName>(state: Box<ParserState<'_,
Rule>>) -> ParseResult<Box<ParserState<'_, Rule>>>` associated function on
`Mcrl2Parser`, built from the same public `ParserState` combinators
(`state.sequence`, `state.optional`, `state.repeat`, `state.rule`,
`state.lookahead`, `.and_then`, `.or_else`) that any hand-written caller can
use directly. This is not a private implementation detail — it's the same
mechanism `pest`'s own generated code uses internally, and it's public
specifically so a rule can be driven by hand when needed.

That means we don't need to reimplement `DataExpr`, `ProcExprPrimary`, or
any other rule's grammar. We only need to hand-write the *one* piece of glue
that currently causes the blow-up — the `ProcExprPrefix` choice and the
`ProcExpr`/`ProcExprNoIf` repetition loops around it — as Rust functions that
call the **existing, unmodified, pest-generated** functions for everything
else, threading the *same* `ParserState` through (no substring, no new
`Position` — so no span arithmetic is needed anywhere: every span produced
is byte-identical to what the declarative grammar would have produced,
because it's produced by the exact same `state.rule(...)`/`state.match_*`
calls in the exact same order for every input that currently succeeds).

Concretely:

1. **Preprocessing pass** (new, small, tested in isolation): a single
   left-to-right lexical scan over the raw source — bracket-depth aware
   (`(`/`)`, `[`/`]`, `{`/`}` treated as opaque balanced groups, matching
   the fact that every place these tokens nest in the grammar is itself a
   self-contained sub-rule that can't leak an unparenthesized `->` outward)
   — computing, for every byte offset that can start a `ProcExprPrefix`
   attempt, whether a greedy `DataExprPrimary (DataExprPostfix)*
   (DataExprInfix DataExprPrimary (DataExprPostfix)*)*`-shaped walk from
   that offset would reach a bare `->` before hitting a stop token (`;`,
   `<>`, a closing bracket of an *enclosing* group, EOI, or a token that
   can only belong to `ProcExpr`). Output: `reach: Vec<bool>` (or a
   `bit-set`) indexed by byte offset, O(n) time and space, computed once
   before `Mcrl2Parser::parse` is ever called. No grammar semantics are
   duplicated here beyond tokenizing + bracket matching — it does not parse
   `DataExpr`, it only classifies token shapes.

2. **`ProcExprPrefixGated(state)`** — a hand-written replacement for the
   `.pest` file's `ProcExprPrefix` alternation. Tries
   `Mcrl2Parser::ProcExprSum(state)`, then `Mcrl2Parser::ProcExprDist(state)`
   unchanged; only attempts `Mcrl2Parser::ProcExprIfPrefix(state)` when
   `reach[state.position().pos()]` says it can possibly succeed — an O(1)
   table lookup gating the one expensive alternative. Every position where
   the lookup says "no" now fails in O(1) instead of O(n−i).

3. **`ProcExpr(state)` / `ProcExprNoIf(state)`** — hand-written to mirror
   exactly the possessive-repetition shape pest would have generated for
   `ProcExprPrefix* ~ ProcExprPrimary ~ ProcExprPostfix? ~ (ProcExprInfix ~
   ProcExprPrefix* ~ ProcExprPrimary ~ ProcExprPostfix?)*`, but calling
   `ProcExprPrefixGated` in place of the plain generated `ProcExprPrefix`,
   and otherwise calling the **existing, untouched** generated
   `Mcrl2Parser::ProcExprPrimary`/`ProcExprPostfix`/`ProcExprInfix`
   functions. Wrapped in `state.rule(Rule::ProcExpr, ...)` exactly as the
   generated code would, so the resulting `Pairs<Rule>` is structurally
   identical to today's — meaning **`consume.rs` and `precedence.rs` need
   zero changes**, since they only ever see the same rule tags/spans they
   see today.

4. `mcrl2_grammar.pest` keeps `ProcExpr`/`ProcExprNoIf`/`ProcExprPrefix` as
   *documentation* of the intended grammar (pest itself no longer drives
   these three), or they're removed from the `.pest` file with a comment
   pointing at the hand-written equivalents — TBD during implementation,
   whichever keeps the grammar file honest about what actually executes.

**Complexity result:** every position where the if-check would have failed
now costs O(1) (one table lookup) instead of O(n−i); the reach-table itself
is O(n) total. Overall parse time for a chain of n operands becomes O(n).
Positions where the if-check succeeds still cost whatever `DataExpr`
genuinely costs to parse the condition — unchanged from today, and bounded
by the condition's own size, not the whole chain's.

**Risk / correctness plan:**
- The reach-table's bracket-matching scan is the only genuinely new logic;
  it's independently unit-testable (feed it synthetic chains with known
  answers) and differentially testable against a slow-but-obviously-correct
  reference (for small inputs, just try `Mcrl2Parser::parse(Rule::DataExpr,
  ...)` at every position and check whether it's followed by `->`, compare
  against the table) via a property/fuzz test before it ever gates real
  parsing.
- The hand-written `ProcExpr`/`ProcExprNoIf`/`ProcExprPrefixGated` functions
  should be differentially fuzzed against the *current* (Phase 1) grammar's
  declarative `ProcExpr`/`ProcExprNoIf` on random small-to-medium generated
  process expressions before removing the declarative rules, to catch any
  divergence in accepted language or `Pairs` shape early.
- Exact `ParserState` combinator names/behaviour (`sequence`, `optional`,
  `repeat`, `rule`, the `hidden::skip` whitespace/comment splice between
  sequence items) need a final check against pest 2.9.0's public API surface
  during implementation — spot-checked against `pest_generator`'s own
  codegen (§3.3 above) but not yet written and compiled.
- Add, as defense-in-depth independent of whether this lands cleanly: an
  explicit recursion-depth guard so a pathological input produces a parse
  error instead of the stack-overflow/process-abort observed at n≈1600.

### 3.4 Not pursued

- **Switching parser generator** (e.g. to `peg`, which has `#[cache]`
  packrat memoization) or patching the vendored `pest`/`pest_derive`
  crates: too large a blast radius for this fix.
- **`pest3`**: re-checked directly against the `pest-parser/pest3` repo
  (corrects an earlier note here that called it stalled — it is not; commits
  as recent as February 2026, still pre-alpha at `0.0.3`). Its only
  memoization-related work is
  [issue #24](https://github.com/pest-parser/pest3/issues/24) /
  [PR #30](https://github.com/pest-parser/pest3/pull/30) (draft, unreviewed
  as of April 2026), which adds caching *only* to rules explicitly marked
  `@`/`%` as left-recursive, specifically to support left recursion — not
  general packrat caching, and not something that would touch the *silent*
  sub-rules (`DataExprPrimary`, `DataExprInfix`, etc.) where our redundant
  work actually happens (see the equivalent finding for the pest 2.x
  memoization gist below — same conclusion, different repo). `ProcExpr`/
  `DataExpr` aren't left-recursive, so this feature doesn't apply to them
  even once it lands. Also a much bigger migration (different grammar syntax,
  renamed built-ins, a typed output API) for zero benefit here.
- **A [pest 2.x memoization
  experiment](https://github.com/pest-parser/pest/discussions/1081)**
  (unmerged, `HashMap<"{rule}-{pos}", "Err">` added to `ParserState::rule`):
  read the actual diff, not just the discussion summary. It only hooks
  `state.rule()`, which *silent* (`_{ }`) rules like `DataExprPrimary`/
  `DataExprInfix`/`DataExprPostfix`/`ProcExprPrefix` never call. Our
  redundant work is exactly there: for a chain `a1+a2+...+an` with no `->`,
  `DataExpr` (a named rule) is invoked fresh from each `aᵢ` exactly once —
  there's no repeated `(rule, position)` query for memoization to catch —
  but each of those n invocations independently re-walks the *silent*
  primary/infix sub-rules for the remaining suffix, which is where the O(n²)
  actually comes from and which this patch structurally cannot see. Confirmed
  by reasoning through the exact mechanism rather than assumed from the
  reported 1000x speedup, which was very likely on a differently-shaped
  (deeply nested/ambiguous, genuinely-repeated-named-rule) grammar. Not worth
  forking pest for.
- **`merc_pest_consume`** (the `pest_consume` fork used for the AST-building
  layer): only wraps the post-parse `Pairs` walk, can't intervene during
  pest's own mid-parse backtracking — irrelevant to this problem either way.

## 4. Phase 3 — re-enable what this was blocking

- **Done**: `MLV.mcrl2` and `garage-ver.mcrl2` `#[test_case]` lines in
  [example_test.rs](../crates/syntax/tests/example_test.rs) are restored,
  with freshly generated snapshots (`result_mlv.mcrl2`,
  `result_garage-ver.mcrl2`).
- The `+ Monitor(l1,l2,b);` edit to
  [WMS.mcrl2](../examples/mCRL2/industrial/DIRAC/WMS.mcrl2) mentioned in an
  earlier revision of this doc is no longer present in the working tree
  (`git diff`/`git status` show none) — nothing to decide here now.
- **Still open**: a permanent regression test (e.g.
  `crates/syntax/tests/procexpr_perf_test.rs`) asserting a large synthetic
  `+`/`.`/`||` chain (~2000 terms, both with and without `<>`, per §1 and
  §1b) parses within a generous time bound, so a future regression fails
  fast in CI instead of silently reintroducing quadratic/exponential
  behaviour. `condition_marker.rs`'s own unit tests cover correctness of the
  marking, not a timing regression guard.
- **Still open, optional**: `crates/syntax/benchmarks` (criterion, following
  the `crates/sabre/benchmarks` layout) parsing
  `WMS.mcrl2`/`MLV.mcrl2`/`garage-ver.mcrl2` for before/after tracking.

## 5. Verification

- **Done**: `cargo nextest run -p merc_syntax --include-ignored` — 503
  tests pass, snapshot + roundtrip suites byte-identical for every
  previously-passing case, fresh snapshots generated for the two newly
  re-enabled cases.
- **Done**: re-ran the §1 quadratic-chain probe and the §1b if-without-`<>`
  probe through the full `mark_process_conditions` → grammar pipeline —
  both now scale linearly (§1b's synthetic chain: n=10 → 1.5ms, n=2560 →
  252ms).
- **Done**: timed `garage-ver.mcrl2` (96ms) and `MLV.mcrl2` (53ms) directly
  via `UntypedProcessSpecification::parse` — both previously did not finish.
- **Done**: `cargo +nightly fmt --check` and `cargo clippy --all-targets`
  for `merc_syntax`/`merc_utilities` — clean (one pre-existing, unrelated
  `traverse.rs` clippy warning left as-is).
- **Still open**: the §3.3 differential-fuzz idea does not apply — §3.3 was
  never implemented (superseded by the marker approach); a differential
  fuzz test comparing `mark_process_conditions`'s marking decisions against
  a slow-but-obviously-correct reference (e.g. trying
  `Mcrl2Parser::parse(Rule::DataExpr, ...)` at every candidate position)
  would still be a reasonable follow-up hardening step for both markers.
- **Still open**: run the `check` skill's full scope (all three workspaces,
  including `tools/mcrl2`/`tools/gui`) before considering this fully closed
  — this session verified the root workspace only.
