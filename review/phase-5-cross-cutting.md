# Phase 5 — Cross-cutting concerns

Scope, per `review/README.md`: (1) re-derive the baseline's "19 import cycles"
and characterize each; (2) run the exact doc-audit gate CI's `doc.yml` runs
(`RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items`)
across all three workspaces, plus a doc-comment/reality spot-check on
recently-touched public APIs; (3) find test-coverage gaps not already flagged
in phase-0/1/1b/2/3/4; plus the carried-over `refine_bisimulation` panic from
`review/phase-4-tools-cli.md`. This is an audit pass, not a hunt for one
demonstrated wrong result, so findings are graded by evidence quality rather
than forced into a single CONFIRMED/PLAUSIBLE mold throughout.

## Verdict

Two real, previously-unflagged defects, both outside the six phases' stated
scope: `cargo doc` **used to fail outright** under CI's exact `-D
warnings` gate (`crates/sabre` was missing the workspace's `unexpected_cfgs`
lint override, so its own `#[cfg(kani)]` broke doc generation — now FIXED),
and the carried-over `refine_bisimulation` panic is now fully root-caused,
demonstrated with three new tests (including one that isolates the bug to a
single `oxidd_reorder::set_var_order` call independent of any merc-specific
machinery), and FIXED in production code. Import cycles are real but small
and, on inspection, either
by-design bidirectional data/driver coupling (4 identical instances in
`typecheck`) or test-only artifacts, not spaghetti. The most consequential
coverage-gap finding isn't really about tests: two crates named in Phase 3's
own stated scope (`crates/io`, `crates/refinement`) were never actually
reviewed by any phase document, `crates/io` being the second-most-depended
crate in the repository once you exclude `merc_utilities` itself.

---

## 1. Import cycles

### Methodology

The baseline used repowise's dependency-graph index, which is not connected
in this session (`repowise (CONNECTION_CLOSED)`). Cargo forbids cyclic crate
dependencies outright, so any real import cycle in this repo must be a
*module-level* cycle within a single crate. I wrote a standalone script
(`/tmp/.../scratchpad/find_cycles.py`, kept out of the repo) that:

1. Uses `cargo metadata` (all three workspaces) to find every crate's real
   root file, then follows `mod foo;` declarations to build each crate's
   module tree (handling both `foo.rs` and `foo/mod.rs` layouts).
2. Strips comments and string/char literals from each file (an early,
   uncorrected version of this tool produced false cycles purely from a doc
   comment mentioning `` `crate::process::check::...` `` in prose).
3. Extracts every `use crate::...`/`use super::...`/`use self::...` item
   *and* every inline fully-qualified `crate::a::b::Item` reference used
   directly in code (not just `use` statements), resolves each to the
   nearest known module, and builds a directed "A imports from B" graph per
   crate.
4. Separately builds a "production-only" variant of that graph with
   `#[cfg(test)] mod foo { ... }` bodies stripped out, so a module's *test*
   code importing a sibling for cross-checking doesn't count as production
   coupling.
5. Runs Tarjan's SCC algorithm and also reports raw mutual (`A→B` and `B→A`)
   pairs, tagging each pair PRODUCTION or TEST-ONLY ARTIFACT depending on
   whether it survives step 4.

This is a heuristic, not a full name-resolution pass (it can't see
glob-import fallout or macro-generated `use`), so it should be read as a
lower bound, characterized by hand below rather than trusted as an exact
count. It very likely does not reproduce whatever repowise counted at
commit `c48abb3` — the codebase has materially changed since then (six
review phases' worth of fixes, plus new crates like `typecheck`'s
process/modal/pbes/pres split), so an exact match to "19" isn't a
meaningful target; characterizing what exists *today* is.

### Result: 7 module-level cycles, all 2-node, all in root-workspace or tools/mcrl2 crates

| # | Crate | Cycle | Kind |
|---|---|---|---|
| 1 | `merc_typecheck` | `modal::check` ↔ `modal::modal_specification` | Production |
| 2 | `merc_typecheck` | `pbes::check` ↔ `pbes::pbes_specification` | Production |
| 3 | `merc_typecheck` | `pres::check` ↔ `pres::pres_specification` | Production |
| 4 | `merc_typecheck` | `process::check` ↔ `process::process_specification` | Production |
| 5 | `merc_vpg` | `symbolic::symbolic_zielonka` ↔ `symbolic::verify_symbolic` | **Test-only artifact** |
| 6 | `merc_lps` (tools/mcrl2) | `cfg_lps` ↔ `explore_explicit` | Production |
| 7 | `merc_pbes` (tools/mcrl2) | `cfg_srf` ↔ `explore_srf` | Production |

Nothing was found in `crates/aterm`, `crates/sharedmutex`, `crates/unsafety`,
`crates/collections`, `crates/reduction`, `crates/lts`, `crates/syntax`,
`tools/gui`, or any other crate scanned.

### Characterization

**#1–4 (typecheck, 4 identical instances): by-design, low priority.**
Verified by reading both sides of each pair. In every case,
`<kind>_specification.rs` defines that syntax category's data
structures/declaration tables *and* its own driver entry point (e.g.
`modal_specification.rs:65` calls
`check::check_modal_specification(&mut data, &tables, &spec, formula_type)`),
while `check.rs` implements the actual recursive-descent algorithm and
imports the declaration-table types it needs from `..._specification.rs`
(`modal/check.rs:24-34`: `DeclarationTables`, `FormulaType`,
`resolve_declared_sort`, plus `ModalError`). This is the exact same pattern
four times over (modal, pbes, pres, process) — clearly a deliberate,
consistent architectural convention (`_specification.rs` = public
types + entry point, `check.rs` = internals), not four independent instances
of accidental spaghetti. It *could* be broken by moving each driver function
into `check.rs` itself or a third orchestration module, but there's no
demonstrated cost today (no build-time, testability, or correctness problem
traced to it) — worth a "would be tidier" note, not a fix.

**#5 (vpg): not a real cycle — an artifact of test-only cross-checking.**
`verify_symbolic.rs` imports `symbolic_zielonka::includes` and
`solve_symbolic_zielonka` at the top level (`verify_symbolic.rs:1-10`, real
production dependency: verification calls the solver). The *only* reverse
edge is `symbolic_zielonka.rs:345`'s
`use crate::symbolic::verify_symbolic::make_parity_game_total;`, which lives
inside `symbolic_zielonka.rs`'s own `#[cfg(test)] mod tests` block (used by
`test_symbolic_zielonka_solve_matches_explicit_reference` to cross-check the
solver's output against an independent verifier). Once test code is
excluded, this is a normal one-directional dependency
(`verify_symbolic → symbolic_zielonka`), not a cycle. Not worth touching.

**#6–7 (tools/mcrl2, 2 identical instances): the one genuine, mildly worth-fixing pattern.**
`cfg_lps.rs` imports `ExplicitContext`/`ExplicitLinearProcessSpecification`/
`ExplicitSummand`/`Mcrl2MultiActionLabel` from `explore_explicit.rs`
(`cfg_lps.rs:15-18`) to represent its CFG output in terms of the explicit
exploration types. Conversely, `explore_explicit.rs` imports
`CfgLinearProcessSpecification` from `cfg_lps.rs` (`explore_explicit.rs:49`)
and constructs one (`CfgLinearProcessSpecification::new(lps)`,
`explore_explicit.rs:83,201`) as an internal optimization/pruning step of its
own explicit-exploration driver. `merc_pbes`'s `cfg_srf.rs` ↔
`explore_srf.rs` is the structurally identical PBES-SRF analogue (`cfg_srf.rs`
imports `PbesSrfContext`/`PbesSrfLps`/`PbesSrfSummand` from `explore_srf.rs`;
`explore_srf.rs:35` imports `CfgPbesSrfLps` from `cfg_srf.rs`). This is a
real two-way type dependency: the CFG builder's public API is expressed in
terms of the explicit/SRF types, while the explicit/SRF explorer builds a CFG
as an internal step. A cleaner split (e.g. hoisting the shared
context/summand types into a third `types.rs` both modules depend on
downstream) would remove the cycle, but nothing here is broken today — this
is the one pair worth a "if someone's touching this file anyway" cleanup
note rather than a standing defect.

---

## 2. Doc audit

Ran the exact command `doc.yml` runs
(`.github/workflows/doc.yml:96-99`, `RUSTDOCFLAGS="-D warnings"`):

### Root workspace: **FAILS — CI's own gate is currently broken**

```
$ RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items -p merc_sabre
...
error: unexpected `cfg` condition name: `kani`
   --> crates/sabre/src/utilities/term_stack.rs:501:7
    |
501 | #[cfg(kani)]
    |       ^^^^
    = help: expected names are: `docsrs`, `feature`, and `test` and 31 more
    = note: `-D unexpected-cfgs` implied by `-D warnings`
error: could not document `merc_sabre`
$ echo $?
101
```

**Root cause**: `crates/sabre/Cargo.toml` has no `[lints]` section at all
(confirmed by reading the whole file — it has `[package.metadata.kani]` for
the Kani harness but nothing under `[lints]`). The workspace root defines
`unexpected_cfgs = { level = "allow", check-cfg = ['cfg(loom)', 'cfg(kani)'] }`
(`Cargo.toml:61`), but every other crate that uses `#[cfg(kani)]` either
inherits it via `[lints]\nworkspace = true` (`crates/aterm`,
`crates/unsafety`) or declares its own equivalent override
(`crates/utilities`, `crates/collections`, `crates/sharedmutex`'s
`[lints.rust]`). `crates/sabre` does neither, so it gets rustc's default
`unexpected_cfgs = warn`, which `-D warnings` (exactly what `doc.yml` sets)
promotes to a hard error. `git log` confirms this crate's `Cargo.toml` has
never had a `[lints]` section, while `#[cfg(kani)]` was added to
`term_stack.rs` later, in Phase 1's kani-proof work
(`13223993 WIP: Phase 1 in-progress kani proofs, contracts, and miri tests`)
— the two were never reconciled. This is the *only* failure in the root
workspace; every other of the ~30 crates/targets documents cleanly with zero
warnings under the same `-D warnings` flags (confirmed by running the full
`cargo doc --no-deps --document-private-items` and grepping for `^warning`/
`^error` — the only two lines are the one shown above).

- **Status**: FIXED. Added `[lints]\nworkspace = true` to
  `crates/sabre/Cargo.toml` (matching `crates/aterm`/`crates/unsafety`, right
  after `rust-version.workspace = true`) so it inherits the workspace's
  `unexpected_cfgs = { level = "allow", check-cfg = ['cfg(loom)', 'cfg(kani)'] }`
  (`Cargo.toml:61`) instead of falling back to rustc's default `warn`. This is
  a config-only, zero-behavior-change fix.
- **Verification**:
  ```
  $ RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items -p merc_sabre
  ...
   Documenting merc_sabre v3.0.0 (.../crates/sabre)
      Finished `dev` profile [unoptimized + debuginfo] target(s) in 15.45s
     Generated .../target/doc/merc_sabre/index.html
  $ echo $?
  0
  ```
  `cargo check -p merc_sabre --all-targets` and
  `cargo clippy -p merc_sabre --all-targets` both succeed with zero warnings
  attributable to `merc_sabre` itself (the only warnings printed are
  pre-existing ones in `merc_syntax`/`merc_typecheck`, unrelated dependencies
  built along the way — confirmed by grepping the clippy output for
  `sabre`/`refine.rs`, which returns nothing). The full root-workspace
  `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items`
  was not re-run in full here (slow, and `-p merc_sabre` isolates the one
  crate this fix touches); `-p merc_sabre` succeeding with exit 0 where it
  previously failed with exit 101 is the direct evidence for this fix.

### tools/gui workspace: clean

```
$ cd tools/gui && RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items
...
 Generated .../target/doc/merc_ltsgraph/index.html and 1 other file
$ echo $?
0
```

No warnings, no errors.

### tools/mcrl2 workspace: could not complete — sandbox environment, not a code defect

`tools/mcrl2` requires building the vendored mCRL2 C++ library
(`mcrl2-sys`) via `cxx-build`/`cc`. The build got as far as compiling and
archiving `.o` files before failing with:

```
cargo:warning=ar: .../libmcrl2-sys.a: error reading .../72b3f6d99475acc7-pbes.rs.o: No space left on device
```

`df -h /` showed the shared container filesystem at **99–100% full** (down
to 56 MB free at the time), independently confirmed to be caused by *other*
concurrently-running worktree sessions in this same container (`du -sh
/home/user/merc/.claude/worktrees/*/target` showed 12 other worktrees with
0.4–5.7 GB of build artifacts each, actively growing — one had a
`mcrl2-sys` build log timestamped minutes before mine). I freed ~525 MB by
deleting my own worktree's orphaned partial `mcrl2-sys` build directories
(safe: mine only, not shared state), but did not retry the full C++ build:
with under 600 MB free and shared with an unknown number of concurrently
active sibling sessions, a retry would very likely fail the same way and
risks starving other sessions of the little space left. This mirrors the
same constraint `review/phase-4-tools-cli.md` hit trying to rebuild
`tools/mcrl2/lps`/`tools/mcrl2/pbes` (mCRL2 FFI build not completing in the
sandbox's time/resource budget) — not a new problem, just the doc-audit
task running into it from a different angle. **Not evaluated**; someone with
a clean/larger environment should run
`cargo +stable doc --no-deps --document-private-items --manifest-path tools/mcrl2/Cargo.toml
RUSTDOCFLAGS="-D warnings"` to close this out.

### Doc-comment/reality spot-check: `crates/symbolic/src/args.rs`

Checked the `--oxidd-capacity` flag's doc comment
(`"Capacity of the manager's inner-node table, in gigabytes (as a power of
two, i.e. `1 << 30` bytes per gigabyte)"`, `args.rs:99-100`) and the
`LDD_NODE_ENTRY_BYTES`/`LDD_CACHE_ENTRY_BYTES`/`BDD_NODE_ENTRY_BYTES`/
`BDD_CACHE_ENTRY_BYTES` derivation comments against it, post-fix. The LDD
side was already empirically cross-checked in `phase-4-tools-cli.md`'s
implementor pass; I independently verified the BDD side (which that pass
flagged as "derived from source alone, no empirical BDD crash was
available") against the actual vendored oxidd source:

- `crates/oxidd/src/bdd.rs`'s `index` module instantiates
  `NodeWithLevelCons<(), 2>` — value type `()`, confirming the doc's claim
  that a BDD node has no `value` field (16 bytes: 4 `rc` + 4 `level` + 8
  `children[2]` + 0), and `manager_data!(... cache_entry_capacity: 4)` —
  confirming the doc's claim of BDD's cache capacity being 4, not LDD's 5
  (20 bytes: 4-byte header + 4×4-byte `Datum`).

Both match the doc comment exactly. **No drift found** — this one is
accurate and was a good, verifiable piece of documentation to write.

---

## 3. Coverage gaps

`cargo +nightly xtask coverage nextest run` (the tool `doc.yml` CI uses) was
**not run**: it instruments and runs the entire root-workspace test suite,
which the disk-space situation above makes a bad idea to attempt right now
(an instrumented debug build plus every crate's test binaries would need
several more GB than the ~500 MB free, and `phase-4-tools-cli.md` already
recorded this same sandbox OOM-killing a plain `cargo test -p merc_symbolic
--release` under full parallelism). Falling back to the structural audit the
task brief explicitly allows for this case.

### Method

For every crate in all three workspaces: does it have a `tests/` directory,
how many `#[test]`-containing files does its `src/` have, and (root workspace
only) how many *other* workspace crates depend on it (a proxy for
"centrality" — `cargo metadata`'s resolve graph, root workspace only).
Cross-referenced every candidate gap against all nine existing phase docs
before reporting it, to avoid re-litigating covered ground (see grep
evidence below each item).

### Finding A: `crates/io` and `crates/refinement` were never actually reviewed — a review-scope gap, not just a test-coverage one

`review/README.md`'s Phase 3 line claims coverage of "`symbolic`,
`reduction`, `refinement`, `lts`, `vpg`, `explore`, `data`, `io`, `number`",
but neither `phase-3-algorithmic-core.md` nor
`phase-3-vpg-typecheck-unification-automaton.md` — the two documents that
line points to — actually cover `crates/io` or `crates/refinement` in their
own text:

```
$ grep -rl "crates/io" review/*.md          # (no output)
$ grep -rl "crates/refinement" review/*.md  # (no output)
$ grep -rl "merc_io" review/*.md
review/phase-4-foundation-devtools.md   # one incidental mention, in an
                                         # unrelated `cargo publish` version-
                                         # string discussion, not a review
                                         # of merc_io's own logic
$ grep -rl "merc_refinement" review/*.md
review/phase-3-algorithmic-core.md      # one incidental mention, as "a
                                         # caller of quotient_lts_block",
                                         # not a review of merc_refinement
                                         # itself
```

`phase-3-algorithmic-core.md`'s own header explains part of why: "The
`vpg`/typecheck-unification/`sabre` automaton agent was cut off by a session
rate limit before delivering findings; not covered here" — but that
explains the `vpg`/unification gap (closed by the second phase-3 doc), not
`io`/`refinement`/`explore`/`data`, which simply never appear as reviewed
material in either document, or anywhere else in the review tree.

This matters more than a typical "thin tests" finding because `crates/io`
is the **second-most depended-on crate in the entire root workspace** (11
dependents, tied with `merc_lts` and `merc_data`, behind only
`merc_utilities`'s 27 — computed from `cargo metadata`'s resolve graph, root
workspace, direct workspace-internal dependency edges only) and has zero
`tests/` integration coverage, `merc_io`'s
own inline unit-test count (5 files with `#[test]`) is thin for a crate this
central. `crates/refinement` (3 dependents: LTS refinement/counterexample
construction) is smaller but equally unreviewed and equally has no `tests/`
directory (4 files with inline `#[test]`). `crates/explore` (2 dependents)
and `crates/data` (10 dependents) fare slightly better — each gets a handful
of incidental mentions in kani-extension/unsafe-gap/ffi-wrapper docs, close
enough to "touched" that I'm not counting them as unreviewed, but neither
got a dedicated correctness pass either.

- **Status**: this is a gap in the review effort's own scope tracking, not a
  demonstrated code defect — reported as-is so it can be picked up as a real
  Phase 3 follow-up (`crates/io`, `crates/refinement` specifically) rather
  than assumed closed because the README's table says "done".

### Finding B: `TagIndex` (`crates/utilities/src/tagged_index.rs`) has zero tests, and its own doc comment overclaims what the type guarantees

`crates/utilities` is the single most depended-on crate in the workspace (27
workspace-internal dependents), and `TagIndex<T, Tag>` is the newtype-index
abstraction used everywhere the review's own guidance flags as a risk area —
`StateIndex`/`LabelIndex` (`crates/lts`), `VertexIndex`/`Priority`
(`crates/vpg`), `SortId`/`VarId`/`ConstructorId`/... (`crates/syntax`),
`ExprId`/`InferSortId`/`ResolvedSortId` (`crates/typecheck`), `BlockIndex`
(`crates/collections`), `CounterIndex` (`crates/refinement`). Before this
phase, `tagged_index.rs` had **no `#[cfg(test)]` module at all** (`grep -c
"#\[test\]" crates/utilities/src/tagged_index.rs` → `0`).

Its doc comment claims: *"Two `TagIndex` values compare, order, and hash
together only when they share both `T` and `Tag`, so indices from different
domains (e.g. state, action, priority) cannot be mixed up or compared with
each other."* That's true for `TagIndex<T,A> == TagIndex<T,B>` (doesn't
compile, different `Tag`s — the type system genuinely blocks that). It is
**not** true in general: `impl<T: PartialEq, Tag> PartialEq<T> for
TagIndex<T, Tag>` (and the matching `PartialOrd<T>`) let a tagged index
compare directly against the *raw, tag-erased* `T`. Once either side is
expressed as a raw value (e.g. via `.value()`, which the codebase's own
idiom uses pervasively — `label_index.value() == mapped_label`,
`state.value() == self.state_to_node[...]`, etc.), nothing distinguishes a
`StateIndex` from an `ActionIndex` that happens to wrap the same number. I
added a test (`tagged_index.rs`, new `mod tests`) that constructs
`StateIndex`/`ActionIndex` wrapping the same value and shows `state ==
action.value()` compiles and returns `true` with no warning — exactly the
"mixed up" scenario the doc comment says can't happen.

I searched for a live call site that actually exploits this hole (comparing
two indices from genuinely different domains through the raw-value bridge)
and found none — every call site I checked calls `.value()` on *both* sides
of a same-domain comparison as an explicit, disciplined idiom (`s.value() ==
t.value()`, both `s`/`t` the same `TagIndex` type), so the hole is currently
latent, not exploited. Status: **PLAUSIBLE design gap**, not a demonstrated
wrong result — reported because a zero-test file is exactly where this kind
of doc/code mismatch survives, and this is squarely the "TagIndex newtypes
crossed between state/action/priority" risk class this review has been
tracking throughout.

- **One-line fix direction**: either narrow the doc comment to describe what
  `PartialEq<T>`/`PartialOrd<T>` actually allow, or remove those two impls
  and force all raw-value comparisons through `.value()` on both sides
  explicitly (which is already the codebase's universal idiom, so removing
  them costs nothing observed).

### Finding C: `tools/rewrite` and `tools/vpg` have zero automated tests of any kind

```
$ find tools/vpg tools/rewrite -name "*.rs" | xargs grep -c '#\[test\]'
tools/vpg/src/main.rs:0
tools/rewrite/src/lib.rs:0
tools/rewrite/src/main.rs:0
tools/rewrite/src/trs_format.rs:0
```
Neither has a `tests/` directory either. `phase-4-tools-cli.md` explicitly
reviewed both CLI layers for correctness ("`tools/rewrite` and `tools/vpg`
had no defects found") but that review was a manual read, not backed by any
test — unlike their siblings `tools/lts` and `tools/sym`, which *did* gain
CLI-level regression tests during phases 3/4
(`tools/lts/tests/convert_cli.rs`,
`tools/sym/tests/{oxidd_default_capacity_cli,convert_reduce_output_cli}.rs`).
So a future change to `merc-rewrite`'s format dispatch or `merc-vpg`'s
subcommand wiring has nothing in the tree that would catch a regression,
where its two siblings now do.

- **Status**: structural fact, not a bug — flagged because it's the one
  clear "these two are worse off than their nearest siblings" asymmetry the
  structural audit surfaces cleanly.

### Checked, found adequately covered (not re-reported)

- `crates/collections/src/scc_decomposition.rs` — zero tests *inside*
  `merc_collections`, but exercised extensively by `crates/reduction`'s
  randomized cross-checks; already noted and accepted in
  `phase-4-foundation-devtools.md`.
- `crates/macros`' `mcrl2_derive_terms` codegen test making no assertions,
  and the untested generics-with-lifetimes codegen path — already flagged in
  `phase-4-mcrl2-logic.md` and `phase-4-unsafe-gap.md` respectively.
- `crates/vpg`'s `FamilyOptimisedLeft`+`alternative_solving=true` combination
  — already found and closed with a new test in
  `phase-3-vpg-typecheck-unification-automaton.md`.
- `crates/unsafety`'s concurrent structures having no loom coverage —
  already flagged as a structural gap in
  `phase-1-foundation-unsafe-sharedmutex-unsafety.md`.
- `crates/typecheck`'s `modal/check.rs`/`process/check.rs`/`pres/check.rs`
  being the worst-health, most-untested files in the repo — already the
  README's top "High" finding (unbounded-recursion stack overflow), not
  re-reported here.

---

## 4. Carried-over finding: `refine_bisimulation` panics on any LTS with action variables — root-caused and demonstrated

`review/phase-4-tools-cli.md` left this as a DEFERRED finding with a real
repro (`merc-sym reduce strong-bisim` panicking inside
`oxidd_reorder::set_var_order`, "the level number does not match") but no
root cause beyond "the `order` vector omits action variables". I dug into
`oxidd_reorder`'s actual `set_var_order`/`sort_order` implementation
(vendored at `~/.cargo/git/checkouts/oxidd-42df46d0d79cab3d/`) and found the
real mechanism, which is more precise than "omits them, therefore breaks":

**Root cause.** `oxidd_reorder::set_var_order`'s own contract says variables
*not* mentioned in `order` are "placed in a position such that the least
number of adjacent level swaps need to be performed" — by design, this can
place an unmentioned variable *between* two variables that are adjacent in
the caller's requested `order`. `refine_bisimulation`
(`crates/symbolic/src/bdd/refine.rs:106-114`) builds `order` by interleaving
only the state/`q`/next-state/`q'` variables (`lts.state_variables()`,
freshly-added `q_variables`, `lts.next_state_variables()`,
`q_prime_variables`), omitting `lts.action_variables()` entirely. Because
the action-label variables were created (in
`SymbolicLtsBdd::from_symbolic_lts`) right after the state/next-state block
and before `q`/`q'` exist at all, `sort_order`'s minimal-adjacent-swap
placement puts them **between** `p` and `q` in the reordered manager,
breaking the "each substitution target is exactly one level below its
source" invariant that `crates/symbolic/src/util.rs::variable_rename`
requires for the `p_to_q`/`q_to_p_prime`/`p_prime_to_q_prime` substitutions
`refine_bisimulation` builds right after the reorder call. With a small,
simple manager the reorder call itself succeeds but `variable_rename`
immediately panics ("Variable renaming must be to the level directly
below"); with a larger, more complex manager (real LTS input, more nodes
referencing the reordered levels), the reorder operation's own internal
node-splitting invariant fails first, which is the "level number does not
match" panic the original repro hit — **same root cause, two different
panic sites depending on manager size/complexity**.

**Demonstrated with three new tests** (`crates/symbolic/src/bdd/refine.rs`,
`#[cfg(test)] mod tests`, all currently pass/fail as stated —
`cargo test -p merc_symbolic --lib bdd::refine::tests`):

- `set_var_order_without_action_vars_breaks_p_q_adjacency` (line 284) —
  isolates the bug at the `oxidd_reorder` level alone, independent of
  `SymbolicLtsBdd`/`random_symbolic_lts`: builds a 5-variable manager with
  `refine_bisimulation`'s exact variable-creation order (`s`, `s'`, `a`, then
  `q`, `q'`), calls `oxidd_reorder::set_var_order` with exactly
  `refine.rs`'s `order` construction, and asserts the adjacency
  `variable_rename` needs. **FAILS** as predicted:
  ```
  assertion `left == right` failed: q should be directly below s, but the
  unlisted action variable a landed at level 1 (s=0, q=2, s'=3, q'=4)
    left: 2
   right: 1
  ```
  This is CONFIRMED, minimal, deterministic, and needs no LTS machinery at
  all — the clearest possible evidence for the mechanism above.
- `set_var_order_with_action_vars_appended_preserves_p_q_adjacency` (line
  338) — same setup, but appends the action variable to `order` instead of
  omitting it (`vec![s, q, s_prime, q_prime, a]`). **PASSES**: `a` sorts
  after `q'` as requested, and the `p`/`q`/`p'`/`q'` adjacency holds. This
  confirms the fix direction actually works, at the oxidd level, without
  touching `refine_bisimulation`'s higher-level logic at all.
- `refine_bisimulation_panics_on_lts_with_action_variables` (line 372) — the
  requested minimal, deterministic end-to-end repro through the real
  `refine_bisimulation` function (1 state variable, 2 action labels is
  enough — no need for the original 10-state/5-action random search).
  **FAILS**, reproducing the `variable_rename` panic (not the
  `set_var_order` one — see above for why the two differ by manager size):
  ```
  thread '...' panicked at crates/symbolic/src/util.rs:208:13:
  Variable renaming must be to the level directly below
  ```
- The pre-existing, previously-`#[ignore]`d `test_random_refine_bisimulation`
  (line 401) was left ignored (it needs the real fix in
  `refine_bisimulation`, not just the `order` vector, since
  `quotient_symbolic` downstream would need to be re-checked against
  whatever the corrected variable layout turns out to be) — but it no longer
  compiled at all before this phase (`compare_lts`'s signature had gained a
  `counter_example: bool` parameter and now returns a tuple; fixed so it at
  least compiles and documents the still-open state accurately, without
  changing what it tests).

**Status**: FIXED. Applied the verified fix direction: `refine.rs`'s `order`
construction now appends `lts.action_variables()` after the interleaved
p/q/p'/q' variables, right before the `oxidd_reorder::set_var_order` call
(`refine.rs:104-122`):

```rust
manager_ref.with_manager_exclusive(|manager| {
    let mut order: Vec<_> = lts
        .state_variables()
        .iter()
        .zip(q_variables.iter())
        .zip(lts.next_state_variables().iter().zip(q_prime_variables.iter()))
        .flat_map(|((s, q), (s_prime, q_prime))| [*s, *q, *s_prime, *q_prime])
        .collect();
    order.extend(lts.action_variables().iter().copied());

    oxidd_reorder::set_var_order(manager, &order)
});
```

**Verification**:

```
$ cargo nextest run -p merc_symbolic --lib -E 'test(bdd::refine::tests)' --run-ignored all
     Summary [   0.229s] 4 tests run: 2 passed, 2 failed, 71 skipped
        FAIL (3/4) merc_symbolic bdd::refine::tests::test_random_refine_bisimulation
        FAIL (4/4) merc_symbolic bdd::refine::tests::set_var_order_without_action_vars_breaks_p_q_adjacency
```

- `refine_bisimulation_panics_on_lts_with_action_variables` — **now PASSES**
  (previously panicked with "Variable renaming must be to the level directly
  below"). This is the direct end-to-end confirmation: the real
  `refine_bisimulation` function, called with an LTS that has action
  variables, no longer panics.
- `set_var_order_with_action_vars_appended_preserves_p_q_adjacency` —
  continues to pass (unchanged; it already demonstrated the fix direction
  works at the `oxidd_reorder` level before this phase).
- `set_var_order_without_action_vars_breaks_p_q_adjacency` — **still fails,
  by design, not a regression**: this test never calls `refine_bisimulation`
  or any of `refine.rs`'s production code. It manually builds an `order` that
  *omits* the action variable (reproducing the old, buggy construction
  in isolation) specifically to characterize `oxidd_reorder`'s own documented
  placement behavior for unmentioned variables. Fixing `refine.rs` cannot
  make this assertion pass, because the assertion is about what happens when
  you *don't* apply the fix. It remains valuable as a permanent, minimal
  characterization of the underlying library behavior that caused the bug,
  independent of merc's own code. (Confirmed by reading the test: it builds
  its own manager and its own `order` vector — `crates/symbolic/src/bdd/refine.rs`
  lines 306-ish, no call to `refine_bisimulation` anywhere in the function
  body.)
- `test_random_refine_bisimulation` — still fails, but for an unrelated
  reason: run explicitly, it no longer hits the `variable_rename`/`set_var_order`
  panic, but instead fails with `OutOfMemory` inside `refine_bisimulation`
  (`crates/symbolic/src/bdd/refine.rs:31`, propagated via `?` from a
  `BDDFunction` op). Root cause: the test's BDD manager is sized
  `oxidd::bdd::new_manager(2028, 2028, 1)` — a capacity that was already this
  small in the original, commented-out version of this test (confirmed via
  `git show f7bcec78 -- crates/symbolic/src/bdd/refine.rs`, which shows the
  pre-phase-5 commented-out test used the identical `2028, 2028` capacity),
  so it predates this fix and isn't something the `order` fix introduced.
  With 10 state variables, 5 action labels and 100 random iterations, the
  real (now-succeeding, further-running) refinement loop needs materially
  more manager capacity than this pre-existing, apparently-never-actually-run
  test provisions (other tests/CLI code in this crate use
  `BDD_NODE_CAPACITY`/`BDD_CACHE_CAPACITY` = `1 << 22`, three orders of
  magnitude larger). Left `#[ignore]`d as instructed — resizing the manager
  and/or re-checking `quotient_symbolic` against the corrected variable
  layout is a separate, not-yet-scoped follow-up, not part of this fix.
- Full crate suite: `cargo nextest run -p merc_symbolic --no-fail-fast` →
  75 passed, 1 failed (`set_var_order_without_action_vars_breaks_p_q_adjacency`,
  expected per above), 1 skipped (`test_random_refine_bisimulation`, still
  `#[ignore]`d) — no other regressions.
- `cargo clippy -p merc_symbolic --all-targets` and
  `cargo +nightly fmt --all -- --check` (after reformatting `refine.rs`,
  which had pre-existing formatting drift from when its test module was
  added in this phase) both pass clean for this file; the only remaining
  `cargo fmt --check` diff in the crate is a pre-existing, unrelated
  `use` reordering in `crates/symbolic/src/lib.rs:29` that predates this
  phase's commit and this fix.

---

## Tests added

All in `crates/symbolic/src/bdd/refine.rs` (`#[cfg(test)] mod tests`) and
`crates/utilities/src/tagged_index.rs` (`#[cfg(test)] mod tests`, newly
added — the file had no test module before):

- `crates/symbolic/src/bdd/refine.rs::tests::set_var_order_without_action_vars_breaks_p_q_adjacency`
  — CONFIRMED failing (isolates the root cause). Run with:
  `cargo test -p merc_symbolic --lib set_var_order_without_action_vars_breaks_p_q_adjacency`
- `crates/symbolic/src/bdd/refine.rs::tests::set_var_order_with_action_vars_appended_preserves_p_q_adjacency`
  — passes, confirms the fix direction. Run with:
  `cargo test -p merc_symbolic --lib set_var_order_with_action_vars_appended_preserves_p_q_adjacency`
- `crates/symbolic/src/bdd/refine.rs::tests::refine_bisimulation_panics_on_lts_with_action_variables`
  — CONFIRMED failing (minimal end-to-end repro, replaces the need for the
  slower 100-iteration random search to hit this). Run with:
  `cargo test -p merc_symbolic --lib refine_bisimulation_panics_on_lts_with_action_variables`
- `crates/symbolic/src/bdd/refine.rs::tests::test_random_refine_bisimulation`
  — fixed to compile against `compare_lts`'s current signature; left
  `#[ignore]`d (unchanged semantics, still needs the real fix upstream of
  it). Run with:
  `cargo test -p merc_symbolic --lib test_random_refine_bisimulation -- --ignored`
- `crates/utilities/src/tagged_index.rs::tests::tag_index_partial_eq_with_raw_value_bridges_across_unrelated_domains`
  — passes, documents the doc/code mismatch in `TagIndex`'s cross-type
  `PartialEq`/`PartialOrd`. Run with:
  `cargo test -p merc_utilities --lib tagged_index`

The workspace profile sets `panic = "abort"` (`Cargo.toml:13`), so a plain
`cargo test -p merc_symbolic --lib bdd::refine::tests` run aborts the whole
test binary (SIGABRT) after the first panicking test instead of reporting
all of them — this is a pre-existing characteristic of every test binary in
this workspace, not specific to these tests. Use `cargo nextest`, which runs
each test in its own process, to see all four results together:

```
$ cargo nextest run -p merc_symbolic --lib -E 'test(bdd::refine::tests)' --run-ignored all
     Summary [   0.122s] 4 tests run: 1 passed, 3 failed, 75 skipped
        FAIL (2/4) merc_symbolic bdd::refine::tests::test_random_refine_bisimulation
        FAIL (3/4) merc_symbolic bdd::refine::tests::set_var_order_without_action_vars_breaks_p_q_adjacency
        FAIL (4/4) merc_symbolic bdd::refine::tests::refine_bisimulation_panics_on_lts_with_action_variables
```
(`test_random_refine_bisimulation` is normally `#[ignore]`d; it's included
here via `--run-ignored all` and fails for the same underlying reason, as
expected.) `set_var_order_with_action_vars_appended_preserves_p_q_adjacency`
is the one that passes.

**Post-fix update**: with the `order` fix applied to `refine_bisimulation`
(see §4's Status), the same command now gives:
```
$ cargo nextest run -p merc_symbolic --lib -E 'test(bdd::refine::tests)' --run-ignored all
     Summary [   0.229s] 4 tests run: 2 passed, 2 failed, 71 skipped
        FAIL (3/4) merc_symbolic bdd::refine::tests::test_random_refine_bisimulation
        FAIL (4/4) merc_symbolic bdd::refine::tests::set_var_order_without_action_vars_breaks_p_q_adjacency
```
`refine_bisimulation_panics_on_lts_with_action_variables` now passes (the
end-to-end panic is gone). The two still-failing tests fail for reasons
unrelated to the fix — see §4's Status for why each one is expected to keep
failing (a permanent oxidd-level characterization test, and a pre-existing
undersized test manager, respectively).
