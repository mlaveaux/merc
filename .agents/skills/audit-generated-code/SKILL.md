---
name: audit-generated-code
description: Find code and documentation in merc that looks machine-generated, unreviewed by a human, or inconsistent with the rest of the repository. Use when asked to audit, clean up, or plan a review pass over a crate or the whole repository, or before accepting a large diff written by an agent.
---

# Auditing for unreviewed or inconsistent code

Authorship is not recoverable from git here, so judge the text. Generated code is not wrong by
being generated; the target is code **nobody has decided to keep**: it does not match its
neighbours, or it explains itself with more words than the code carries. Read the
`architecture` skill first, so that "inconsistent" is measured against how the repo is actually
built.

## Signals

Each is a reason to read, never a finding by itself. Run them per crate (`crates/<group>/<crate>`).

| Signal | Command (from the repo root) | Why it matters |
|---|---|---|
| History-narrating comments | `grep -rniE "//.*\b(previously\|used to\|no longer\|refactor(ed)?\|legacy\|for now\|originally)\b" --include=*.rs <path>` | Comments describe the code as it is; history belongs in the commit message (see the `review` skill). |
| Comments longer than the code | read the largest doc comments in the crate (`traverse.rs`, `rec_tests.rs` are examples) | Contract belongs in rustdoc; reasoning and measurements belong on the website (`documentation` skill). |
| Comments naming private items, tests, or "the same trick used elsewhere in this file" | `grep -rnE "see \`[a-z_]+\` in this module" <path>` and read the hits | They rot when the private item is renamed. |
| Suppressed warnings | `grep -rn "allow(dead_code)\|allow(unused\|expect(dead_code" --include=*.rs <path>` | Dead code is usually a sign nobody used it; delete it or use `#[expect(..., reason = "...")]`. |
| Blanket `unwrap()` outside tests | `grep -rc "\.unwrap()" --include=*.rs <path>/src` | An input-dependent `unwrap` should be a `MercError`; an invariant should be `expect("why it holds")`. |
| `TODO` that is a question | `grep -rnE "TODO\|FIXME\|XXX" --include=*.rs <path>` | "Is this correct?" is unfinished reasoning. Answer with a test or an issue. |
| Duplicate abstractions | search for the same trait name in two crates (`DataExpressionVisitor` exists in `merc_data` and in `mcrl2`) | Two APIs for one idea means one of them was not reviewed against the other. |
| Tests that assert nothing new | tests whose body only constructs a value, or that mirror the implementation line by line | They pass on both sides of a bug (see the `review` skill, Step 2). |
| Style drift | `cargo +nightly fmt --all -- --check`, `cargo clippy --all-targets` | Formatting needs nightly (`imports_granularity = "Item"`). |

## What counts as the repo's own style

Measure against the `architecture` skill, in particular:

- one `use` per item, no glob imports (except `use super::*` in tests);
- errors as `merc_utilities::MercError` or a `thiserror` enum, binaries on `clap` derive with the
  `merc_tools` helpers;
- traversal through `Visit`/`Step`, term rebuilding through `TermBuilder`, never native recursion
  on terms of unbounded depth;
- rustdoc states preconditions, postconditions and panics; crate READMEs carry the architecture;
- `unsafe` blocks have a `// SAFETY:` comment that states the invariant actually upheld.

## Procedure

1. Run the signals on one crate and write the hits next to the file names. Do not edit yet.
2. Read each flagged place in context and decide: keep, rewrite, or delete. Check claims in
   comments against the code (grep the caller, read the invariant) before you keep them.
3. Where the decision is a rewrite, make the smallest change that makes the place match its
   neighbours. Do not restyle unrelated code in the same commit.
4. Run the `check` skill in every workspace the crate feeds (`crates/` is a path dependency of
   `tools/mcrl2` and `tools/gui`).
5. Report per crate: what was flagged, what was kept and why, what was changed. A place you kept
   needs one sentence of reason.

The standing plan with the first scan is in `docs/review-pass.md`.

## Do not

- Treat "sounds like an LLM" as a defect. Fluent prose that is correct and specific is fine.
- Delete a test, an `unsafe` comment or a benchmark because it looks verbose. Run the relevant
  skill (`unsafe-verify`, `benchmark`) and decide on evidence.
- Rewrite the public API of a published `crates/*` crate without checking `cargo semver-checks`.
