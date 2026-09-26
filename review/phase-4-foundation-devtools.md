# Phase 4 — foundation collections/utilities and dev tooling

Coverage: `crates/collections` (`block_partition`, `compressed_vec`, `graph`, `indexed_partition`,
`indexed_set`, `remove_duplicates`, `scc_decomposition`, `vec_difference`, `vecbag`, `vecset`,
`lib.rs`), `crates/utilities` (all modules: `generational_index`, `fixed_cache_policy`,
`fixed_size_cache`, `permutation`, `sharded_counter`, `tagged_index`, `span`, `source_map`,
`cast_macro`, `debug_trace`, `error`, `helper`, `pest_display_pair`, `snapshot`, `timing`,
`traversal`, `random_test`, `kani_rng`, `test_logger`), `crates/rec-tests` (light pass), and
`crates/xtask` (`coverage.rs`, `sanitizer.rs`, `discover_tests.rs`, `package.rs`, `publish.rs`,
`tool_testing.rs`). Both `collections` and `utilities` are `#![forbid(unsafe_code)]` and contain no
`unsafe`, confirmed by `grep -rn unsafe crates/utilities/src crates/collections/src
crates/rec-tests/src crates/xtask/src` (only hits are the `forbid` attributes themselves, a
doc-comment, and an unrelated `crates/xtask/src/publish.rs` string literal `"merc_unsafety"`); no
`unsafe-verify` work was needed for this phase.

## Verdict

`crates/collections` and `crates/utilities` are sound: dense algorithmic code
(`scc_decomposition`'s recursive and iterative Tarjan implementations, `BlockPartition`'s in-place
partitioning, `ByteCompressedVec`'s cycle-following permutations, `IndexedSet`'s free-list/
generational-index bookkeeping) traced by hand and, where coverage was thin, exercised with new
randomized/reference-model tests — no defect found in either crate. `crates/xtask`, by contrast, is
broken: `cargo xtask publish` (`publish.rs`) cannot get past its own crate list without failing,
and the failure mode was never caught because nothing in CI ever runs it. `crates/rec-tests`'
existing 37-test suite (`cargo test -p merc_rec-tests`, 216s) all pass; light pass found nothing.

## Findings

### 1. `cargo xtask publish` fails immediately and always: two un-versioned dependencies and one nonexistent crate name in its hard-coded list — CONFIRMED

- **Location**: `crates/xtask/src/publish.rs:8-26` (the `crates` list inside `publish_crates`).
- **The failure scenario**: `publish_crates` iterates a hard-coded list of 17 crate names, running
  `cargo publish --dry-run -p <name>` for each, in what the code comment claims is dependency
  order. Three of the list's own entries make this fail, in list order:
  1. `merc_sabre` (list position 14) has a real, non-optional `[dependencies]` entry on
     `merc_typecheck` (`crates/sabre/Cargo.toml`), and `merc_typecheck`'s entry in the root
     `Cargo.toml`'s `[workspace.dependencies]` (`merc_typecheck = { path = "crates/typecheck" }`,
     line 158) has **no `version`**. `cargo publish` requires every non-dev dependency —
     including a merely-optional one — to carry a version when it is a path dependency; without
     one the manifest check fails before anything about the crate's own code is verified.
  2. `merc_ldd` (list position 15) is not a package in this workspace **at all** — there is no
     crate anywhere named `merc_ldd` (confirmed: no `crates/*/Cargo.toml` declares it, and
     `find . -iname '*ldd*' -maxdepth 3 -type d` finds only `examples/ldd`, an unrelated example
     directory). `cargo publish -p merc_ldd` fails with "package ID specification `merc_ldd` did
     not match any packages" (cargo's own suggestion: "a package with a similar name exists:
     `merc_lts`").
  3. `merc_symbolic` (list position 16) has a real (though feature-optional) `[dependencies]`
     entry on `merc_tools` (`crates/symbolic/Cargo.toml`: `merc_tools = { workspace = true,
     optional = true }`), and `merc_tools`'s workspace entry (`merc_tools = { path =
     "crates/tools" }`, root `Cargo.toml` line 157) likewise has **no `version`** — same failure
     mode as (1).

  Since the crates are dry-run in list order, `cargo xtask publish` cannot get past `merc_sabre`
  (its 14th of 17 entries) at all on the current tree — it never even reaches `merc_ldd` or
  `merc_symbolic` in a real run, but all three are independently broken.
- **Why nobody noticed**: `cargo xtask publish` is not invoked anywhere in CI — `grep -rln
  publish .github/workflows/*.yml` returns nothing — so this tooling's claim ("Runs `cargo publish
  --dry-run` for every library crate, in dependency order") has never actually been exercised.
- **Verification** (direct reproduction, matching the order the script would hit them):
  ```
  $ cargo publish --dry-run -p merc_sabre --allow-dirty
  error: failed to verify manifest at `.../crates/sabre/Cargo.toml`
  Caused by:
    all dependencies must have a version requirement specified when publishing.
    dependency `merc_typecheck` does not specify a version

  $ cargo publish --dry-run -p merc_ldd --allow-dirty
  error: package ID specification `merc_ldd` did not match any packages
  help: a package with a similar name exists: `merc_lts`

  $ cargo publish --dry-run -p merc_symbolic --allow-dirty
  error: failed to verify manifest at `.../crates/symbolic/Cargo.toml`
  Caused by:
    all dependencies must have a version requirement specified when publishing.
    dependency `merc_tools` does not specify a version
  ```
- **Status**: CONFIRMED, three independent instances of the same class of defect (a
  publish-order/dependency-completeness list that has drifted from the actual workspace).
- **Fix direction**: add `version = "..."` to the `merc_tools` and `merc_typecheck` workspace
  dependency entries in the root `Cargo.toml` (if they are meant to be published at all — if not,
  every crate that depends on them, i.e. `merc_sabre` and `merc_symbolic`, can never be published
  either, which is a design question for whoever owns the crate boundaries) and drop `merc_ldd`
  from the list (or add the crate, if one was meant to exist under that name).

## Checked and found correct

- **`crates/collections/src/scc_decomposition.rs`** (both `scc_decomposition`, recursive Tarjan,
  and `scc_decomposition_iterative`, an explicit-work-stack Tarjan): traced the iterative
  version's `disc`/`low`/`on_scc_stack` bookkeeping by hand, including the `disc[v] = 0` "queued"
  sentinel that shares its value with a real discovery time of 0 (confirmed not to cause
  misbehavior: the work stack is LIFO, so a freshly-queued child is always the very next thing
  popped and its placeholder `disc` value is overwritten before any other vertex could observe
  it), and the always-propagate-`low[s]`-to-parent step after a vertex is fully explored (matches
  the standard Tarjan invariant: once `s` is its own SCC root, `low[s] == disc[s] > disc[parent]
  >= low[parent]`, so the propagation is a no-op in that case rather than a bug). This file has
  **zero tests inside `merc_collections` itself** (`grep -n "mod tests" crates/collections/src/scc_decomposition.rs`
  finds nothing), but it is exercised extensively from `crates/reduction`'s
  `test_random_tau_scc_decomposition_compare_iterative` (100 iterations × 1000-state random LTS,
  cross-checking the recursive and iterative algorithms pairwise on every state pair) and
  `test_random_tau_scc_decomposition` (100 iterations, checks reachability-closure correctness) —
  both re-run here and pass (`cargo test -p merc_reduction scc_decomposition`, not shown above
  since `crates/reduction` is out of this phase's scope, but its existing suite was run as
  corroborating evidence rather than taken on faith).
- **`crates/collections/src/block_partition.rs`**: `split_block`'s in-place Lomuto-style
  partition (traced by hand against a worked 5-element example: predicate-matching elements end
  up correctly grouped in `[begin, begin+size)` regardless of swap order) and
  `from_indexed_partition`'s block-size counting / prefix-sum / scatter / `retain(!is_empty)`
  densification (already covered by an existing randomized test,
  `test_random_from_indexed_partition`, whose own comment records a *previously fixed* regression
  — "was sized by `num_of_blocks` instead of `num_of_elements`" — so this file already had a
  live regression test before this phase).
- **`crates/collections/src/compressed_vec.rs`**: `usize::bytes_required`'s bit-width formula
  (`(*self).max(1).ilog2() / u8::BITS + 1`) checked against every byte-boundary edge case
  (0, 255/256, 65535/65536, `usize::MAX`) both by hand and via the existing
  `test_bytes_required_boundaries`; `permute` (cycle-following scatter, doc-claimed result
  `v_p^-1`) and `permute_indices`/`permute_indices_fast` (cycle-following gather, doc-claimed
  result `v_p`) traced by hand through worked 3-cycle and 2-cycle (swap) examples and matched
  against their existing randomized tests, which cross-check `permute` vs. `permute_indices` vs.
  `permute_indices_fast` against each other and against a `Vec`-based reference.
- **`crates/collections/src/indexed_set.rs`** (`IndexedSet`/`SetIndex`): the free-list encoding
  (a self-loop, `Empty(index)` pointing at itself, marks the list's terminal node instead of a
  separate `None` sentinel) traced by hand through `remove`/`retain_mut`/`insert_into_table` and
  found consistent in both directions. This module had only one randomized test
  (`test_random_indexed_set_construction`, insert/remove but no `retain_mut`, and never checks
  `Index`/`IndexMut`), despite being used pervasively outside `collections` itself (`aterm`,
  `unsafety`, `symbolic`, `typecheck`, `vpg`, `explore`, `tools/mcrl2`'s FFI crates — none of which
  audit `IndexedSet`'s own internals, since they were reviewed for their *own* unsafe code in
  earlier phases, not this crate). Added
  `crates/collections/tests/indexed_set_stress_test.rs`
  (`test_random_indexed_set_survives_interleaved_insert_remove_retain`, 200 iterations × 200 random
  `insert`/`remove`/`retain_mut` operations each, cross-checked against a `HashMap` reference model
  after every single operation, including index stability across `retain_mut` and `Index`/
  `IndexMut`/iteration agreement) — passes with no failures
  (`cargo test -p merc_collections --test indexed_set_stress_test`).
- **`crates/collections/src/vecbag.rs` / `vecset.rs` / `vec_difference.rs`**: the shared sorted
  merge-based `Difference` iterator and `VecBag`/`VecSet`'s `is_subset` — standard sorted-merge
  algorithms, already covered by randomized tests against independent `Vec`/`HashSet`-based
  references; no defect found.
- **`crates/collections/src/remove_duplicates.rs`**: `scatter_into_buckets`'s counting-sort
  prefix-sum and `dedup_grouped`'s linear-scan/hash-map dual path (split at
  `HASH_DEDUP_THRESHOLD`) — both extensively randomized-tested already (200 iterations each,
  three separate test functions cross-checking against `HashSet`/`HashMap` references); traced the
  hash-path/linear-path boundary condition and found no off-by-one.
- **`crates/utilities/src/generational_index.rs`**: `GenerationalIndex`/`GenerationCounter`'s
  `PartialEq`/`Ord`/`Hash` sentinel handling (`generation == usize::MAX` for `Default`) — checked
  that `PartialEq`, `PartialOrd` and `Ord` all special-case the sentinel identically (a
  precondition for `Ord`/`PartialOrd` consistency), and that `Hash` and `PartialEq` are consistent
  with each other (both defined purely in terms of `index`, ignoring `generation`) — a mismatch
  there would silently break `HashMap`/`HashSet` usage.
- **`crates/utilities/src/tagged_index.rs`**: `TagIndex<T, Tag>`'s trait impls are all correctly
  bounded on `Tag` being a pure marker (no bound on `Tag` itself anywhere), so two `TagIndex`
  values with different `Tag` parameters are different Rust types and cannot be compared, ordered,
  hashed, or indexed against each other — the "state/action/priority index confusion" failure mode
  this type exists to prevent is a compile error, not a runtime one, for anything expressed purely
  in terms of `TagIndex` (a caller extracting `.value()` and re-wrapping it in the wrong `TagIndex`
  is outside what this type can statically prevent, but no such re-wrapping exists inside
  `collections`/`utilities` themselves).
- **`crates/utilities/src/permutation.rs`** (`is_valid_permutation`): out-of-bounds and
  duplicate-target detection both checked by hand and via its existing randomized test.
- **`crates/utilities/src/fixed_size_cache.rs` / `fixed_cache_policy.rs`**: traced
  `FixedSizeCache::insert`'s eviction path (evict-then-insert when `len() + 1 > maximum_size`) for
  a scenario where `replacement_candidate` could return `None` while the map is nonetheless full
  (which would let the cache silently grow past `maximum_size`) — did not find one: `FifoPolicy`/
  `LruPolicy`'s queues are kept in sync with the map's live keys by every `insert`/`touch`, and
  `replacement_candidate`'s self-cleaning `while let ... pop_front` loop tolerates any staleness
  that does creep in. `NoPolicy::replacement_candidate` always returns a candidate when the map is
  nonempty. The "overwrite counts as a touch, not a fresh `inserted`" comment (which would
  otherwise let a hot key's queue entries grow unboundedly) was checked against `insert`'s early
  `contains_key` branch and holds.
- **`crates/utilities/src/sharded_counter.rs`**: single-threaded and 8-thread × 100k-increment
  concurrent tests already present and re-run; per-thread `ThreadLocal<CachePadded<AtomicU64>>`
  shards mean no synchronization is needed between `add`/`increment` calls from different threads
  (no shared mutable state), so no data race is possible independent of the `Relaxed` ordering
  choice, which the doc comment correctly represents as carrying no cross-thread ordering
  guarantee.
- **`crates/utilities/src/span.rs` / `source_map.rs`**: `SourceMap`'s one-byte gap between files
  (so an end-exclusive span landing exactly at a file's length resolves to that file, not the
  next) and `Span::render`'s multi-file/single-file header suppression, multibyte-safe
  column counting, and cross-newline-span clamping — all already covered by dedicated tests
  (including two tests explicitly documented as regression tests for past bugs in this exact
  area), re-run and passing.
- **`crates/utilities/src/snapshot.rs`**: `ensure_snapshot_version`'s atomic
  write-to-temp-then-rename (per-process-unique filename via `std::process::id()`) correctly
  avoids the torn-write race between concurrent `nextest` processes updating the same `VERSION`
  file; `check_snapshot`'s "regenerate rather than compare" behavior on a version bump is
  documented and intentional (not a defect), though note this means a version bump makes a first
  run always pass by silently overwriting the golden file rather than flagging the diff — a
  deliberate design tradeoff of this snapshot helper, not something demonstrated to produce a
  wrong test result.
- **`crates/xtask/src/sanitizer.rs`**: `address_sanitizer`/`thread_sanitizer` do set the
  documented `RUSTFLAGS`/`RUSTDOCFLAGS`/`CFLAGS`/`CXXFLAGS` and add `-Zbuild-std --target
  <host-triple>`; `add_target_flag`'s architecture/OS detection matches the two OSes
  (Linux/macOS) the doc comment claims support for, and its suppression files
  (`data/leak_sanitizer.suppress`, `data/thread_sanitizer.suppress`) exist and are wired via
  `LSAN_OPTIONS`/`TSAN_OPTIONS`. `env!("CARGO_MANIFEST_DIR")` correctly resolves per-workspace
  (there is a *separate* `xtask` crate at `tools/mcrl2/crates/xtask` with its own compiled-in
  `CARGO_MANIFEST_DIR`, which is why CI's second sanitizer step, run with `working-directory:
  tools/mcrl2`, still finds its own suppress files rather than the root ones) — initially looked
  like a bug, was not.
- **`crates/xtask/src/coverage.rs` / `discover_tests.rs` / `package.rs` / `tool_testing.rs`**:
  light pass, no defect found; `package.rs`'s cross-workspace binary collection correctly relies
  on `tools/gui` and `tools/mcrl2` sharing the root `target/` directory via their own
  `.cargo/config.toml` `target-dir = "../../target"` (verified those files exist and say so), and
  `discover_tests.rs`'s "error if a glob pattern matches nothing" guards against exactly the kind
  of silent-empty-list mistake that would otherwise hide a typo'd pattern.
- **`crates/rec-tests`**: light pass. `parse_rec_impl`'s include-cycle guard (a `visited: &mut
  HashSet<PathBuf>` threaded through recursive calls, inserted-before-recursing) correctly
  prevents infinite recursion on a cyclic or diamond include graph. `load_rec_from_strings`
  silently ignoring a spec's header `include` directives (no base directory to resolve them
  against) is explicitly documented as intentional, not a defect. Full existing suite
  (`cargo test -p merc_rec-tests`, 37 tests including REC-file-driven step-count regression
  tests) passes, 216s.

## Tests added

- `crates/collections/tests/indexed_set_stress_test.rs`:
  `test_random_indexed_set_survives_interleaved_insert_remove_retain` — 200 iterations of 200
  interleaved `insert`/`remove`/`retain_mut` operations on `IndexedSet<u32>`, cross-checked against
  a `HashMap` reference model after every operation (length, `contains`, `get`, `Index`, and
  iteration all checked). Passes on the current tree (no defect found; added for coverage this
  crate lacked). Run with: `cargo test -p merc_collections --test indexed_set_stress_test`.
- `crates/xtask/tests/publish_dependencies_have_versions_test.rs`:
  `test_publish_crates_dependencies_all_have_versions` — statically re-derives, from the actual
  `Cargo.toml` files (no network access needed, unlike shelling out to `cargo publish` directly),
  whether every crate `publish_crates`' hard-coded list tries to dry-run-publish has a
  workspace-declared name and only `[dependencies]` that carry a version. **Fails on the current
  tree** with exactly the three problems in Finding 1 (`merc_sabre -> merc_typecheck`, `merc_ldd`
  not a package, `merc_symbolic -> merc_tools`); would pass once the root `Cargo.toml` and/or the
  `publish.rs` crate list are fixed. Run with: `cargo test -p xtask --test
  publish_dependencies_have_versions_test`. (Its hard-coded `PUBLISH_CRATES` list is a manual copy
  of `publish_crates`'s own list — that function and its list are `pub(crate)`/local, so they
  cannot be imported from an external test file without changing production code; keep the two in
  sync if either changes.)

## Verification

```
cargo test -p merc_collections                              # 19 unit + 1 stress test, all pass
cargo test -p merc_collections --test indexed_set_stress_test  # new test, passes
cargo test -p merc_utilities                                 # 25 unit + 1 doctest, all pass
cargo test -p xtask                                           # new test FAILS (documents Finding 1)
cargo test -p merc_rec-tests                                  # 37 tests, all pass (216s)

cargo publish --dry-run -p merc_sabre --allow-dirty            # fails: merc_typecheck has no version
cargo publish --dry-run -p merc_ldd --allow-dirty               # fails: no such package
cargo publish --dry-run -p merc_symbolic --allow-dirty          # fails: merc_tools has no version
grep -rln publish .github/workflows/*.yml                      # (no output - never run in CI)
grep -rn unsafe crates/utilities/src crates/collections/src \
  crates/rec-tests/src crates/xtask/src                        # forbid attributes + 1 string literal only
```
