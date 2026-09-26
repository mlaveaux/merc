# Phase 4 — CLI/binary layer of the tool crates

Coverage: `tools/rewrite`, `tools/sym`, `tools/vpg` (root workspace), and
`tools/mcrl2/lps/src/main.rs` / `tools/mcrl2/pbes/src/main.rs` (`tools/mcrl2`
workspace) — argument parsing, format handling, I/O and the main-loop code
only. The library crates they drive (`reduction`, `symbolic`, `vpg`, and the
`merc_lps`/`merc_pbes` internals) are out of scope, already covered by Phase 3
or by a parallel review pass.

## Verdict

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

### 1. `OxiddArgs`'s "gigabytes" default requests tens of gigabytes, aborting every `merc-sym`/`merc-lps explore`/`merc-pbes explore-symbolic`/`solve-symbolic` invocation by default — CONFIRMED

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

### 2. `merc-sym convert`/`reduce` silently do nothing when `--output` is omitted — PLAUSIBLE (blocked by finding 1)

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

### 3. `merc-sym reduce strong-bisim` never reports its result, under any flag combination — PLAUSIBLE (blocked by finding 1)

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
