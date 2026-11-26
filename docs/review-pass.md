# Review pass: unreviewed or inconsistent generated code

A working plan. Once the findings are handled, delete this file and keep only what generalises in
the `audit-generated-code` skill.

The repository has about 150k lines of Rust, all committed by the maintainer, so authorship says
nothing about what a human read. The signals below come from the code itself. A signal is a reason
to read a place, not a verdict: only a human decision (keep, rewrite, delete) closes an item.

## Candidates found by the first scan

| Area | Signal | Action |
|---|---|---|
| `tools/mcrl2/crates/mcrl2/src/visitor.rs` vs `crates/rewriter/data/src/visitor.rs`, `crates/language/pbes/src/visitor.rs` | Two generations of visitor under the same names. The mCRL2 `DataExpressionVisitor` returns `Option<DataExpression>` and has no early exit or context; the root-workspace one returns `Visit` and threads a `Copy` context. The mCRL2 file also holds a third style (`*ContextVisitor`, `ClosureVisitor`). | Decide whether the mCRL2 wrappers move to `Visit`/`Step`; at least rename or document the difference. |
| `crates/language/syntax/src/traverse.rs` (1673 lines) | Doc comments cross-reference tests and sibling code in prose ("the same trick every other stack-based walk in this file uses", `stack_depth_probe`); very long comments on small functions. | Human read for correctness, then shorten comments to the contract. |
| `crates/rewriter/rec-tests/tests/rec_tests.rs` | A multi-paragraph `TODO` about `State::derive_transition` and a test named `..._todo_stepcount_regression`; names a private function that can be renamed away. | Turn into a tracked issue; rename the test; drop the private function name. |
| `tools/mcrl2/crates/mcrl2/src/data.rs` | `TODO` plus `#[expect(dead_code)]` placeholder type `Mcrl2RewriterJitty`. | Delete or implement. |
| `#[allow(dead_code)]` / `#[allow(unused)]` (61) | Concentrated in `explore/src/discovered_set.rs` (9), `typecheck/src/inference/resolved_sort.rs` (6), `lowering/src/lowering.rs` (5), `aterm/src/storage/symbol_pool.rs` (5). | Delete unused code, or turn the allow into `#[expect(..., reason = "...")]`. |
| History-narrating comments (182 matches for "previously", "used to", "no longer", "refactor", "legacy", "for now") | Top files: `unsafety/src/block_allocator.rs`, `symbolic/src/bdd/symbolic_lts_bdd.rs`, `syntax/tests/roundtrip_test.rs`, `merc_pbes/src/explore_srf.rs`, `typecheck/src/instantiate.rs`. Many hits are false positives. | Triage by hand; rewrite to describe the current code. |
| `.unwrap()` in non-test code (1227 in total) | Highest in `typecheck/.../standard_sorts.rs` (35), `merc_pbes/.../graph_symmetry.rs` (34), `symbolic/.../summand_grouping.rs` (30), `sharedmutex/.../recursive_lock.rs` (29). | Where the invariant is real, `expect("why")`; where it is input-dependent, return `MercError`. |
| Two stale TODO styles (23 `TODO`s) | Several are questions ("Is this correct, now a|b matches a?" in `lts/src/multi_action.rs`, "Verify support correctness" in `symbolic/src/util.rs`) that look like unfinished reasoning, not plans. | Answer with a test or open an issue. |
| `.agents/skills/review/SKILL.md` | Referenced `rust-analyzer` and `repowise` MCP servers that no repository config declares. | Fixed in this pass: the paragraph now says "when available". |
| `.agents/skills/documentation/SKILL.md` | Depends on `~/merc-website` and `~/merc-verified`, which only exist on the maintainer's machine. | Fixed in this pass: says what to do when they are absent. |

## Passes, in order

1. **Mechanical triage** (`audit-generated-code` skill, "Signals"): run the greps, record hits per
   file in the table above. No edits.
2. **Per-crate read, core first.** Order: `core/unsafety`, `core/sharedmutex`, `core/aterm`,
   `core/utilities`, `rewriter/data`, `rewriter/sabre`, `language/syntax`, then the rest. The
   early crates carry the safety invariants everything else relies on, so inconsistencies there
   are the expensive ones. For each crate, one reviewer, one sitting, using the `review` skill;
   for `unsafe` also the `unsafe-verify` skill.
3. **Cross-crate consistency**, which the per-crate read cannot see: visitors and traversal
   (above), error types (`MercError` vs ad hoc `thiserror` enums vs `Box<dyn Error>`), CLI
   structure (`clap` derive plus `merc_tools` helpers in every binary), naming of
   `*Ref` / owned pairs, test style (`random_test` vs hard-coded examples).
4. **Verification.** Anything a human decides to keep must still pass the `check` skill in all
   three workspaces. Deleting dead code can break `tools/mcrl2` or `tools/gui`, which are only
   built from their own directories.
5. **Close out.** Fix in small commits grouped by decision, not by file. Delete this plan.

## Definition of done per item

A named human has read it and either changed it or can say in one sentence why it is right as it
is. "Looks fine" without a reason is not a close.
