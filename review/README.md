# merc codebase review

Ongoing detailed review of the merc workspace (20 `merc_*` crates + three
cargo workspaces), driven by the `review` / `review-adversary` / `implementor`
skills, the repowise and rust-analyzer MCP tools, and — for `unsafe` and
concurrent code — miri/loom/kani/sanitizers per the `unsafe-verify` skill.

Findings are risk-ordered, not file-ordered: foundation and `unsafe` crates
first (everything else depends on them), then the highest-churn/lowest-health
application code, then the rest. See the baseline data and phase breakdown
below.

## Status

| Phase | Scope | Status |
|---|---|---|
| 0 | Tooling: kani, miri, loom, sanitizers, nextest, cargo-deny | done — see [phase-0-tooling.md](phase-0-tooling.md) |
| 1 | Foundation/unsafe: `aterm`, `sharedmutex`, `unsafety`, `sabre`, `sabre_compiling` | done — see phase-1-foundation-unsafe-{aterm,sharedmutex-unsafety,sabre}.md |
| 1b | mcrl2 FFI boundary (`tools/mcrl2/crates/mcrl2`) — miri boundary tests + written contracts only, kani doesn't model-check across the C++ FFI boundary | done — see phase-1b-mcrl2-ffi-{atermpp,wrappers}.md |
| 2 | `typecheck` / `syntax` core (highest churn + lowest health) | done — see phase-2-typecheck-{inference,signature}.md, phase-2-syntax.md |
| 3 | Remaining algorithmic crates (`symbolic`, `reduction`, `refinement`, `lts`, `vpg`, `explore`, `data`, `io`, `number`) | not started |
| 4 | Tool binaries & GUI, three-workspace boundary check | not started |
| 5 | Cross-cutting: import cycles, doc audit, coverage gaps | not started |

## Findings so far

| Severity | Finding | Location | Status |
|---|---|---|---|
| Critical | `RecursiveLock` write/read aliasing — safe code could manufacture a `&mut T`/`&T` pair, real Stacked Borrows UB | `crates/sharedmutex` | **FIXED**, miri-clean |
| Critical | 3× `unsafe impl Send` let safe code SIGABRT the process | `tools/mcrl2` (merc_pbes, merc_lps) | **FIXED**, compile-fail regression tests added |
| High | Unbounded recursion → stack overflow (SIGABRT) on deep input, 3 independent instances | `crates/typecheck` (`modal/check.rs`, `process/check.rs`, structurally identical pattern also in `pres/check.rs`, untested) | confirmed, not yet fixed |
| High | `func_update` sort-inference gap: inference-only function sorts get no ground rewrite equations, permanently stuck term | `crates/typecheck/src/lowering/instantiate.rs` | confirmed, not yet fixed |
| High | Real SIGSEGV: release-mode `debug_assert_eq!` compiled out, arity mismatch reads OOB across the FFI | `tools/mcrl2/crates/mcrl2/src/atermpp` | confirmed (structurally + agent's real repro), not yet fixed |
| Medium | `MultiAction::eq` is not multiset equality, breaks `Hash`/`Eq` contract | `crates/syntax` | confirmed, not yet fixed |
| Medium | `DataApplication::sort()` silently returns the wrong term (dead code, zero callers) | `tools/mcrl2/crates/mcrl2` | confirmed, deferred (no user impact today) |
| Low/Plausible | `SharedSymbol` relied on undefined `#[repr(Rust)]` field order | `crates/aterm` | **FIXED** (`#[repr(C)]` added) |
| Low | Dead `arity` parameter, discarded and recomputed elsewhere | `crates/aterm` | confirmed, cosmetic |
| Plausible | Data race: `Debug for GlobalTermPool` reads protection sets with no coordination against a concurrent `write_exclusive` | `tools/mcrl2/crates/mcrl2/src/atermpp` | confirmed by source tracing, unreachable in practice (no current callers), not fixed |

Both crates reviewed as sound with no defects: `crates/sabre`, `crates/sabre_compiling`.

## Baseline (repowise index, commit `c48abb3`)

- Average health 7.29/10, hotspot-weighted health 5/10 — the files that
  change most and are depended on most are the unhealthy ones.
- `crates/typecheck` is both the worst-health and the highest-churn module.
- Worst file: `crates/typecheck/src/modal/check.rs` (1.0/10, untested, 11
  dependents).
- Highest-leverage fix: `crates/typecheck/src/inference/inference.rs` (6-level
  nesting, 8.8% of the repo's total health gap).
- 19 import cycles in the dependency graph.
- Bus factor 1 on all 554 git-attributed files (systemic, not a per-file
  finding).
- No high-confidence dead code.
- `unsafe` by file count: `tools/mcrl2` 19, `crates/aterm` 14,
  `tools/mcrl2/crates/mcrl2` 13, `crates/unsafety` 10, `crates/sabre` 7,
  `crates/sabre_compiling` 5, `crates/sharedmutex` 3. `crates/collections`
  is `#![forbid(unsafe_code)]`.
- Existing kani proof harnesses: `crates/unsafety` (`slice_dst.rs`,
  `freelist.rs`, `protection_set.rs`, `index_edge.rs`) and `crates/number`
  (`power_of_two.rs`, `bits_for_value.rs`), using the RNG harness in
  `crates/utilities/src/kani_rng.rs`.

## Layout

Each phase gets its own file once work on it starts:

- `phase-0-tooling.md`
- `phase-1-foundation-unsafe.md`
- `phase-2-typecheck-syntax.md`
- `phase-3-algorithmic-core.md`
- `phase-4-tools-gui.md`
- `phase-5-cross-cutting.md`
- `SUMMARY.md` — final ranked findings across all phases (written last)

Each finding: file:line, failure scenario, verification evidence
(command + output, or the kani proof that encodes it), and status
(CONFIRMED / PLAUSIBLE / FIXED / WITHDRAWN), matching the `review-adversary`
report format.
