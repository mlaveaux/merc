---
name: unsafe-verify
description: Verify unsafe or concurrent code with miri, loom, kani, and the address/thread sanitizers, matching what merc CI runs. Use when a change touches unsafe blocks, atomics, locks, allocators, or the aterm, sharedmutex, unsafety, or collections crates.
---

# Verify unsafe and concurrent code

Plain tests are not sufficient evidence for `unsafe` or lock-free code in this repository — CI additionally runs miri, loom, kani, and both sanitizers. Run the relevant subset locally before committing; scope to the affected crate with `-p <crate>` because these runs are slow.

Which verifier applies:

- **miri** — any `unsafe` code (undefined behaviour, aliasing, leaks).
- **loom** — concurrency permutations; loom-gated tests live in `crates/sharedmutex` and `crates/unsafety`.
- **sanitizers** — ASan for memory errors, TSan for data races, on the real (non-interpreted) build.
- **kani** — proof harnesses in `crates/unsafety` and `crates/number`.

## Instructions

### Miri

Needs nightly with the `miri` and `rust-src` components (`rustup +nightly component add miri rust-src`):

```bash
MIRIFLAGS="-Zmiri-disable-isolation --cfg chacha20_force_soft" \
  cargo +nightly miri nextest run --no-fail-fast -p <crate>
```

(`cargo +nightly miri test -p <crate>` works without nextest.) Note that CI deliberately does **not** pass `--include-ignored` to miri — long-running ignored tests stay excluded — so do not add it here either. Miri is very slow; do not run it workspace-wide unless asked.

When writing tests for unsafe code, prefer tests that state the postcondition explicitly as an assertion rather than comparing against one memorized output: randomized tests (`merc_utilities::random_test`, seed reproducible via `MERC_SEED=<seed>`) where possible, or small deterministic tests for miri — randomized stress tests are far too slow under miri and are marked `#[cfg_attr(miri, ignore)]`, so give miri its own small tests of the same postcondition (see `crates/aterm/tests/miri_aterm.rs` for the pattern).

### Loom

Loom tests are behind `--cfg loom` and named with `loom`; CI runs them in release mode:

```bash
RUSTFLAGS="--cfg loom" cargo nextest run --no-fail-fast --release -- --include-ignored loom
```

### Sanitizers (via xtask)

The `xtask` crate wraps the RUSTFLAGS/target setup. Needs nightly with `rust-src`:

```bash
cargo +nightly xtask address-sanitizer test --no-fail-fast   # ASan; also run in tools/mcrl2 if affected
cargo +nightly xtask thread-sanitizer test --no-fail-fast    # TSan
```

Everything after `address-sanitizer`/`thread-sanitizer` is passed through to cargo, so `-p <crate>` works.

### Kani

CI proves the harnesses in `crates/unsafety` and `crates/number`. Locally, only if [Kani](https://model-checking.github.io/kani/) is installed; run in the crate directory:

```bash
cd crates/unsafety && cargo kani   # likewise crates/number
```

### Reporting

State explicitly which verifiers were run and which were skipped (and why, e.g. tool not installed) — a green `cargo test` alone does not validate an unsafe change here.
