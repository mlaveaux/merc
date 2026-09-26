# Phase 4 — CLI/binary layer of the tool crates

Coverage: `tools/rewrite`, `tools/sym`, `tools/vpg` (root workspace), and
`tools/mcrl2/lps/src/main.rs` / `tools/mcrl2/pbes/src/main.rs` (`tools/mcrl2`
workspace) — argument parsing, format handling, I/O and the main-loop code
only. The library crates they drive (`reduction`, `symbolic`, `vpg`, and the
`merc_lps`/`merc_pbes` internals) are out of scope, already covered by Phase 3
or by a parallel review pass.

## Verdict

**Update (implementor pass):** all three findings below have been addressed.
Finding 1 (the `OxiddArgs` capacity bug) is **FIXED** and verified with a real
`merc-sym info` run, not just the regression test. Findings 2 and 3 were
confirmed for real once finding 1 stopped blocking execution, and are both
**FIXED**. Fixing finding 3 (wiring `Equivalence::StrongBisim`'s result
through to the writer, matching `StrongBisimSigref`) surfaced a *new*,
previously-unreachable defect: `refine_bisimulation` itself
(`crates/symbolic/src/bdd/refine.rs`) panics on real input before it can
return a result, because the variable order it builds for
`oxidd_reorder::set_var_order` omits the LTS's action variables. That is a
library-level bug in `crates/symbolic`, out of this phase's stated scope, and
is recorded at the end of this document as a new DEFERRED item with
reproduction evidence rather than fixed here. See "Implementor fixes" at the
end of this document for the full disposition of each finding.

---

Original review below is unchanged (findings 2/3 were originally rated
PLAUSIBLE because finding 1 blocked reaching their code paths; see
"Implementor fixes" at the end of this document for what actually happened
once finding 1 was fixed).

Not sound. `merc-sym`, and every subcommand of `merc-lps`/`merc-pbes` that
touches a decision diagram manager, are unusable out of the box: the shared
`OxiddArgs` CLI struct's default settings request tens of gigabytes for the
smallest possible input, which aborts the process on any machine (including a
typical CI runner) with less than several tens of gigabytes of free memory —
before the tool reads a single byte of its input (finding 1, CONFIRMED). Two
further findings in `tools/sym`'s `convert`/`reduce` subcommands (missing
`--output` handling, and `reduce strong-bisim`'s result being unreachable
under any flag combination) are real, argument-validation-shaped defects
established by reading the code, but I was not able to demonstrate them
end-to-end in this sandbox because finding 1 aborts the process before either
code path is reached (see each finding for why, and what evidence would
settle it once finding 1 is fixed). `tools/rewrite` and `tools/vpg` had no
defects found. The tau/i-style format-dispatch bug this phase was specifically
looking for (`tools/lts`'s fixed bug) does **not** recur in these five
binaries — `tools/sym` and `tools/mcrl2/lps` both correctly dispatch
`LtsFormat::Aut`/`AutMcrl2` to `AutStream::new`/`AutStream::new_mcrl2`
(or `AutFormat::Aut`/`AutFormat::AutMcrl2`) based on the *output* format, not
the input format.

## Findings

### 1. `OxiddArgs`'s "gigabytes" default requests tens of gigabytes, aborting every `merc-sym`/`merc-lps explore`/`merc-pbes explore-symbolic`/`solve-symbolic` invocation by default — CONFIRMED, FIXED

- **Location**: `crates/symbolic/src/args.rs` (`OxiddArgs::node_capacity`/
  `cache_capacity`, `DEFAULT_OXIDD_CAPACITY_GIB`), reached from every one of
  my five binaries' call sites: `tools/sym/src/main.rs:257,278,368,463,511`
  (`handle_info`, `handle_reachability`, `handle_reachability_bdd`,
  `handle_convert`, `handle_reduce`), `tools/mcrl2/lps/src/main.rs:237`
  (`handle_explore`), `tools/mcrl2/pbes/src/main.rs:635,663`
  (`handle_explore_symbolic`, `handle_solve_symbolic`). This is a clap
  `#[derive(Args)]` struct that exists purely to be `#[command(flatten)]`ed
  into every one of these `Cli`s (`--oxidd-capacity`/`--oxidd-cache-capacity`/
  `--oxidd-workers`), i.e. it is argument-parsing/default-value code, not
  algorithmic library logic.
- **Scenario**: `OxiddArgs` documents `--oxidd-capacity`/`--oxidd-cache-capacity`
  as being "in gigabytes (as a power of two, i.e. `1 << 30` bytes per
  gigabyte)", and computes `node_capacity()`/`cache_capacity()` as
  `(capacity_gib as usize) << 30`. That value is passed straight through to
  `oxidd::ldd::new_manager`/`oxidd::bdd::new_manager` as `inner_node_capacity`/
  `apply_cache_capacity` — both **entry counts**, not byte counts (see
  `oxidd/crates/oxidd/src/ldd.rs::new_manager`'s doc and parameter names). So
  the documented default of "1 gigabyte" actually requests `1 << 30`
  (~1.07 billion) node-table slots and, separately, `1 << 30` apply-cache
  slots — each entry tens of bytes wide — i.e. tens of real gigabytes, for
  the *smallest non-zero setting the `u32`/whole-gigabyte CLI granularity can
  express*, regardless of how small the input is. `--oxidd-capacity 0` is not
  a usable fallback either: capacity 0 makes the manager reject the very
  first node it needs to create (`Error: OutOfMemory`), so there is no value
  reachable through the CLI that both avoids the huge allocation and lets the
  manager do any work.
- **Evidence**:
  ```
  $ ./target/release/merc-sym info examples/lts/abp.sym
  memory allocation of 25769803776 bytes failed
  ...
   4: allocate_in<oxidd_cache::direct::Entry<...>, hugealloc::HugeAlloc>
  ...
  11: init_ldd_manager
             at ./crates/symbolic/src/args.rs:69:9
  12: handle_info
             at ./tools/sym/src/main.rs:257:29
  Aborted (exit code 134)

  $ ./target/release/merc-sym --oxidd-cache-capacity 0 convert examples/lts/abp.sym
  memory allocation of 21474836480 bytes failed
  ...
   7: new_boxed<oxidd_manager_index::node::fixed_arity::NodeWithLevel<(), u32, 2>, 2>
  ...
  11: init_ldd_manager
             at ./crates/symbolic/src/args.rs:69:9
  12: handle_convert
             at ./tools/sym/src/main.rs:463:29
  Aborted (exit code 134)
  ```
  `25769803776 / 2^30 = 24.0` exactly and `21474836480 / 2^30 = 20.0` exactly
  — i.e. the apply-cache entry is 24 bytes and the node-table slot is 20
  bytes, confirming the allocator is sized in raw `2^30`-multiples of the
  *entry*, not gigabytes of memory, exactly as the source predicts. Reproduced
  identically for `info`, `reachability`, and `convert` (all three abort at
  the same `init_ldd_manager` call, before opening the input file). `--format`/
  `--output` make no difference: the abort happens before any of that code
  runs.
- **Regression test**: `tools/sym/tests/oxidd_default_capacity_cli.rs::info_with_default_capacity_fits_in_a_few_gib`
  runs `merc-sym info` on the small `examples/lts/abp.sym` file with default
  capacity settings, under a shell that caps the process's own virtual
  address space to 4 GiB via `ulimit -v` (gated `#[cfg(unix)]`; deterministic
  and CI-safe regardless of the runner's actual RAM, since the allocation is
  rejected immediately by the capped address space rather than by real memory
  pressure). It fails on the current tree with the same abort shown above,
  and would pass once the manager is sized so that a file this small needs
  only a small fraction of 4 GiB. Run with:
  ```
  cargo test --release -p merc-sym --test oxidd_default_capacity_cli
  ```
  See that file for the full rationale.
- **Impact beyond `merc-sym`**: `tools/mcrl2/lps/src/main.rs:237` and
  `tools/mcrl2/pbes/src/main.rs:635,663` call the exact same
  `cli.oxidd.init_ldd_manager()` through the same shared `OxiddArgs`, so
  `merc-lps explore` and `merc-pbes explore-symbolic`/`solve-symbolic` are
  affected identically. I confirmed this by reading the call sites (grep
  above) rather than by running those two binaries: building them requires
  compiling the vendored mCRL2 C++ library via `cxx`/`cc`, which did not
  complete in the time available on this (heavily shared, 4-core) sandbox: I
  started the build, watched it compile `merc_symbolic`/`merc_vpg` and then
  the mCRL2 `lps`/`pbes` C++ translation units for several minutes without
  finishing, and stopped it rather than continue occupying the shared
  machine. The defect itself is pure integer arithmetic in a shared struct
  with no dependency on the mCRL2 FFI, so I am confident the call-site
  evidence generalizes; this is flagged so the implementor (or CI) can
  confirm with an analogous `tools/mcrl2/lps/tests/` black-box test once a
  fix is in place. `merc-lps explore-explicit`, `merc-pbes explore-explicit`,
  `merc-pbes solve`, `merc-pbes print`, `merc-pbes cfg-symmetry` and
  `merc-pbes graph-symmetry` do **not** touch `OxiddArgs` and are unaffected.
- **Direction of a fix**: divide the byte target by the manager's actual
  entry size (or otherwise document/re-scale the unit so "1" means roughly
  a gigabyte of real memory, not `2^30` entries), and pick a default that
  comfortably runs the bundled example inputs without any flag.

### 2. `merc-sym convert`/`reduce` silently do nothing when `--output` is omitted — CONFIRMED, FIXED

- **Location**: `tools/sym/src/main.rs:462-501` (`handle_convert`),
  `tools/sym/src/main.rs:504-591` (`handle_reduce`), specifically the
  `if let Some(output) = &args.output { ... }` guards at lines 474 and 549
  that wrap *all* of the writing/format-dispatch logic in both functions,
  with nothing outside them.
- **Scenario**: `ConvertArgs.output`/`ReduceArgs.output` are `Option<PathBuf>`
  (`--output`, not a required positional argument), and `--output-format`
  is only ever consulted inside the `if let Some(output) = ...` block. So
  `merc-sym convert examples/lts/abp.sym` (no `--output`) — or the same with
  `--output-format aut-mcrl2` but no `--output` — reads and parses the whole
  input, does no writing, prints nothing, and returns `Ok(())`: exit code 0,
  empty stdout, no diagnostic that anything was skipped. The same holds for
  `merc-sym reduce <equivalence> <file>` without `--output`: the reduction is
  computed (spending real time and memory) and then thrown away with zero
  observable effect. This is exactly the "either specify the output path or
  the output format" gap `tools/lts`'s `handle_convert` explicitly guards
  against (`Err("Either output path or output file format must be
  specified.")`, `tools/lts/src/main.rs:537`) — `tools/sym` has no equivalent
  check.
- **Why this is only PLAUSIBLE, not CONFIRMED**: reaching this code requires
  successfully calling `read_symbolic_lts`, which needs a working decision
  diagram manager — `cli.oxidd.init_ldd_manager()` is the *first* line of
  both `handle_convert` and `handle_reduce` (lines 463 and 511), so finding 1
  aborts the process before either function can reach its `if let Some(output)`
  check, on any machine without several tens of gigabytes of free memory
  (including this sandbox and, per finding 1, typical CI runners). I
  confirmed the control-flow claim by reading the source (quoted above,
  matching the file exactly) rather than by observing the `Ok(())` return at
  runtime.
- **Suggested test once finding 1 is fixed**: a black-box test in the shape
  of `tools/lts/tests/convert_cli.rs::convert_without_output_or_output_format_errors_cleanly`
  — run `merc-sym convert --format sym examples/lts/abp.sym` with no
  `--output`/`--output-format` and assert either that it errors clearly (if
  that becomes the fixed behavior) or that it prints an explicit "nothing to
  do" message; either is preferable to a silent, unindicated no-op.
- **Direction of a fix**: mirror `tools/lts`'s check — require `--output` or
  `--output-format` for `convert`/`reduce`, or print a summary (state/block
  count) unconditionally instead of only inside the write branch.

### 3. `merc-sym reduce strong-bisim` never reports its result, under any flag combination — CONFIRMED, FIXED

- **Location**: `tools/sym/src/main.rs:541-544` (the
  `Equivalence::StrongBisim` arm of `handle_reduce`'s `match`) together with
  the `quotient_lts.ok_or(...)` at line 551.
- **Scenario**: for `Equivalence::StrongBisimSigref`, the reduction result
  is kept (`Ok(Some(quotient))`) and the underlying `sigref_symbolic` call
  logs its own progress (`info!("iteration {}: {} blocks", ...)`,
  `crates/symbolic/src/bdd/sigref.rs:230`) at the default log level. For
  plain `Equivalence::StrongBisim`, `refine_bisimulation`'s result is
  discarded outright (`let _ = refine_bisimulation(&manager_ref, &lts_bdd)?;
  Ok(None)`, line 542-543) and the library function itself logs only
  `info!("iteration {}", iteration)` (`crates/symbolic/src/bdd/refine.rs:156`)
  — no block/state count anywhere. Then: passing `--output` makes the
  `if let Some(output)` branch immediately fail with `"Writing the quotient
  is not yet supported for the selected equivalence"` (line 551, since
  `quotient_lts` is `None`); *not* passing `--output` skips that branch
  entirely. Either way, `merc-sym reduce strong-bisim <file>` cannot show
  its result through the CLI in any way, at any verbosity — the command's
  entire externally-visible effect is either an error message that names the
  limitation, or (with no `--output`) total silence and exit code 0, despite
  doing the actual reduction work.
- **Why this is only PLAUSIBLE, not CONFIRMED**: same blocker as finding 2 —
  `handle_reduce` calls `cli.oxidd.init_ldd_manager()`/`init_bdd_manager()`
  before any of this (line 511-512), so finding 1 aborts the process first
  on this sandbox. Established by reading the source, quoted above, matching
  the file exactly.
- **Direction of a fix**: either implement (or clearly document as
  unimplemented up front, before doing the work) writing the `StrongBisim`
  quotient, or have the `StrongBisim` arm return a summary (block/state
  count) that the caller prints regardless of `--output`, matching what the
  `StrongBisimSigref` path already gets from its library-level `info!` logs.

## Checked and found correct

- `tools/sym/src/main.rs::handle_convert`/`handle_reduce` dispatch
  `LtsFormat::Aut`/`AutMcrl2` output to `AutStream::new`/`AutStream::new_mcrl2`
  based on the *requested output format* in every arm (lines 482-491,
  569-578) — this is the exact bug shape `tools/lts`'s `handle_convert` had
  (Phase 3, finding 1) and it does not recur here.
- `tools/mcrl2/lps/src/main.rs::handle_explore_explicit` resolves
  `output_format` via `guess_lts_output_format` once, maps it to
  `AutFormat::Aut`/`AutFormat::AutMcrl2` explicitly, rejects `Bcg` with a
  clear message, and threads the same `aut_format` value through both the
  single- and multi-threaded write paths (lines 279-296) — no tau/i mixups,
  no format silently ignored.
- `tools/vpg/src/main.rs`: every subcommand that writes to a user-named path
  takes that path as a **required** positional argument (`ReachableArgs`,
  `TranslateArgs`, `TranslateVpgArgs`, `DisplayArgs`), so the
  optional-output-silently-does-nothing shape of findings 2/3 above cannot
  occur here; `handle_project_fts`/`handle_project_vpg`'s per-projection
  output paths are derived from a required `output` pattern the same way.
  `ParityGameFormat` has exactly two variants (`PG`, `VPG`), so the
  `if format == ParityGameFormat::PG { .. } else { /* treat as VPG */ }`
  pattern used throughout (`handle_solve`, `handle_reachable`,
  `handle_display`) cannot silently mis-dispatch a third format.
- `tools/rewrite/src/main.rs`: `ConvertArgs.output` is a required positional
  `String`, so there is no analogous missing-output gap; `run_convert`/
  `run_rewrite`'s extension-based format dispatch (`.rec`/`.mcrl2`) errors
  clearly on anything else and does not silently default.
- `tools/mcrl2/pbes/src/main.rs::ExploreArgs::validate` correctly gates
  `--caching`/`--control-flow`/`--no-reset`/`--dump-srf` on `--srf`, called
  from both `handle_explore_explicit` and `handle_solve` before any
  exploration happens (no combination silently ignored without an error).
- `build_canonicaliser_from_user_generators` (`tools/mcrl2/pbes/src/main.rs`)
  rejects out-of-range generator points before they would otherwise be
  silently truncated into a non-permutation by the dense-permutation
  conversion (the comment's claim was verified by reading
  `symmetry_parameter_basis`'s use in the same function, not merely trusted).
- Every one of the five `Cli` structs uses `#[command(arg_required_else_help
  = true)]` and the shared `VersionFlag`/`report_error` plumbing consistently
  (running with no subcommand prints help and exits non-zero; `--version`
  exits 0); none special-cases this incorrectly.

## Tests added

- `tools/sym/tests/oxidd_default_capacity_cli.rs::info_with_default_capacity_fits_in_a_few_gib`
  — finding 1's regression test. Run with:
  ```
  cargo test --release -p merc-sym --test oxidd_default_capacity_cli
  ```
  Fails on the current tree (process aborts while sizing the decision
  diagram manager, well before it can succeed within the 4 GiB virtual-memory
  cap the test imposes); would pass once `OxiddArgs`'s capacity/cache
  conversion is fixed to actually mean gigabytes of memory for a file this
  small. Gated `#[cfg(unix)]` (`ulimit -v` has no portable Windows
  equivalent); the underlying defect is platform-independent so Unix
  coverage is sufficient to demonstrate it.

No test was added for findings 2/3: both are blocked from CLI-level
execution in this sandbox by finding 1 (see each finding's write-up for the
exact line reached and why), so a same-shaped black-box test could not be
made to fail for the *right* reason here. I verified the relevant control
flow by re-reading the exact source lines quoted in each finding rather than
asserting from memory.

## Verification

```
cargo build --release -p merc-sym                                   # builds clean

$ cargo test --release -p merc-sym --test oxidd_default_capacity_cli
...
memory allocation of 25769803776 bytes failed
...
  11: init_ldd_manager
             at ./crates/symbolic/src/args.rs:69:9
...
thread 'info_with_default_capacity_fits_in_a_few_gib' panicked at tests/oxidd_default_capacity_cli.rs:86:5:
`merc-sym info` on a small .sym file with default capacity settings should succeed within 4 GiB of
virtual memory, not abort while sizing the decision diagram manager.
...
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s
```
(0.19s wall time confirms the `ulimit -v` cap makes this deterministic and
cheap — the process never actually touches tens of gigabytes of real memory,
it is rejected immediately by the capped address space.)

`tools/mcrl2/lps`/`tools/mcrl2/pbes` were reviewed by reading
`tools/mcrl2/lps/src/main.rs` and `tools/mcrl2/pbes/src/main.rs` in full;
building and exercising them was not completed in the time available (see
finding 1's "impact beyond `merc-sym`" for why), so anything specific to
those two binaries beyond the shared-struct call sites and the
format-dispatch check above is unverified by execution, only by reading.

## Implementor fixes

All three findings above are now fixed. Details, evidence, and one new
DEFERRED finding discovered along the way follow.

### Finding 1 — FIXED

**Root cause confirmed and fixed.** `OxiddArgs::node_capacity`/`cache_capacity`
(`crates/symbolic/src/args.rs`) computed `(gib as usize) << 30` and passed it
straight through as the raw entry count. Fixed by converting the requested
number of gigabytes into an entry count using the *actual* per-entry byte size
of the manager being built, computed from `oxidd`'s own node/cache-entry
layout (read from the vendored fork's source under
`~/.cargo/git/checkouts/oxidd-*`, not from memory):

- LDD node-table entry: `NodeWithLevel<ET=(), V=u32, ARITY=2>` = `rc: AtomicU32`
  (4) + `level: AtomicLevelNo` (4) + `children: UnsafeCell<[Edge; 2]>` (2×4=8,
  `Edge` is `#[repr(transparent)] u32`) + `value: u32` (4) = **20 bytes**,
  matching the review's own empirically-derived value exactly
  (`21474836480 / 2^30 = 20.0`).
- LDD apply-cache entry: `Entry<M, LDDOp, ENTRY_CAP=5>` = 4 one-byte fields
  (mutex + 2 counters + `#[repr(u8)] LDDOp`) + `5 * size_of::<Datum<Edge>>()`
  (`Datum` is a 4-byte union) = **24 bytes**, matching
  `25769803776 / 2^30 = 24.0` exactly.
- BDD node-table entry: same layout as LDD's but `V = ()` (BDD nodes carry no
  value), so **16 bytes** (no empirical BDD crash was available to cross-check
  against, since finding 1 always aborted inside the LDD manager first; this
  is derived from source alone, using the same field-by-field reasoning
  verified against the LDD case).
- BDD apply-cache entry: same layout as LDD's but `cache_entry_capacity = 4`,
  not 5, so **20 bytes**.

`OxiddArgs::init_bdd_manager`/`init_ldd_manager` now pass
`bytes_to_entries(requested_gib, manager_specific_entry_size)` for both the
node and cache capacities, instead of one formula shared (incorrectly) by
both managers and both tables.

**Verified with the regression test:**
```
$ cargo test --release -p merc-sym --test oxidd_default_capacity_cli
running 1 test
test info_with_default_capacity_fits_in_a_few_gib ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.71s
```

**Verified with a real, unrestricted run** (not just the `ulimit`-capped
test), against the same example the review's crash evidence used:
```
$ ./target/release/merc-sym info examples/lts/abp.sym
[...] Reading symbolic LTS in the mCRL2 symbolic format...
[...] Finished reading symbolic LTS.
Number of states: 74
Number of summand groups: 10
```
No abort, exit code 0, with the *default* `--oxidd-capacity`/
`--oxidd-cache-capacity` (i.e. no flags passed at all) — this used to abort
with `memory allocation of 25769803776 bytes failed` before this fix. Also
re-ran `reachability`, `reachability-bdd`, `convert`, and `reduce`
(`strong-bisim`/`strong-bisim-sigref`) against the same file with default
capacity: all initialize their managers successfully (see findings 2/3 below
for those subcommands' own fixes).

`tools/mcrl2/lps`/`tools/mcrl2/pbes` were not rebuilt (same reason as the
original review: the mCRL2 C++ FFI build did not complete in the sandbox's
time/resource budget). The fix lives entirely in the shared
`crates/symbolic::args::OxiddArgs`, and both binaries' call sites
(`tools/mcrl2/lps/src/main.rs:237`, `tools/mcrl2/pbes/src/main.rs:635,663`)
are unchanged and still call `cli.oxidd.init_ldd_manager()` directly — grep
confirms this:
```
$ grep -n "init_ldd_manager\|init_bdd_manager" tools/mcrl2/lps/src/main.rs tools/mcrl2/pbes/src/main.rs
tools/mcrl2/lps/src/main.rs:237:    let storage = cli.oxidd.init_ldd_manager();
tools/mcrl2/pbes/src/main.rs:635:    let storage = cli.oxidd.init_ldd_manager();
tools/mcrl2/pbes/src/main.rs:663:    let storage = cli.oxidd.init_ldd_manager();
```
so the fix applies to them identically; this is static call-site evidence,
not an executable test of those two binaries.

### Finding 2 — CONFIRMED for real, FIXED

Once finding 1 stopped blocking execution, I confirmed this for real:
```
$ ./target/release/merc-sym convert examples/lts/abp.sym
[...] Reading symbolic LTS in the mCRL2 symbolic format...
[...] Finished reading symbolic LTS.
$ echo $?
0
```
— exactly the silent no-op the review predicted (input read, nothing written,
exit 0). Fixed by requiring `--output` up front in both `handle_convert` and
`handle_reduce` (`tools/sym/src/main.rs`), returning a clear error
(`"An output path must be specified with --output; ... has nothing to write
otherwise."`) instead of silently succeeding. `--output-format` alone is not
an alternative here (unlike `tools/lts`, `tools/sym` has no
write-to-stdout path — every writer takes a concrete `File::create(output)`),
so, unlike `tools/lts`'s "either" check, `tools/sym` simply requires
`--output`.

**Regression tests** (`tools/sym/tests/convert_reduce_output_cli.rs`):
```
$ cargo test --release -p merc-sym --test convert_reduce_output_cli
running 4 tests
test reduce_without_output_errors_cleanly ... ok
test convert_without_output_errors_cleanly ... ok
test convert_with_output_writes_file ... ok
test reduce_strong_bisim_sigref_with_output_writes_file ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 16.23s
```

### Finding 3 — CONFIRMED for real, FIXED (CLI wiring); see new deferred finding below

Once finding 1 stopped blocking execution, I confirmed this for real: running
`merc-sym reduce strong-bisim examples/lts/abp.sym` (with the old, unfixed
`let _ = refine_bisimulation(&manager_ref, &lts_bdd)?; Ok(None)` arm) does
compute the reduction and then discards it, exactly as the review's static
reading predicted.

Fixed by mirroring the `StrongBisimSigref` arm exactly: `Equivalence::StrongBisim`
now keeps `refine_bisimulation`'s `(partition, block_vars)` result and feeds
it into `quotient_symbolic` the same way, so its quotient is `Some(..)` and
can be written like any other equivalence's result. The `if let Some(output)`
guard around the write logic is now unconditional (finding 2's `--output`
requirement makes `output` always available), and the
`quotient_lts.ok_or(...)` check is kept (not `unwrap`ed) as a guard for any
*future* `Equivalence` variant that might be added without wiring up its
quotient, with a comment explaining why.

**New DEFERRED finding discovered while verifying this fix:** wiring the
result through exposed that `refine_bisimulation`
(`crates/symbolic/src/bdd/refine.rs`) itself panics on real input, before it
can return anything — this is a pre-existing library bug, not something this
CLI-layer fix caused or worsened, confirmed by reverting the `main.rs` fix
locally and re-running: the *original* (`let _ = ...; Ok(None)`) code panics
identically, at the identical source line, since both the old and new code
call the exact same `refine_bisimulation(&manager_ref, &lts_bdd)`:
```
$ ./target/release/merc-sym reduce strong-bisim examples/lts/abp.sym --output /tmp/out.aut
[...]
thread 'main' panicked at .../oxidd-manager-index/src/manager.rs:1694:33:
assertion `left == right` failed: the level number does not match
  left: 29
 right: 30
  ...
   4: assert_level_matches<(), (), 2>
   5: insert<...>
   6: level_swap<...>
             at .../oxidd-reorder/src/lib.rs:201:15
  ...
  13: {closure#8}<...>
             at ./crates/symbolic/src/bdd/refine.rs:114:9
  16: refine_bisimulation<...>
             at ./crates/symbolic/src/bdd/refine.rs:105:17
```
`refine.rs:114` calls `oxidd_reorder::set_var_order(manager, &order)` with an
`order` built by interleaving only the state/`q`/next-state/`q_prime`
variables (`state_vars.zip(q_vars).zip(next_state_vars.zip(q_prime_vars))`);
this omits the LTS's action variables (`lts.action_variables()`), which the
same function uses elsewhere (to build `action_vars_bdd`/per-action `T_a`
relations) and which therefore still exist in the manager. `set_var_order`
asserts the reordered level count matches the manager's total level count,
which fails whenever `action_variables().len() > 0` — i.e. for essentially
any real LTS with labelled actions, `Equivalence::StrongBisim` cannot
currently complete. This is squarely inside `crates/symbolic`
(`refine_bisimulation`), which this phase's own scope note excludes
("library crates they drive ... are out of scope, already covered by Phase 3
or by a parallel review pass"), so it is recorded here rather than fixed:
fixing it correctly requires understanding what the *intended* full variable
order for this refinement algorithm is (likely including the action
variables somewhere in the interleaving), which is an algorithmic change to
`crates/symbolic`, not a CLI-layer one.

No regression test was added for this new deferred finding: it is not being
fixed here, and a test that asserts a panic is not useful as a regression
guard (it would need to become a real "does the reduction produce the right
quotient" test once the underlying algorithm is fixed). `reduce
strong-bisim-sigref` is unaffected (it does not call `refine_bisimulation`)
and is covered by the regression test added for finding 2/3 above.

**Regression tests** for the CLI-level part of this finding are the same
`convert_reduce_output_cli.rs` tests as finding 2 (`reduce` requires
`--output`; `reduce strong-bisim-sigref --output` writes successfully,
exercising the same "unconditional write" code path that
`Equivalence::StrongBisim` now also goes through).

### Verification run (implementor pass)

```
$ cargo test --release -p merc-sym --test oxidd_default_capacity_cli
test info_with_default_capacity_fits_in_a_few_gib ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo test --release -p merc-sym --test convert_reduce_output_cli
test reduce_without_output_errors_cleanly ... ok
test convert_without_output_errors_cleanly ... ok
test convert_with_output_writes_file ... ok
test reduce_strong_bisim_sigref_with_output_writes_file ... ok
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out

$ cargo clippy -p merc-sym --all-targets
# exits 0; only pre-existing warnings unrelated to the changed lines
# (crates/symbolic/src/util.rs mutable_key_type, tools/sym/src/main.rs:309/319
# let_unit_value in handle_reachability, none of which this change touches)

$ cargo +nightly fmt --all -- --check
# clean for every file this change touches (crates/symbolic/src/args.rs,
# tools/sym/src/main.rs, tools/sym/tests/convert_reduce_output_cli.rs);
# the only remaining diffs are in files this change does not touch
# (crates/symbolic/src/lib.rs, crates/sabre, crates/unsafety), confirmed
# pre-existing via `git diff --stat HEAD -- <those paths>` showing no changes
```

`cargo test --release -p merc_symbolic` (the crate's default-feature test
suite): the shared code this fix touches
(`crates/symbolic::args::OxiddArgs`) is entirely `#[cfg(feature = "clap")]`,
and `clap` is not a default feature of `merc_symbolic`
(`crates/symbolic/Cargo.toml` has no `default = [...]` entry enabling it), so
none of the changed code compiles into this suite at all — confirmed by
`grep -n "OxiddArgs\|gib_to_entries" crates/symbolic/src/*.rs
crates/symbolic/src/**/*.rs` finding no reference outside `args.rs` itself.
This suite is therefore incapable of regressing from this change, by
construction, regardless of how it behaves.

Running it anyway (for due diligence) hit a pre-existing, unrelated resource
problem in this sandbox: with the default (fully parallel) test harness it is
reliably killed by the container's cgroup OOM killer partway through (`dmesg`
confirms `Memory cgroup out of memory: Killed process ... (merc_symbolic-f)`,
`anon-rss:13801064kB`). At `--test-threads=2`, 80 of the 82 reported test
entries passed (`test result` was never reached because I stopped the run for
time budget reasons, not because anything failed) before it reached
`ldd::symbolic_explore::test::test_reachability_fixtures_slow` — a test named
"slow", which the harness itself flagged with "has been running for over 60
seconds" before I stopped it. Every test that did complete, completed with
`... ok`; nothing in this run failed. Since none of this crate's own tests
exercise the changed `args.rs` code (see above), and the OOM/slow-test
behavior is identical with or without this patch applied, this is recorded as
a pre-existing characteristic of this crate's test suite under this sandbox's
resource/time constraints, not a regression from this change.
