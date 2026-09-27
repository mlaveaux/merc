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
| 3 | Remaining algorithmic crates (`symbolic`, `reduction`, `refinement`, `lts`, `vpg`, `explore`, `data`, `io`, `number`) | done — see [phase-3-algorithmic-core.md](phase-3-algorithmic-core.md) (`symbolic`, `reduction`, `sabre` automaton, `lts`, `graph_symmetry`) and [phase-3-vpg-typecheck-unification-automaton.md](phase-3-vpg-typecheck-unification-automaton.md) (`vpg`, typecheck unification, sabre automaton) |
| — | Random-data-specification fuzzing plan: generate arbitrary struct/container/function sorts, well-typed values of them, and wire that into the PBES/PRES/process generators for more realistic testing | done — `random_data_specification`/`random_value_expression`, `random_pbes_with_data_specification`, `random_pres_with_data_specification`, and `make_process_specification_with_data_specification` all generate and type-check cleanly. Found and fixed two real solver bugs (`solve_join`'s fast path never backtracking; lambda bodies got no widening against their expected range) and two generator bugs (`random_pbes`'s and `random_lps`'s predicate-/process-variable parameters could produce an ill-typed `Pos`/`Nat` value) along the way — see [typecheck-lambda-and-join-widening-bugs.md](typecheck-lambda-and-join-widening-bugs.md) |
| 4 | Tool binaries, GUI, and the remaining foundation/devtools/mCRL2-algorithmic gaps | done — see phase-4-{unsafe-gap,tools-cli,gui,foundation-devtools,mcrl2-logic,kani-extension}.md |
| 5 | Cross-cutting: import cycles, doc audit, coverage gaps | done — see [phase-5-cross-cutting.md](phase-5-cross-cutting.md) |
| 6 | Gaps Phase 5 found: `crates/io`/`crates/refinement` (claimed covered by Phase 3, never actually reviewed), and `tools/mcrl2/crates/mcrl2`'s non-FFI modules + its own `xtask` | done — see phase-6-{io-refinement,mcrl2-misc}.md |
| — | Comment cleanup: trim the extensive `# Safety` comments added during Phase 1/1b to concise ones, verifying each claim first | done — see [comment-cleanup.md](comment-cleanup.md) |

## Findings so far

| Severity | Finding | Location | Status |
|---|---|---|---|
| Critical | `RecursiveLock` write/read aliasing — safe code could manufacture a `&mut T`/`&T` pair, real Stacked Borrows UB | `crates/sharedmutex` | **FIXED**, miri-clean |
| Critical | 3× `unsafe impl Send` let safe code SIGABRT the process | `tools/mcrl2` (merc_pbes, merc_lps) | **FIXED**, compile-fail regression tests added |
| Medium | `crates/syntax`'s shared `Traverse` trait (`SortExpression`/`DataExpr`/`ProcessExpr`/`StateFrm`/`RegFrm`/`ActFrm`/`PbesExpr`/`PresExpr`) recursed natively for `visit_subtree`/`apply_subtree` and everything built on them, so *any* consumer's pre-order walk (not just the checkers below) would SIGABRT on a pathologically deep tree | `crates/syntax/src/traverse.rs` | **FIXED** — `visit_subtree`/`apply_subtree` (and `visit`/`try_visit`/`visit_with`/`visit_children`/`apply`/`apply_mut`/`apply_with`/`apply_children`) now walk an explicit heap-allocated stack instead of the call stack, with zero call-site or behavior changes anywhere (all 18 pre-existing unit tests pass unmodified); proven with two new million-deep regression tests. `transform_children`/`try_transform`/`transform` (bottom-up rewriting) is **not** fixed — a genuinely harder problem for an owned `Box`/`Vec` tree, not just more of the same trick — see [stack-overflow-recursion.md](stack-overflow-recursion.md) |
| High | Unbounded recursion → stack overflow (SIGABRT) on deep input, at least 4 independent instances: `check_state_formula`/`collect_scope` and `resolve_modal_variables` (modal), `check_process_expr` (process), and structurally the same pattern in `pres/check.rs` (untested) | `crates/typecheck` (`modal/check.rs`, `modal/modal_specification.rs`'s `resolve_modal_variables` call, `process/check.rs`, `pres/check.rs`) | **PARTIALLY FIXED** — `modal/check.rs`'s `check_state_formula`/`collect_scope` (and their `check_fixed_point`/`check_reg_formula`/`check_action_formula`/`collect_scope_regfrm`/`collect_scope_actfrm` helpers) are migrated onto `Traverse::visit_scoped`/`try_visit_mixed`, the two API additions this needed; each hand-written `_regfrm`/`_actfrm` recursive cousin is gone, and `modal::check::stack_depth_probe` now passes (its own investigation also reconfirmed the *second*, distinct recursive-`Drop` bug below: the SIGABRT it used to report moved to the test's own end, on the same 100,000-deep `Box` chain, once this fix stopped masking it). `resolve_modal_variables`, `process::check::check_process_expr`, and `pres::check::check_pres_expr` are still hand-written recursive-descent, not yet migrated — not blocked on `Traverse` for `check_process_expr`/`check_pres_expr` (no node-type crossing there), just not done; see [stack-overflow-recursion.md](stack-overflow-recursion.md) for the fuller picture. A stack-size mitigation was also prototyped and shown to work for the checking-phase recursion, but the *second*, distinct bug mentioned above was found underneath it before shipping anything: dropping the resulting deeply-nested `Box`-chained AST itself also overflows via ordinary recursive `Drop` glue, on the original caller's thread, which no stack-size wrapper around the checking call can reach, and remains unfixed for any of this codebase's `Box`-chained ASTs |
| High | `func_update` sort-inference gap: inference-only function sorts get no ground rewrite equations, permanently stuck term | `crates/typecheck/src/lowering/instantiate.rs` | **FIXED** — the inferred-sort top-up pass now also scans for `Function` sorts (previously containers only), filtered to exclude `Id`/`Resolved` name references (a declared `map`/`cons`/`var` sort is already covered elsewhere, and a builtin comparison operator's own per-use-site type, e.g. `==`'s `(S # S) -> Bool`, is never itself a function *value* function-update applies to) so it doesn't also instantiate bogus `@func_update` machinery for an operator's own type |
| High | Real SIGSEGV: release-mode `debug_assert_eq!` compiled out, arity mismatch reads OOB across the FFI | `tools/mcrl2/crates/mcrl2/src/atermpp` | confirmed (structurally + agent's real repro), not yet fixed |
| High | `quotient_lts_block` (branching) mishandles a block with no bottom state: `debug_assert` panics in debug, silently drops transitions in release | `crates/reduction` | confirmed (regression test, `#[ignore]`d), not fixed |
| Medium | Function sorts had no subtyping at all: a `Sub` between `D -> S` and `D -> T` could never succeed, so a lambda body needing sort widening (e.g. a `{ .. }` literal's `FSet`/`FBag` widening to `Set`/`Bag`) or a *named* function reference (no body to push a fix into) failed to type-check | `crates/typecheck/src/inference/unification.rs`, `crates/typecheck/src/inference/resolved_sort.rs`, `crates/typecheck/src/lowering/mcrl2_lowering.rs` | **FIXED** (function sorts now widen covariantly in the range at equal domain, materialized at lowering time by eta-expanding a wrapper around the value — the only construction available, since mCRL2 has no builtin coercion between function values), verified against the mcrl2 oracle-conformance suite (see typecheck-lambda-and-join-widening-bugs.md) |
| Medium | `merc-lts convert` silently rewrote `"tau"` to `"i"` on AutMcrl2 output regardless of requested output format | `tools/lts` | **FIXED** |
| Medium | `zielonka_family_optimised`'s `C_restricted` picked its universe with an inverted `alternative_solving` condition relative to every other call site in the file | `crates/vpg` | **FIXED** |
| Medium | `solve_join`'s fast path never backtracks to the per-source sequential search when its greedy least-upper-bound doesn't satisfy a later constraint (e.g. an enclosing container needing the join wider still) | `crates/typecheck/src/inference/inference.rs` | **FIXED**, verified against the mcrl2 oracle-conformance suite |
| Medium | `MultiAction::eq` is not multiset equality, breaks `Hash`/`Eq` contract | `crates/syntax` | confirmed, not yet fixed |
| Medium | `DataApplication::sort()` silently returns the wrong term (dead code, zero callers) | `tools/mcrl2/crates/mcrl2` | confirmed, deferred (no user impact today) |
| Medium/Plausible | Data race: `Debug for GlobalTermPool` reads protection sets with no coordination against a concurrent `write_exclusive` | `tools/mcrl2/crates/mcrl2/src/atermpp` | confirmed real and reproducible (existing TSan-targeted test `read_races_with_concurrent_term_creation`), not fixed |
| Low/Plausible | `SharedSymbol` relied on undefined `#[repr(Rust)]` field order | `crates/aterm` | **FIXED** (`#[repr(C)]` added) |
| Low | Dead `arity` parameter, discarded and recomputed elsewhere | `crates/aterm` | confirmed, cosmetic |
| Low/Plausible | Two `# Safety` comments cite a false auto-trait justification (`FreeList<Entry<T>>`/`BlockList<T,N>` are never auto-`Send`) for otherwise-plausible manual `Send`/`Sync` impls | `crates/unsafety` | confirmed false citation, outer impls not shown unsound, not fixed |
| Low/Plausible | `StablePointer::ptr()` (safe fn) can read the pointee via `Erasable::unerase` for header-reading `T` (e.g. `SharedTerm`), contradicting its own doc's "never touches the pointee" claim | `crates/unsafety` | confirmed, not fixed |
| High | `OxiddArgs`' `--oxidd-capacity`/`--oxidd-cache-capacity` documented "gigabytes" but passed `(gib as usize) << 30` straight through as a raw oxidd entry count — the default requested ~20-24 real GiB instead of 1, aborting `merc-sym`/`merc-lps explore`/`merc-pbes explore-symbolic\|solve-symbolic` on any machine without tens of GiB free | `crates/symbolic/src/args.rs` | **FIXED**, verified with real unrestricted `merc-sym` runs (not just a unit test) |
| High | `refine_bisimulation` panicked on any real LTS with action variables: its `oxidd_reorder::set_var_order` call omitted them from `order`, letting the reorder insert them between the interleaved p/q variables and break `variable_rename`'s adjacency invariant | `crates/symbolic/src/bdd/refine.rs` | **FIXED** (append action variables to `order`); `test_random_refine_bisimulation` stays `#[ignore]`d — no longer hits this panic, but now needs a larger BDD manager capacity, an unscoped follow-up |
| Medium | `init_console` unconditionally called `AttachConsole` whenever `GetConsoleWindow()` was null, which is also true for a GUI process whose stdout/stderr were already validly redirected by its caller — silently clobbers that redirection | `crates/tools/src/console.rs` | fix applied (decision logic extracted to a tested pure function) but **PLAUSIBLE** only — no Windows runtime in this sandbox to confirm the real Win32 behavior end-to-end |
| Medium | ltsgraph's LTS-reload path updated `viewer`/`graph_layout`/`lts` behind three independent locks; a background render/layout thread could observe a partially-updated combination and index out of bounds on a differently-sized LTS | `tools/gui/ltsgraph`, `tools/gui/ltsgraph-lib` | **FIXED** (merged into one `Mutex<ReloadState>`, plus defensive indexing) |
| Medium | Parallel transitions between the same two states only got fanned apart (distinct render offset) when a back-transition also existed; otherwise they rendered on top of each other | `tools/gui/ltsgraph-lib/src/viewer.rs` | **FIXED** |
| Medium | `Permutation::from_cycle_notation`/`from_mapping_notation` panicked on ordinary fixed-point notation (`"(5)"`, `"[5->5]"`) via an unfiltered identity pair reaching a `debug_assert!`, reachable from `merc-pbes`'s `--generators` CLI flag | `tools/mcrl2/crates/merc_pbes/src/permutation.rs` | **FIXED** |
| Medium | `cargo xtask publish`'s 17-crate list was unpublishable in 3 places: two workspace deps (`merc_typecheck`, `merc_tools`) had no `version =`, and a `merc_ldd` entry named no real package | `crates/xtask/src/publish.rs`, root `Cargo.toml` | **FIXED**; separately found and DEFERRED a pre-existing, out-of-scope crates.io registry-lag issue (local versions ahead of what's published) that blocks a real end-to-end `cargo xtask publish` regardless |
| Medium | `tools/sym`'s `convert`/`reduce` silently did nothing and returned `Ok(())` when `--output` was omitted | `tools/sym/src/main.rs` | **FIXED** (now errors, matching `tools/lts`'s convention) |
| Medium | `tools/sym reduce strong-bisim`'s result was discarded and had no way to be reported under any flag combination | `tools/sym/src/main.rs` | **FIXED** |
| Medium | Root workspace's CI doc-build gate (`RUSTDOCFLAGS="-D warnings" cargo doc`) failed: `crates/sabre` had no `[lints]` section, so its `#[cfg(kani)]` triggered `unexpected_cfgs`, promoted to a hard error | `crates/sabre/Cargo.toml` | **FIXED** |
| Medium | `crates/refinement`'s counterexample construction (`WeakTrace`/`Divergence`/`StableFailures`/`ImpossibleFutures`) recorded raw impl-side tau transitions into the trace, contradicting its own doc that taus are implicit — masked today only because its sole consumer defensively filters tau itself | `crates/refinement/src/failures_refinement.rs` | **FIXED** at the source |
| Medium/Plausible | `control_flow.rs`'s `CfgEdge::target` collapsed "parameter unchanged" (self-loop) and "parameter overwritten with a non-constant expression" (unknown post-state) into the identical `None`, contradicting its own doc; not exploitable by either real consumer today | `tools/mcrl2/crates/mcrl2/src/control_flow.rs` | **FIXED** (new 3-state `CfgTarget` enum) |
| Low/Plausible | `TagIndex`'s `PartialEq<T>`/`PartialOrd<T>` silently bridge across tagged domains via the raw value, overclaiming what its doc guarantees | `crates/utilities/src/tagged_index.rs` | confirmed design gap, not fixed (no live exploit found) |

Both crates reviewed as sound with no defects: `crates/sabre`, `crates/sabre_compiling`. Also reviewed and sound: `crates/collections`, `crates/utilities`, `crates/rec-tests`, `crates/io`, `tools/rewrite`, `tools/vpg` (the latter two have zero automated tests, a coverage gap rather than a defect). `crates/macros`' `unsafe impl Transmutable`/`transmute_lifetime` proven sound with a new Kani proof. New Kani coverage with no defects found also landed in `crates/sharedmutex` (2 harnesses) and `crates/aterm` (1 harness); `crates/sabre_compiling` reconfirmed non-viable for Kani (real FFI/codegen, nothing to model-check).

## Baseline (repowise index, commit `c48abb3`)

- Average health 7.29/10, hotspot-weighted health 5/10 — the files that
  change most and are depended on most are the unhealthy ones.
- `crates/typecheck` is both the worst-health and the highest-churn module.
- Worst file: `crates/typecheck/src/modal/check.rs` (1.0/10, untested, 11
  dependents).
- Highest-leverage fix: `crates/typecheck/src/inference/inference.rs` (6-level
  nesting, 8.8% of the repo's total health gap).
- 19 import cycles in the dependency graph at the original baseline commit.
  Phase 5 re-derived this on the post-Phase-4 tree (repowise was unavailable,
  so with a standalone Tarjan-SCC script instead) and found 7: 4 identical
  `typecheck` driver/data-type pairs (by design, low priority), 1 `vpg` pair
  that's a false positive (a test-only cross-check edge), and 2 genuine but
  mild `tools/mcrl2` two-way type couplings (`cfg_lps`↔`explore_explicit`,
  `cfg_srf`↔`explore_srf`). See [phase-5-cross-cutting.md](phase-5-cross-cutting.md).
- Bus factor 1 on all 554 git-attributed files (systemic, not a per-file
  finding).
- No high-confidence dead code.
- `unsafe` by file count: `tools/mcrl2` 19, `crates/aterm` 14,
  `tools/mcrl2/crates/mcrl2` 13, `crates/unsafety` 10, `crates/sabre` 7,
  `crates/sabre_compiling` 5, `crates/sharedmutex` 3. `crates/collections`
  is `#![forbid(unsafe_code)]`.
- Kani proof harnesses: `crates/unsafety` (`slice_dst.rs`, `freelist.rs`,
  `protection_set.rs`, `index_edge.rs`), `crates/number` (`power_of_two.rs`,
  `bits_for_value.rs`), `crates/macros` (`tests/kani_transmute.rs`, proving
  the derive-generated `Transmutable` impl sound), `crates/sharedmutex`
  (`bf_sharedmutex.rs`, `recursive_lock.rs`), and `crates/aterm`
  (`shared_term.rs`, extended with a two-argument case) — all using the RNG
  harness in `crates/utilities/src/kani_rng.rs`. `crates/sabre`/
  `crates/sabre_compiling` confirmed to have no viable Kani target (real
  FFI/dylib-codegen unsafe, nothing Kani can model-check).

## Layout

Each phase has its own file:

- `phase-0-tooling.md`
- `phase-1-foundation-unsafe-{aterm,sharedmutex-unsafety,sabre}.md`
- `phase-1b-mcrl2-ffi-{atermpp,wrappers}.md`
- `phase-2-{typecheck-inference,typecheck-signature,syntax}.md`
- `phase-3-algorithmic-core.md`, `phase-3-vpg-typecheck-unification-automaton.md`
- `phase-4-unsafe-gap.md` — `crates/macros`' transmute + `crates/tools/console.rs`, plus new Kani coverage
- `phase-4-tools-cli.md` — `tools/rewrite`, `tools/sym`, `tools/vpg`, `merc-lps`/`merc-pbes` CLI mains
- `phase-4-gui.md` — `tools/gui/ltsgraph{,-lib}`
- `phase-4-foundation-devtools.md` — `crates/collections`, `crates/utilities`, `crates/rec-tests`, `crates/xtask`
- `phase-4-mcrl2-logic.md` — `merc_lps`/`merc_pbes` non-FFI algorithmic logic, `mcrl2-macros`
- `phase-4-kani-extension.md` — new Kani harnesses for `crates/sharedmutex`, `crates/aterm`
- `phase-5-cross-cutting.md` — import cycles, doc audit, coverage-gap audit (found the two gaps phase 6 closed)
- `phase-6-io-refinement.md` — `crates/io`, `crates/refinement`
- `phase-6-mcrl2-misc.md` — `tools/mcrl2/crates/mcrl2`'s remaining non-FFI modules, its own `xtask`
- `comment-cleanup.md`, `stack-overflow-recursion.md`,
  `typecheck-lambda-and-join-widening-bugs.md` — cross-cutting follow-up work
  that grew out of the phased review rather than a single phase

Each finding: file:line, failure scenario, verification evidence
(command + output, or the kani proof that encodes it), and status
(CONFIRMED / PLAUSIBLE / FIXED / WITHDRAWN), matching the `review-adversary`
report format.

## Known open items (not part of this review's remaining scope, but tracked)

- `crates/typecheck`'s unbounded-recursion stack overflows (4 instances) and
  the deeper "dropping the resulting AST also overflows" problem underneath
  them — see [stack-overflow-recursion.md](stack-overflow-recursion.md).
- The mCRL2 FFI OOB read (`tools/mcrl2/crates/mcrl2/src/atermpp`),
  `quotient_lts_block`'s no-bottom-state bug (`crates/reduction`),
  `MultiAction::eq`'s non-multiset equality (`crates/syntax`), and the two
  `crates/unsafety` `# Safety`-comment/doc-accuracy issues — all confirmed,
  none yet fixed.
- `test_random_refine_bisimulation` (`crates/symbolic/src/bdd/refine.rs`)
  needs its BDD manager capacity resized (and `quotient_symbolic` re-checked
  against the corrected variable layout) before it can be un-`#[ignore]`d.
- The `tools/mcrl2` workspace's CI doc-build gate was never actually
  evaluated in this sandbox (blocked by disk exhaustion during Phase 5);
  worth a real CI run to confirm it's clean the way the root workspace and
  `tools/gui` now are.
- `crates/tools/src/console.rs`'s `init_console` fix is applied and unit-
  tested but not confirmed against a real Windows runtime.
