# Kani coverage for `crates/collections`

`crates/collections` is `#![forbid(unsafe_code)]` (confirmed:
`grep -rn unsafe crates/collections/src` finds only the `forbid` attribute
itself), so Kani is not needed here for soundness the way it is in
`crates/aterm`/`crates/sharedmutex`/`crates/unsafety`. Phase D
(`phase-4-foundation-devtools.md`) already reviewed this crate as sound via
heavy manual tracing plus randomized/reference-model stress testing (notably
`crates/collections/tests/indexed_set_stress_test.rs` for `IndexedSet`'s
free-list bookkeeping), but recorded no Kani-level coverage for it — the
`review/README.md` baseline's "Kani proof harnesses" line lists only
`crates/unsafety`, `crates/number`, `crates/macros`, `crates/sharedmutex`,
`crates/aterm`.

This note adds bounded-model-checking coverage for a handful of this crate's
core algorithmic invariants: Kani exhaustively explores every input up to a
small size instead of randomly sampling them, which is strictly stronger
evidence than Phase D's randomized tests for the same code, even though the
code itself is ordinary safe Rust.

## Harnesses added

### `crates/collections/src/block_partition.rs` (0 → 1 harness)

**`split_block_refines_correctly_and_preserves_elements`** — proves
`BlockPartition::split_block`'s core partition-refinement invariant,
exhaustively over all `2^5` predicates on a 5-element single-block partition:

- a predicate matching all or none of the elements causes no split (matches
  the doc comment);
- otherwise, every element left in the original block satisfies the
  predicate and every element moved to the new block does not;
- the in-place Lomuto-style swap that achieves this is a genuine permutation
  of the original elements — none lost, none duplicated across the two
  blocks.

Phase D's own manual trace of `split_block` ("predicate-matching elements end
up correctly grouped ... regardless of swap order", checked against one
worked 5-element example) is now backed by an exhaustive proof over every
predicate on that same element count, not one hand-picked example.

Run (from `crates/collections`):

```
cargo kani --harness split_block_refines_correctly_and_preserves_elements
```

Output:

```
SUMMARY:
 ** 0 of 3043 failed (145 unreachable)

VERIFICATION:- SUCCESSFUL
Verification Time: 19.370472s

Manual Harness Summary:
Complete - 1 successfully verified harnesses, 0 failures, 1 total.
```

No counterexample found.

### `crates/collections/src/indexed_set.rs` (0 → 2 harnesses)

This is the exact free-list/generational-index bookkeeping Phase D traced by
hand and added a new randomized stress test for
(`indexed_set_stress_test.rs`'s
`test_random_indexed_set_survives_interleaved_insert_remove_retain`, 200
iterations x 200 operations each, HashMap-reference-checked). The two
harnesses below give that same mechanism exhaustive, small-bounded coverage
instead of a random sample — directly the "a freed-then-reused index is
never aliased" target this task called out.

Both harnesses call the private `insert_into_table` directly and replicate
`remove`'s/`retain_mut`'s exact three-statement free-list update
(`free_slot`, a `#[cfg(kani)]`-only helper) rather than going through the
public `insert`/`remove` API. This is deliberate: an earlier version drove
the identical scenario through the real public API (`insert`/`remove`, which
hash through `hashbrown::HashTable`) and did not terminate within 5 minutes
on a single 3-element scenario. `HashTable::find`/`insert_unique`'s
SIMD-oriented probing is orthogonal to the free-list invariant being proved
and is not itself practically model-checkable by this Kani version in
reasonable time — the same kind of "real code, not viable for Kani" gap
`phase-4-kani-extension.md` already documented for `std::sync::Mutex`'s
futex retry loop. Isolating the free-list mechanics by direct field access
(same technique `recursive_lock.rs`'s existing harness uses to bypass its
own `Mutex`) sidesteps this without weakening what's proved about the
free-list itself.

**`freed_slot_is_reused_without_aliasing_other_entries`** — for two live
inserts (`v1`, `v2`, arbitrary `u8`) followed by freeing `v1`'s slot and
inserting a third value `v3`: the freed slot is reused (not a freshly
appended one), the reused slot resolves to exactly `v3`, and `v2`'s slot is
untouched and does not alias `v3`'s.

```
cargo kani --harness freed_slot_is_reused_without_aliasing_other_entries
```

```
SUMMARY:
 ** 0 of 872 failed (76 unreachable)

VERIFICATION:- SUCCESSFUL
Verification Time: 2.6442966s

Manual Harness Summary:
Complete - 1 successfully verified harnesses, 0 failures, 1 total.
```

**`multiple_freed_slots_are_reused_in_lifo_order`** — extends the above to
two interleaved frees (five arbitrary `u8` values): proves the free-list
*chain*, not just a single freed slot, is walked correctly — slots are
reused in last-freed-first order (matching `insert_into_table` popping
`self.free` as a stack) — and that the two reused slots plus the one
untouched slot never end up aliasing each other.

```
cargo kani --harness multiple_freed_slots_are_reused_in_lifo_order
```

```
SUMMARY:
 ** 0 of 892 failed (76 unreachable)

VERIFICATION:- SUCCESSFUL
Verification Time: 5.6074505s

Manual Harness Summary:
Complete - 1 successfully verified harnesses, 0 failures, 1 total.
```

No counterexample found by either harness: the free-list bookkeeping Phase D
traced by hand holds exhaustively for these bounded scenarios too.

### `crates/collections/src/indexed_partition.rs` (0 → 1 harness)

**`set_block_and_num_of_blocks_match_reference_model`** — proves
`IndexedPartition::set_block`/`num_of_blocks`'s bookkeeping against a plain
array-based reference model, exhaustively over every combination of 3
`set_block` calls with element and block-number both bounded to a 4-element
domain: `block(i)` always returns the most recently set block number for
`i` (or block 0, `new`'s initial state, if never set), and `num_of_blocks()`
always equals one plus the highest block number ever passed to `set_block` —
the "block numbers are dense" contract `set_block`'s own doc comment claims,
which `BlockPartition::from_indexed_partition` (already covered by an
existing randomized regression test per Phase D) depends on as an accurate
upper bound when sizing its own block array.

```
cargo kani --harness set_block_and_num_of_blocks_match_reference_model
```

```
SUMMARY:
 ** 0 of 415 failed (6 unreachable)

VERIFICATION:- SUCCESSFUL
Verification Time: 2.1150737s

Manual Harness Summary:
Complete - 1 successfully verified harnesses, 0 failures, 1 total.
```

No counterexample found.

## Investigated and not landed (not defects — tooling/environment limits)

### `scc_decomposition.rs`'s Tarjan-SCC correctness — attempted, reverted

The highest-value target on paper: both `scc_decomposition` (recursive) and
`scc_decomposition_iterative` (explicit work-stack) checked against an
independent brute-force reference (Floyd-Warshall transitive closure) on
every directed graph on a small fixed vertex count, via a minimal `Graph`
trait implementation with a fully symbolic adjacency matrix. This is
genuinely new-shape coverage beyond what Phase D's manual trace and
`crates/reduction`'s existing 1000-state randomized cross-checks give (an
exhaustive small-bounded proof against a reference definition, rather than
either a hand trace or a random sample), so it was worth the attempt.

It was not landed, for two compounding reasons, both confirmed independently
of each other:

1. **A real Kani/CBMC (this toolchain: Kani 0.68.0, CBMC 6.11.0) limitation
   with certain `std` iterator-adapter internals**, reproduced identically in
   two unrelated places:
   - `scc_decomposition_iterative`'s own `graph.outgoing_edges(s_vertex).skip(offset)`
     call: `Skip::next` delegates to the default `nth`, whose default
     delegates to the unstable `advance_by`/`try_fold`/`ControlFlow`
     machinery. A custom `EdgeIter` with the naive derived behavior did not
     unwind (CBMC's `Unwinding loop ... iteration N` trace grew unboundedly
     past the configured bound with no sign of terminating) even at 2
     vertices. Overriding `nth` directly on `EdgeIter` to bypass this path
     was necessary and effective in isolation (confirmed separately below,
     via `indexed_partition.rs`'s dropped `with_subset` harness hitting the
     *same* underlying mechanism through `IndexedPartition::iter_elements`'s
     `.filter_map()`, whose `FilterMap::next` similarly delegates to
     `find_map`).
   - Even after that fix, a full run (`scc_decomposition_iterative` *and*
     the simpler, non-`.skip()`-using recursive `scc_decomposition`, both at
     just 2 vertices) did not complete `cargo kani`'s SAT-solving phase
     within a 10-minute budget, run repeatedly.
2. **Severe, sustained multi-tenant resource contention** in this sandbox
   for the entire session (`uptime`'s load average held at ~48-50 on a
   4-core machine throughout; `dmesg` showed the shared cgroup's OOM killer
   actively killing unrelated processes, e.g. a concurrent `cc1plus`
   invocation from another session's build, mid-session). Other sessions
   were independently running `cargo kani` against `crates/aterm` at the
   same time (confirmed via `ps aux`), competing for the same pinned-nightly
   `kani-compiler`/CBMC toolchain and physical cores.

Reason 2 alone explains most of the individual timeouts seen while
developing every harness in this note (including the ones that ultimately
passed, on a retry) and makes it impossible to cleanly attribute this
particular harness's non-termination to reason 1 alone with full confidence
— but reason 1 is independently confirmed: the pathological unwind pattern
was directly observed and reproduced by a change (removing `.skip()`,
overriding `nth`) that measurably helped, and the same failure signature
(`find_map`/`try_fold`/`ControlFlow` in the unwind trace) recurred in a
completely unrelated harness (`with_subset`, below) that hit the same wall
through a different production function.

Given both factors, and that this task's instructions are explicit that an
unconfirmed harness must not be landed, the `scc_decomposition.rs` changes
were reverted rather than committed half-verified. This is recorded here as
a confirmed non-viable target for *this specific Kani/CBMC version in this
environment*, in the same spirit `phase-4-kani-extension.md` recorded
`std::sync::Mutex`'s futex retry loop and `sabre_compiling`'s FFI/dylib
surface as non-viable — not a defect in `scc_decomposition.rs`'s own logic,
which Phase D already traced by hand and which `crates/reduction`'s existing
randomized tests continue to cross-check at real scale (100 iterations x
1000-state LTS).

### `indexed_partition.rs`'s `with_subset` construction invariant — attempted, dropped

A second harness, `with_subset_includes_exactly_the_given_indices`, aimed to
prove `IndexedPartition::with_subset`'s construction invariant (every
included index resolves to block 0, every excluded index resolves to the
`NOT_IN_PARTITION` sentinel, `num_of_blocks()` is 1 iff any index was
included) exhaustively over every subset of a 4-element domain.

Its first version used `IndexedPartition::iter_elements()` as a cross-check.
That version hit the identical `find_map`/`try_fold` non-termination pattern
described above (`FilterMap::next` delegates to `find_map`), confirming the
same underlying Kani/CBMC limitation from a second, independent production
call site. Rewritten to avoid every iterator-adapter default (`.filter()`,
`.any()`, `iter_elements()`) in favor of plain `for` loops and a manually
built `Vec<usize>`, it consistently reached CBMC's SAT-solving phase in
~8 seconds of `Runtime Symex`/`Runtime Convert SSA` (824 VCCs after
simplification — a small problem) across 5 separate attempts, but the actual
SAT-solving step itself did not complete within the remaining budget in any
of them, plausibly (though not provably, given point 2 above) pure resource
contention rather than a further tooling limitation, since 824 clauses is
not intrinsically hard for a modern SAT solver. It was dropped rather than
landed unconfirmed. `with_subset`'s behavior for this same invariant is
already covered indirectly by `crates/collections/tests/indexed_set_stress_test.rs`-style
existing tests elsewhere in the crate and by the `block()`-only assertions
already proved in `set_block_and_num_of_blocks_match_reference_model` above
for the closely related `set_block` path (both write through the same
`partition: Vec<BlockIndex>` field).

## CONFIRMED defects found by a harness

None. All 4 landed harnesses verified successfully with no counterexample.
This note's value is new bounded-model-checking coverage of algorithmic
invariants Phase D previously established only by hand-tracing and
randomized/reference-model testing — not a new bug report. The one concrete
finding to come out of this work is the Kani/CBMC tooling limitation
documented above (certain `std` iterator-adapter default implementations —
`Skip::nth`'s `advance_by` fallback, `FilterMap::next`'s `find_map`
fallback — do not unwind practically under Kani 0.68.0/CBMC 6.11.0), which
is useful for anyone adding further Kani harnesses to this codebase: prefer
hand-written `Iterator` impls with an explicit `next()` (and an explicit
`nth()` override if `.skip()`/`.nth()` will be called on them) over relying
on adapter chains (`.filter().map()`, `.filter_map()`, `.skip()`) inside
code that must be Kani-model-checked, even when those same adapters are
completely fine in ordinary (non-Kani) compilation and in the plain `cargo
test` suite.

## Checked and found correct (re-confirmed, not new claims)

- `BlockPartition::split_block`'s in-place partition and
  `IndexedSet`'s free-list reuse mechanics: Phase D's hand-traced
  conclusions (`phase-4-foundation-devtools.md`, "Checked and found
  correct") both hold under exhaustive small-bounded proof, not just the
  worked examples and randomized samples already on record.

## Tests / proofs added

| File | Harness | Run with |
|---|---|---|
| `crates/collections/src/block_partition.rs` | `split_block_refines_correctly_and_preserves_elements` | `cd crates/collections && cargo kani --harness split_block_refines_correctly_and_preserves_elements` |
| `crates/collections/src/indexed_set.rs` | `freed_slot_is_reused_without_aliasing_other_entries` | `cd crates/collections && cargo kani --harness freed_slot_is_reused_without_aliasing_other_entries` |
| `crates/collections/src/indexed_set.rs` | `multiple_freed_slots_are_reused_in_lifo_order` | `cd crates/collections && cargo kani --harness multiple_freed_slots_are_reused_in_lifo_order` |
| `crates/collections/src/indexed_partition.rs` | `set_block_and_num_of_blocks_match_reference_model` | `cd crates/collections && cargo kani --harness set_block_and_num_of_blocks_match_reference_model` |

No production code was changed; all additions are `#[cfg(kani)] mod
verification` blocks, following the exact pattern already used in
`crates/unsafety/src/{slice_dst,freelist,protection_set,index_edge}.rs`,
`crates/number/src/{power_of_two,bits_for_value}.rs`, `crates/aterm`'s and
`crates/sharedmutex`'s existing harnesses. `crates/collections/Cargo.toml`
already carried the `unexpected_cfgs = { level = "allow", check-cfg =
['cfg(kani)'] }` lint override needed for this (added by an earlier phase),
so no `Cargo.toml` change was needed here. Each harness above was confirmed
individually with a completed, non-timed-out `cargo kani --harness <name>`
run showing `VERIFICATION:- SUCCESSFUL`; a combined `cargo kani` (all 4
harnesses, no `--harness` filter) was attempted repeatedly but did not
complete within this session's available time due to the environment
contention described above — the individual per-harness runs are the
verification evidence this note relies on. `cargo test -p merc_collections`
and `cargo clippy -p merc_collections --all-targets` were not re-run as part
of this note (no non-`#[cfg(kani)]` code was touched); the existing
`indexed_set_stress_test.rs` and unit-test suites Phase D already covers are
unaffected by these additions.
