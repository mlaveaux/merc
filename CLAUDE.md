# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

MERC is a Rust toolset and library collection for model checking — LTS reduction/refinement, term rewriting, symbolic state spaces, and parity games — inspired by mCRL2, LTSmin, and CADP. Tools are thin CLI wrappers; shared logic always lives in library crates under `crates/`.

## Critical invariants

- **Three independent Cargo workspaces**: the root, `tools/mcrl2`, and `tools/gui`. Cargo commands operate on one workspace at a time — run them from the corresponding directory.
- **Tests run with `cargo nextest run`, not `cargo test`** (doc tests separately via `cargo test --doc`).
- **Format with `cargo +nightly fmt --all`** — `rustfmt.toml` uses nightly-only options. Imports are one `use` per item, never grouped with `{}`. No glob imports (`use path::*;`), including `use super::*;` in `#[cfg(test)]` modules — import each item explicitly. Exceptions: `pub use module::*;` re-exports that flatten a crate's own modules (in `lib.rs` or a `mod.rs`), and the `use super::*;` / `pub use inner::*;` scaffolding inside a `#[merc_derive_terms] mod inner` block. Because a flattened (private `mod` + glob re-export) module is not a public doc target, avoid module-level `//!` docs on those — they never render in `cargo doc`, so document the items instead. Only the crate root's `//!`/`#![doc]` and `pub mod` modules render their module docs.
- **Every `unsafe` block needs a `// SAFETY:` comment** explaining the invariant being upheld; unsafe code is concentrated in `merc_unsafety` and checked by miri, sanitizers, loom, and Kani in CI.
- **No new dependencies without clear justification**; `cargo deny` gates licenses.
- Errors use the crate's existing `thiserror`-based type (or `MercError` from `merc_utilities`); CLI tools use `clap` derive with the shared `merc_tools` helpers.

## Common commands

```bash
cargo build [--release]
cargo +nightly fmt --all
cargo clippy --workspace --all-targets --all-features
cargo nextest run --no-fail-fast -- --include-ignored   # full suite, as CI runs it
cargo nextest run -E 'test(name_substring)'             # single test
cargo doc --no-deps
cargo xtask --help        # automation: sanitizers, coverage, packaging, tool tests
```

## Architecture

Everything in the data/rewriting stack is built on `merc_aterm`: a thread-safe global term pool with maximal sharing, so structural equality implies pointer equality and term identity *is* pointer equality; garbage collection tracks live roots via thread-local protection sets. On top sit data expressions and parsing (`merc_data`, `merc_syntax`), the Sabre rewriter (`merc_sabre`, plus a runtime-compiling variant), and the LTS/symbolic layers (`merc_lts`, `merc_explore`, `merc_reduction`, `merc_refinement`, `merc_ldd`/`merc_symbolic`, `merc_vpg`). `TagIndex` newtypes keep state/action/priority indices apart at compile time — never cast between them. The `merc_derive_terms` macro generates term-subtype boilerplate; use it instead of hand-rolling conversions.

## Detailed references

The project skills in `.claude/skills/` hold the detail — load the matching skill rather than re-deriving:

- `architecture` — full crate map, dependency layers, design invariants.
- `build` — build/format/lint/doc commands, xtask automation, Kani verification.
- `testing` — nextest usage, `MERC_SEED` to reproduce randomized tests, `MERC_DUMP` for intermediate artifacts, Kani proof harnesses.
- `conventions` — Rust 2024 idioms, error/CLI patterns, pest grammar test policy.

## Change hygiene

- Prefer small, focused patches that follow nearby patterns; no placeholders or speculative abstractions.
- Add or update tests for bug fixes and new behavior; grammar changes in `merc_syntax` require updating the related pest rule tests.
- Keep documentation concise: explain **why** a choice was made rather than repeating what the code shows. Avoid duplication, excessive linking to other structs, and internal/implementation details.
- Occasional AI-generated boilerplate is acceptable, but slop is strictly forbidden (see `README.md`).
