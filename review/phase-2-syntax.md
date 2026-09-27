# Phase 2 review: crates/syntax

**Verdict:** `crates/syntax` is not in the unsafe-code list (`grep -rn unsafe crates/syntax/src` finds nothing except `#![forbid(unsafe_code)]` in `lib.rs` — confirmed, this is a pure-safe crate, no miri/loom needed) and its parsing logic (Pratt-parser precedence tables, `specs.rs` collectors, `span_offset.rs`) is largely sound. `MultiAction`'s hand-rolled `PartialEq`/`Hash` in the primary target file (`specs.rs`) was broken: it reported different multisets of actions as equal whenever they shared a repeated element, which also broke the `Hash`/`Eq` contract for anything hashing a `MultiAction` (or an `ActFrmKind::MultAct` built from one). **FIXED** — see finding 1.

## Findings

### 1. `MultiAction::eq` does not implement multiset equality — FIXED

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

**Fix applied** (`crates/syntax/src/syntax_tree/specs.rs:320-334`): `eq` now clones both `actions` vectors, sorts each (`Action` already derives `Ord`, the same total order `Hash` canonicalizes on), and compares the sorted vectors:

```rust
impl PartialEq for MultiAction {
    fn eq(&self, other: &Self) -> bool {
        if self.actions.len() != other.actions.len() {
            return false;
        }

        let mut self_actions = self.actions.clone();
        let mut other_actions = other.actions.clone();
        self_actions.sort();
        other_actions.sort();

        self_actions == other_actions
    }
}
```

This is reflexive/symmetric/transitive because it reduces to `Vec<Action>`'s own (derived, standard) `PartialEq` on a canonical (sorted) form — two multisets are equal iff their canonical forms are structurally equal — and it's now consistent with `Hash`, which sorts the same way before hashing.

Verification:

```
$ cargo test -p merc_syntax --test multi_action_test
running 2 tests
test multi_action_eq_distinguishes_different_multiplicities ... ok
test multi_action_eq_distinguishes_repeated_from_distinct_actions ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

$ cargo test -p merc_syntax
... 41 passed (lib) + 2 passed (multi_action_test) + 21 passed (roundtrip_test) + 1 doctest passed, 0 failed
```

`cargo clippy -p merc_syntax --all-targets` and `cargo +nightly fmt --all -- --check` are both clean on the change (the 2 pre-existing clippy warnings in `random_lps.rs`/`traverse.rs` are unrelated and reproduce identically with the fix reverted).

### 2. `MultiActionLabel`'s derived `Eq`/`Hash` is order-sensitive, not multiset-aware — DEFERRED, confirmed not a live issue anywhere in the repo

`crates/syntax/src/syntax_tree/specs.rs:266` derives `PartialEq`/`Eq`/`Hash`/`Ord` for `MultiActionLabel { actions: Vec<ActionName> }` structurally (i.e. `[a, b] != [b, a]`), even though it represents the same kind of multiset-of-action-names concept as `MultiAction` (used for allow/block-set entries like `a|b`). This is a different, arguably-also-wrong notion of equality living right next to `MultiAction`'s. I did not find anywhere inside `crates/syntax` that relies on two differently-ordered `MultiActionLabel`s comparing equal (the one place that builds and dedups them, `random_lps.rs:387-399`, explicitly sorts each label's `actions` before `dedup()`, sidestepping the issue), so I'm not able to point at a concrete wrong result from this file alone — flagging it as worth a second look by whoever fixes finding 1, since the two types' Eq semantics for "the same concept" are now inconsistent with each other, but not asserting it is a bug that fires today.

**Disposition (implementor pass, following the `MultiAction::eq` fix above):** left unfixed, deferred. Repo-wide grep for `MultiActionLabel` (`grep -rn MultiActionLabel --include='*.rs'`) finds every real consumer of `merc_syntax::MultiActionLabel`:

- `crates/syntax` itself: `parse.rs`, `procexpr.rs`, `random_lps.rs` — the last is the one call site noted above, which sorts each label's `actions` *before* constructing the `MultiActionLabel` (`random_lps.rs:417`, `acts.sort(); acts.dedup();`) and only then sorts/dedups the `Vec<MultiActionLabel>` itself (`random_lps.rs:422-423`), so two differently-ordered-but-same-multiset labels never reach that `dedup()` in the first place.
- `crates/typecheck/src/process/disambiguation.rs:508` — builds only singleton `MultiActionLabel::new(vec![name])`, where order is moot.
- `crates/explore/src/combine.rs` — takes `&[MultiActionLabel]` as input but immediately wraps every element in its own `SortedMultiActionLabel` (defined in that file, `combine.rs:386-395`, itself sorting `actions` in `new`) before any equality-sensitive matching (`is_allowed`, `sorted_allow`); the raw `MultiActionLabel`'s own `Eq`/`Hash`/`Ord` are never invoked for matching there.
- `tools/mcrl2/crates/merc_lps` (`explore_explicit.rs`, `cfg_lps.rs`, `lib.rs`) and `tools/mcrl2/lps/src/main.rs` use `Mcrl2MultiActionLabel`, an unrelated FFI-facing type in `merc_lps` (not `merc_syntax::MultiActionLabel`) — confirmed by checking its definition (`explore_explicit.rs:329`, a distinct `struct Mcrl2MultiActionLabel` wrapping an `ATerm`), so it is out of scope for this finding regardless.

No `HashSet<MultiActionLabel>`/`HashMap<MultiActionLabel, _>` or unguarded `Vec<MultiActionLabel>::sort()`/`dedup()`/direct `==` exists anywhere outside the one already-sorted `random_lps.rs` site. So the design inconsistency is real (two types for "the same concept" now disagree on what equality means), but it has no live wrong-result today. Given the task's directive to keep this patch small and focused (only `MultiAction::eq` was confirmed broken), I left `MultiActionLabel` derived as-is rather than changing its `Eq`/`Hash`/`Ord` speculatively — that change would also need to justify touching `CommExpr`'s derived `Ord` (`specs.rs:383`, which embeds a `MultiActionLabel`) for a currently-unexercised code path, and is better done as its own reviewed change if/when a real consumer needs multiset-aware `MultiActionLabel` equality.

## Checked and found correct

- No `unsafe` in `crates/syntax/src` (`#![forbid(unsafe_code)]` in `lib.rs:2`, confirmed by grep) — no miri/loom/kani needed for this crate.
- `span_offset.rs`: every `OffsetSpans`/`offset_*` function was cross-checked against its node's own enum definition; the recursive descent covers every variant of `DataExprKind`, `ProcessExprKind`, `StateFrmKind`, `ActFrmKind`, `RegFrmKind`, `SortExpressionKind` with no missing arm (all reachable via exhaustive `match`, so a missing case would be a compile error, not a silent span-tracking gap).
- `imports.rs::parse_import_line`'s byte-offset arithmetic (`quote_offset`/`path_start`/`path_end`) is correct: traced through step by step including the interaction between the `rest` rebinding (still quote-prefixed when `quote_offset` is computed) and UTF-8 byte lengths for a non-ASCII path.
- The precedence tables (`*_OPERATORS` constants, lowest level first) in `statefrm.rs`, `actfrm.rs`, and `pbesexpr.rs` were each cross-checked level-by-level against their type's own `Operator::fixity()` impl (used for re-parenthesization on `Display`) — all consistent, no drift between the Pratt-parser table and the fixity table that governs printing.
- `collect_state_frm_spec`/`collect_state_frm_spec_elt` (the function specifically flagged by the health analysis, `specs.rs:886-976`): checked against the actual grammar rule (`StateFrmSpec = SOI ~ ((StateFrmSpecElt* ~ FormSpec ~ StateFrmSpecElt*) | StateFrm) ~ EOI`, `StateFrmSpecElt = SortSpec|ConsSpec|MapSpec|EqnSpec|ActSpec|TypeVarSpec`). The grammar structurally prevents a bare `StateFrm` from co-occurring with `StateFrmSpecElt`s, and the six arms of `collect_state_frm_spec_elt` exactly match the six grammar alternatives of `StateFrmSpecElt`, so the `unimplemented!` fallback is unreachable on any input the grammar accepts. The "multiple formula" double-declaration guard is exercised correctly for both the `StateFrm` and `FormSpec` branches.
- `DataExprKind::Number(String)` stores the literal as an unbounded string rather than parsing to a fixed-width integer, so there is no overflow/truncation risk at the parse layer for arbitrarily large numeric literals.
- `sortexpr.rs::SortProduct`'s `iter.next().unwrap()` is safe: the `SortProduct` grammar rule (`SortExprPrimary ~ (SortExprProduct ~ SortExprPrimary)*`) guarantees at least one child, so this can't panic on any grammar-accepted input.

## Tests added

- `crates/syntax/tests/multi_action_test.rs` — two regression tests for the `MultiAction::eq` multiset-equality defect (finding 1):
  - `multi_action_eq_distinguishes_repeated_from_distinct_actions`
  - `multi_action_eq_distinguishes_different_multiplicities`
  - Run with: `cargo test -p merc_syntax --test multi_action_test` (or `cargo nextest run -p merc_syntax multi_action`)
  - Both failed before the fix (panic on `assert_ne!`) and pass after it. Kept in the tree as the regression test for the fix.

## Fix (implementor pass)

- `crates/syntax/src/syntax_tree/specs.rs` — `MultiAction::eq` rewritten to sort clones of both `actions` vectors (using `Action`'s existing derived `Ord`) and compare the sorted vectors, matching the canonicalization `Hash` already uses. No other production code was touched; `MultiActionLabel` (finding 2) was investigated and left as-is — see finding 2's disposition note above.
- Verified: `cargo test -p merc_syntax --test multi_action_test` (2/2 pass), `cargo test -p merc_syntax` (full suite, 0 failures), `cargo test -p merc_explore` and `cargo test -p merc_data` (dependents, 0 failures), `cargo test -p merc_typecheck -- --skip stack_depth_probe` (0 failures; the skipped test is a pre-existing, unrelated stack-overflow bug tracked in `review/README.md`'s known-open-items and reproduces identically on the unmodified `skills` branch), `cargo clippy -p merc_syntax --all-targets` (clean save for 2 pre-existing unrelated warnings), `cargo +nightly fmt --all -- --check` (clean).
