---
name: check
description: Run the merc validation suite (build, tests, formatting, clippy, cargo deny) the same way CI does, across all three cargo workspaces. Use before committing, after finishing a code change, or when asked to run the tests, lints, or checks.
---

# Validate changes

The repository contains **three independent cargo workspaces**. A change is only fully checked when the affected workspaces all pass:

| Workspace | Directory | Contents |
|---|---|---|
| root | repository root | all `crates/*` and the `merc-lts`, `merc-rewrite`, `merc-sym`, `merc-vpg` tools |
| mCRL2 | `tools/mcrl2` | `merc-lps`, `merc-pbes` and their crates |
| GUI | `tools/gui` | `merc-ltsgraph` (needs `libfontconfig1-dev`, `libfreetype6-dev` on Linux) |

For a change scoped to one crate, run tests for that crate first (`-p <crate>`), then the workspace-wide checks before committing.

## Instructions

### Step 1: Tests

CI uses nextest and includes ignored tests:

```bash
cargo nextest run --no-fail-fast -- --include-ignored
```

If `cargo-nextest` is not installed, either install it (`cargo install cargo-nextest`) or fall back to `cargo test -- --include-ignored`. Repeat inside `tools/mcrl2` and `tools/gui` when those workspaces are affected. When debugging a failure, rerun with `RUST_BACKTRACE=full RUST_LOG=debug`.

CI additionally runs the same suite in **release mode** with `RUST_MIN_STACK=104857600` — release builds disable `debug_assertions` and overflow checks, so a debug-only pass can still fail there. Run `RUST_MIN_STACK=104857600 cargo nextest run --release --no-fail-fast -- --include-ignored` when a change touches arithmetic, assertions, or deep recursion.

### Step 2: Formatting

Formatting **requires nightly rustfmt** — `rustfmt.toml` uses `imports_granularity = "Item"`, which stable rustfmt silently ignores:

```bash
cargo +nightly fmt --all               # apply
cargo +nightly fmt --all -- --check    # what CI verifies
```

Run in each affected workspace directory (rustfmt is per-workspace).

### Step 3: Clippy

```bash
cargo clippy --all-targets
```

CI fails on clippy warnings — fix them rather than `#[allow]`-ing them; run in each affected workspace.

### Step 4: Documentation (when doc comments or public APIs changed)

CI runs doc tests and builds documentation with rustdoc warnings promoted to errors:

```bash
cargo test --doc
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

Broken intra-doc links or malformed doc comments fail CI even when everything else passes.

### Step 5: Dependency policy (only when Cargo.toml changed)

License and dependency checks are enforced with `cargo deny` (config in `deny.toml`, one per workspace):

```bash
cargo deny check
```

### Additional CI-only gates to keep in mind

- MSRV is pinned (`rust-version` in `Cargo.toml`); avoid features newer than it.
- `cargo semver-checks` runs on published crates — breaking a public API of a `crates/*` crate requires a version bump.
- Tests that compare against the upstream mCRL2 toolset read `MCRL2_PATH` (pointing at mCRL2's `build/stage/bin`) and skip when it is unset — a locally green run does not cover them. CI runs them with the filter `mcrl2`.
- Changes to `unsafe` or concurrent code need the deeper verification described in the `unsafe-verify` skill (miri, loom, sanitizers, kani).
