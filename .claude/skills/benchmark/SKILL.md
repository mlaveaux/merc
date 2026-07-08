---
name: benchmark
description: Build and run the criterion benchmarks in the crates/*/benchmarks crates, and compare performance before and after a change using saved baselines. Use when asked to benchmark, measure performance, or check for regressions.
---

# Benchmarks

Benchmarks are criterion-based and live in dedicated sub-crates so they don't burden normal builds:

| Package | Measures |
|---|---|
| `benchmarks_aterm` | term library (`crates/aterm`) |
| `benchmarks_sabre` | Sabre rewriter (`crates/sabre`) |
| `benchmarks_sharedmutex` | `bf_sharedmutex` and lock implementations |
| `benchmarks_unsafety` | sharded hashmap and other unsafe primitives |
| `benchmarks_utilities` | counters and utility types |

The GUI workspace (`tools/gui`) has its own benchmarks; run those from that directory.

## Instructions

### Step 1: Pick scope

Never run all benchmarks for a scoped change — a full run takes very long. Select the package matching the changed crate:

```bash
cargo bench -p benchmarks_sharedmutex
# narrow further to one criterion filter:
cargo bench -p benchmarks_sharedmutex -- <name-substring>
```

CI only verifies benchmarks compile; you can do the same quickly with `cargo bench --no-run`.

### Step 2: Compare against a baseline for before/after claims

A single run proves nothing. Save a baseline on the **unmodified** code, then compare. `git stash` only sets aside uncommitted changes — if the change is already committed, check out the base commit (`git switch main` or `git checkout <base>`) for the baseline run instead:

```bash
git stash                                             # uncommitted change; else: git switch main
cargo bench -p <package> -- --save-baseline before
git stash pop                                         # else: git switch -
cargo bench -p <package> -- --baseline before
```

Criterion prints the relative change and whether it is statistically significant. HTML reports land in `target/criterion/`. Make sure both runs use the same benchmark filter and machine conditions (no parallel builds or tests running).

### Step 3: Report honestly

Quote criterion's own change estimate and significance verdict, not just the raw times. Mention that results from a shared/dev machine (codespace) are noisy — treat small differences (< ~5%) as noise unless criterion marks them significant across runs.
