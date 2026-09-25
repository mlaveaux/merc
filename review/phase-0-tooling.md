# Phase 0 — Tooling

Goal: match what CI runs locally, so review findings on `unsafe`/concurrent
code are backed by miri/loom/kani/sanitizer evidence, not read-through
opinion (per the `unsafe-verify` skill), and kani proofs can be introduced
into crates that don't have them yet.

## Installed

| Tool | Version | Purpose |
|---|---|---|
| `rustup` nightly toolchain | rustc 1.100.0-nightly (2026-09-24) | miri, nightly fmt, sanitizers |
| `miri` + `rust-src` components (nightly) | — | undefined-behaviour checking for `unsafe` code |
| `cargo-nextest` | 0.9.146 | CI's test runner |
| `cargo-deny` | 0.20.2 | license/dependency policy (`check` skill Step 5) |
| `kani-verifier` (`cargo-kani`, `kani`) | 0.68.0, CBMC 6.11.0 | bounded model-checking proof harnesses |

Sanitizers (ASan/TSan) go through `cargo +nightly xtask address-sanitizer` /
`thread-sanitizer` — no separate install, just the nightly toolchain with
`rust-src` already installed above.

## Validation

Ran the existing kani harnesses rather than trusting the install:

```
cd crates/unsafety && cargo kani
  → 16/16 harnesses SUCCESSFUL (0 of 29 checks failed)
cd crates/number && cargo kani
  → 3/3 harnesses SUCCESSFUL (0 of 8 checks failed)
```

Confirms the toolchain matches what CI expects, on the two crates that
already carry proofs (`crates/unsafety`, `crates/number`, both using the
`[package.metadata.kani]` block and `merc_utilities::kani_rng` harness
pattern).

miri, loom and the sanitizers are exercised per-crate in Phase 1 rather than
smoke-tested here — they're slow and scoped to the code under review.

## Not yet run here

- `miri` / loom / sanitizers themselves (deferred to Phase 1, scoped to the
  crates under review — they're too slow to run workspace-wide as a smoke
  test).
- `cargo +nightly xtask address-sanitizer|thread-sanitizer` end-to-end (needs
  a target crate; also deferred to Phase 1).
