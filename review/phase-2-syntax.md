# Phase 2 review: crates/syntax

**Verdict:** `crates/syntax` is not in the unsafe-code list (`grep -rn unsafe crates/syntax/src` finds nothing except `#![forbid(unsafe_code)]` in `lib.rs` — confirmed, this is a pure-safe crate, no miri/loom needed) and its parsing logic (Pratt-parser precedence tables, `specs.rs` collectors, `span_offset.rs`) is largely sound, but `MultiAction`'s hand-rolled `PartialEq`/`Hash` in the primary target file (`specs.rs`) is broken: it reports different multisets of actions as equal whenever they share a repeated element, which also breaks the `Hash`/`Eq` contract for anything hashing a `MultiAction` (or an `ActFrmKind::MultAct` built from one).

## Findings

### 1. `MultiAction::eq` does not implement multiset equality — CONFIRMED

`crates/syntax/src/syntax_tree/specs.rs:320-336`:

```rust
impl PartialEq for MultiAction {
    fn eq(&self, other: &Self) -> bool {
        if self.actions.len() != other.actions.len() {
            return false;
        }
        for action in self.actions.iter() {
            if !other.actions.contains(action) {
                return false;
            }
        }
        true
    }
}
```

`MultiAction` represents a process-algebra multi-action (`a|a|b`), i.e. a multiset. The intended semantics (and what `Hash` at line 338-347 actually implements, by sorting before hashing) is multiset equality. But `eq` checks only that every element of `self` occurs *somewhere* in `other`, without consuming matched elements or otherwise counting multiplicity. For two equal-length multisets that share a repeated element but differ in exactly which element repeats, this returns `true` incorrectly:

- `{a, a}` vs `{a, b}` → both length 2; every element of the first (`a`, `a`) is separately found via `.contains(a)` in the second → reports equal. They are not.
- `{a, a, b}` vs `{a, b, b}` → same failure mode.

Because `Hash` is computed correctly (sorted, multiplicity-sensitive) while `Eq` is not, the `Hash`/`Eq` contract is also violated: these unequal-by-hash-but-equal-by-`eq` pairs would corrupt any `HashSet`/`HashMap` keyed on `MultiAction` or on `ActFrmKind::MultAct(_)` (which derives `PartialEq`/`Hash` through `MultiAction`, e.g. action-formula terms like `<a|a>true` used in modal-formula equality/dedup).

Evidence (both fail on the current tree, and would pass once `eq` is fixed to multiset semantics, e.g. via a sorted-copy comparison or a multiplicity map):

```
$ cargo test -p merc_syntax --test multi_action_test
---- multi_action_eq_distinguishes_repeated_from_distinct_actions stdout ----
thread '...' panicked at crates/syntax/tests/multi_action_test.rs:24:5:
assertion `left != right` failed: {a, a} and {a, b} are different multisets and must not compare equal
  left:  MultiAction { actions: [Action{id:"a",args:[]}, Action{id:"a",args:[]}] }
  right: MultiAction { actions: [Action{id:"a",args:[]}, Action{id:"b",args:[]}] }

---- multi_action_eq_distinguishes_different_multiplicities stdout ----
thread '...' panicked at crates/syntax/tests/multi_action_test.rs:46:5:
assertion `left != right` failed: {a, a, b} and {a, b, b} are different multisets and must not compare equal

test result: FAILED. 0 passed; 2 failed; 0 ignored
```

Direction of fix: sort a clone of each `actions` vector (same ordering `Hash` already uses) before comparing, or build a multiplicity-counting comparison — one line, mirrors the existing `Hash` impl.

### 2. `MultiActionLabel`'s derived `Eq`/`Hash` is order-sensitive, not multiset-aware — PLAUSIBLE, not demonstrated as wrong in this crate

`crates/syntax/src/syntax_tree/specs.rs:266` derives `PartialEq`/`Eq`/`Hash`/`Ord` for `MultiActionLabel { actions: Vec<ActionName> }` structurally (i.e. `[a, b] != [b, a]`), even though it represents the same kind of multiset-of-action-names concept as `MultiAction` (used for allow/block-set entries like `a|b`). This is a different, arguably-also-wrong notion of equality living right next to `MultiAction`'s. I did not find anywhere inside `crates/syntax` that relies on two differently-ordered `MultiActionLabel`s comparing equal (the one place that builds and dedups them, `random_lps.rs:387-399`, explicitly sorts each label's `actions` before `dedup()`, sidestepping the issue), so I'm not able to point at a concrete wrong result from this file alone — flagging it as worth a second look by whoever fixes finding 1, since the two types' Eq semantics for "the same concept" are now inconsistent with each other, but not asserting it is a bug that fires today.

## Checked and found correct

- No `unsafe` in `crates/syntax/src` (`#![forbid(unsafe_code)]` in `lib.rs:2`, confirmed by grep) — no miri/loom/kani needed for this crate.
- `span_offset.rs`: every `OffsetSpans`/`offset_*` function was cross-checked against its node's own enum definition; the recursive descent covers every variant of `DataExprKind`, `ProcessExprKind`, `StateFrmKind`, `ActFrmKind`, `RegFrmKind`, `SortExpressionKind` with no missing arm (all reachable via exhaustive `match`, so a missing case would be a compile error, not a silent span-tracking gap).
- `imports.rs::parse_import_line`'s byte-offset arithmetic (`quote_offset`/`path_start`/`path_end`) is correct: traced through step by step including the interaction between the `rest` rebinding (still quote-prefixed when `quote_offset` is computed) and UTF-8 byte lengths for a non-ASCII path.
- The precedence tables (`*_OPERATORS` constants, lowest level first) in `statefrm.rs`, `actfrm.rs`, and `pbesexpr.rs` were each cross-checked level-by-level against their type's own `Operator::fixity()` impl (used for re-parenthesization on `Display`) — all consistent, no drift between the Pratt-parser table and the fixity table that governs printing.
- `collect_state_frm_spec`/`collect_state_frm_spec_elt` (the function specifically flagged by the health analysis, `specs.rs:886-976`): checked against the actual grammar rule (`StateFrmSpec = SOI ~ ((StateFrmSpecElt* ~ FormSpec ~ StateFrmSpecElt*) | StateFrm) ~ EOI`, `StateFrmSpecElt = SortSpec|ConsSpec|MapSpec|EqnSpec|ActSpec|TypeVarSpec`). The grammar structurally prevents a bare `StateFrm` from co-occurring with `StateFrmSpecElt`s, and the six arms of `collect_state_frm_spec_elt` exactly match the six grammar alternatives of `StateFrmSpecElt`, so the `unimplemented!` fallback is unreachable on any input the grammar accepts. The "multiple formula" double-declaration guard is exercised correctly for both the `StateFrm` and `FormSpec` branches.
- `DataExprKind::Number(String)` stores the literal as an unbounded string rather than parsing to a fixed-width integer, so there is no overflow/truncation risk at the parse layer for arbitrarily large numeric literals.
- `sortexpr.rs::SortProduct`'s `iter.next().unwrap()` is safe: the `SortProduct` grammar rule (`SortExprPrimary ~ (SortExprProduct ~ SortExprPrimary)*`) guarantees at least one child, so this can't panic on any grammar-accepted input.

## Tests added

- `/home/user/merc/crates/syntax/tests/multi_action_test.rs` — two new tests demonstrating the `MultiAction::eq` multiset-equality defect (finding 1):
  - `multi_action_eq_distinguishes_repeated_from_distinct_actions`
  - `multi_action_eq_distinguishes_different_multiplicities`
  - Run with: `cargo test -p merc_syntax --test multi_action_test` (or `cargo nextest run -p merc_syntax multi_action`)
  - Both currently fail (panic on `assert_ne!`); both would pass once `MultiAction::eq` is fixed to multiset semantics. Left in the tree as the regression test for the fix.

No other files were modified; no production code was touched.
