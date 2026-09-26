# Phase 4 review: mcrl2 non-FFI algorithmic logic

Scope: `tools/mcrl2/crates/merc_lps/src/{cfg_lps,convert_data_specification,explore_symbolic,io}.rs`,
`tools/mcrl2/crates/merc_pbes/src/{bsgs,cfg_srf,clone_iterator,explore_symbolic_srf,permutation}.rs`,
`tools/mcrl2/crates/mcrl2-macros/`. Excludes `explore_srf.rs`, `explore_pbes.rs`, `quotient_lps.rs`,
`explore_common.rs`, `explore_explicit.rs`, `symmetry.rs`, `graph_symmetry.rs` (Phase 1b/3) and the
FFI/`unsafe` boundary (Phase 1b). `mcrl2-macros` is `#![forbid(unsafe_code)]` and no other file in
scope contains `unsafe`, so this is a pure algorithmic-logic review — `unsafe-verify` does not apply.

**Verdict**: mostly sound. `bsgs.rs` — the highest-priority file, a from-scratch Schreier–Sims/BSGS
implementation with a hand-rolled pruned-transversal canonicalization walk — held up under close
reading of the group-theory invariants (strictly-increasing base points, Schreier generator
correctness, the pruning soundness argument) and is already exercised by extensive randomized tests
cross-checked against both an in-crate BFS oracle and real GAP output; no defect found there. The
`cfg_lps.rs`/`cfg_srf.rs` control-flow-pruning wrappers, `clone_iterator.rs`, and the
`explore_symbolic*.rs` glue code are also sound. `permutation.rs`, however, has a real, confirmed
defect: its own documented invariant ("mapping should not contain identity mappings") is violated by
both of its public string parsers on ordinary, spec-valid input, causing a panic in debug builds
(including plain `cargo test`) and a silently invariant-violating `Permutation` in release builds,
reachable directly from the `merc-pbes` CLI's user-supplied `--generators` flag.

## Finding 1 — `Permutation::from_cycle_notation`/`from_mapping_notation` panic (debug) / silently
break their own invariant (release) on ordinary fixed-point notation — CONFIRMED

`tools/mcrl2/crates/merc_pbes/src/permutation.rs:120-133` (`from_cycle_notation`) and `:41-88`
(`from_mapping_notation`), both funnelling into `from_mapping`'s invariant check at `:18-38`:

```rust
pub fn from_mapping(mut mapping: Vec<(usize, usize)>) -> Self {
    debug_assert!(is_valid_permutation(&mapping), ...);
    mapping.sort_unstable_by_key(|(d, _)| *d);
    debug_assert!(mapping.iter().is_sorted(), ...);
    debug_assert!(
        mapping.iter().all(|(from, to)| from != to),
        "Mapping should not contain identity mappings."
    );
    ...
}
```

A singleton cycle such as `"(5)"` is ordinary cycle notation naming a fixed point (mCRL2/GAP-style
notation routinely uses this, e.g. to explicitly list a point that's part of the considered index
set but happens not to move). `from_cycle_notation`'s cycle-parsing loop builds
`mapping.push((from, to))` for every adjacent pair in the cycle including the wraparound one, so a
length-1 cycle produces the pair `(5, 5)` — an identity mapping — and passes it straight to
`from_mapping` without filtering it out first. `is_valid_permutation` only checks bijectivity
(domain and image are equal sets with no duplicates), so it does not reject the identity pair
either; the `debug_assert!` fires unconditionally.

The identical defect is reachable through `from_mapping_notation`: `"[5->5]"` is syntactically valid
mapping notation for the same fixed point and hits the exact same `debug_assert!`, since
`from_mapping_notation` likewise does nothing to filter out `from == to` pairs before calling
`from_mapping`.

Both parsers are used directly on **unvalidated user input** from the `merc-pbes` CLI:

```rust
// tools/mcrl2/pbes/src/main.rs:704-715
fn parse_generators(strs: &[String]) -> Result<Vec<Permutation>, MercError> {
    strs.iter()
        .map(|s| {
            let s = s.trim();
            if s.starts_with('[') {
                Permutation::from_mapping_notation(s)
            } else {
                Permutation::from_cycle_notation(s)
            }
        })
        .collect()
}
```

So a user invoking `merc-pbes`'s `--generators` option (`build_canonicaliser_from_user_generators`)
with a generator string containing an explicit fixed point — `(5)` or `[5->5]`, both of which read
as perfectly reasonable permutation notation — panics in any debug build (which includes the
default `cargo test`/`cargo nextest run` profile used in CI) instead of getting either a parsed
identity permutation or a clean `MercError`. In a release build the `debug_assert!` compiles out and
the returned `Permutation`'s `mapping` silently retains the identity pair, contradicting the type's
own documented and enforced (in debug) invariant.

Verified independently with two new tests, both failing on the current tree:

```
cd tools/mcrl2
cargo test -p merc_pbes --test permutation_test from_cycle_notation_accepts_singleton_cycle_without_panicking
cargo test -p merc_pbes --test permutation_test from_mapping_notation_accepts_identity_pair_without_panicking
```

```
thread 'from_cycle_notation_accepts_singleton_cycle_without_panicking' panicked at crates/merc_pbes/src/permutation.rs:28:9:
Mapping should not contain identity mappings.
  2: merc_pbes::permutation::Permutation::from_mapping
  3: merc_pbes::permutation::Permutation::from_cycle_notation
test from_cycle_notation_accepts_singleton_cycle_without_panicking ... FAILED
```

```
thread 'from_mapping_notation_accepts_identity_pair_without_panicking' panicked at crates/merc_pbes/src/permutation.rs:28:9:
Mapping should not contain identity mappings.
  2: merc_pbes::permutation::Permutation::from_mapping
  3: merc_pbes::permutation::Permutation::from_mapping_notation
test from_mapping_notation_accepts_identity_pair_without_panicking ... FAILED
```

Both tests would pass once fixed: they assert only that the parse succeeds and that the resulting
`Permutation` is the identity (`is_identity()`), which is the value both strings unambiguously
denote — no other behavior is asserted, so a fix that filters `from == to` pairs before constructing
the mapping (in either parser, or by relaxing `from_mapping`'s invariant to tolerate — and drop —
literal identity pairs) satisfies them.

Why this was not caught by the existing randomized round-trip tests
(`test_random_cycle_notation`/`test_random_mapping_notation` in `permutation_test.rs`): both
generators explicitly build a *derangement* (`filter(|(x, y)| x != y)` on a shuffled domain), so an
identity pair can never appear in the randomly generated mapping that gets serialized and re-parsed
— the singleton-cycle/identity-pair path is structurally excluded from that coverage.

Direction of a fix: `from_cycle_notation` should skip a cycle of length 1 the same way it already
skips an empty cycle (`if cycle_content.trim().is_empty() { continue; }`); `from_mapping_notation`
and the manual pair-parsing loop should likewise drop (or reject with a clear error) any `from == to`
pair before validating/constructing the mapping.

## Checked and found correct

- **`bsgs.rs`** (highest priority, 1260 lines): read in full.
  - `DensePermutation::compose`/`inverse`/`apply_to_vec`: composition order, inverse construction,
    and the position-vs-value permuting distinction in `apply_to_vec` all check out algebraically
    and match the file's own extensive tests (`dense_perm_compose_and_inverse`,
    `dense_perm_apply_to_vec`).
  - `schreier_sims_chain`/`compute_orbit_transversal`/`schreier_generators`: verified the standard
    Schreier–Sims argument by hand — (a) the smallest point moved by *any* individual generator of a
    group equals the smallest point moved by the whole group (since every generator, and hence every
    word in the generators, fixes any point none of them moves), which is exactly what
    `bsgs_schreier_sims`'s `(0..n).find(|&x| gens.iter().any(|g| g.apply(x) != x))` computes; (b)
    since `schreier_generators` iterates every orbit point *and* every generator, Schreier's lemma
    gives that the produced set generates `Stab_G(base_point)` exactly; (c) by induction, each level's
    stabilizer therefore fixes every point below its base point, so the base points the algorithm
    picks are provably strictly increasing — the invariant `canonicalize_into`'s `debug_assert!`
    relies on. Also checked the `u_x · s · u_{s(x)}^{-1}` composition order against
    `DensePermutation::compose`'s actual (right-to-left) semantics; it fixes `base_point` as claimed.
  - `Bsgs::canonicalize_into`'s pruned-transversal walk: checked the scoring/survivor-filtering logic
    at both base and non-base positions against the algorithm's own doc comment's soundness argument;
    matches. This is also the part with by far the heaviest existing test coverage in the file
    (naive-BFS-oracle comparison, GAP lex-min comparison via `Minimum(List(Elements(G), ...))`,
    idempotence, orbit-invariance, and a specific regression for cosets tying at a base point while
    disagreeing on a skipped position) — did not find a case that breaks it.
  - GAP output parsing (`parse_gap_bsgs_output`, `parse_bsgs_list`, `parse_schreier_level`,
    `parse_transversal_list`, `parse_gap_perm`, `split_top_level`): straightforward, no defect found;
    1-based→0-based conversions are consistent throughout.
- **`cfg_lps.rs`**/**`cfg_srf.rs`**: structurally identical control-flow-pruning wrappers (LPS vs. SRF
  PBES); the `1 +` state-index offset in `cfg_srf.rs` (state layout `[equation_index, params...]`)
  is applied consistently with `cfg_lps.rs`'s unoffset version (state layout `[params...]`) — no
  off-by-one found.
- **`clone_iterator.rs`**: the `CloneIterator`/`Box<dyn CloneIterator>::clone()` object-safety
  pattern is standard and correctly wired (`clone_boxed` → `Box::new(self.clone())` →
  `Iterator::clone()` on the concrete type).
- **`explore_symbolic.rs`**, **`explore_symbolic_srf.rs`**, **`convert_data_specification.rs`**,
  **`io.rs`**: glue code assembling `SymbolicLps`/`SymbolicLts`/`SymbolicParityGame` from FFI
  conversions and diagram-order permutations; traced the `order`/`process_parameters`/
  `read_indices`/`write_indices` index translations in `explore_lps_symbolic_to_sym` and the
  `level`/`blocks` construction in `explore_pbes_symbolic_game` — consistent, no defect found.
  These files are otherwise dominated by `#[cfg(test)]` integration tests gated on
  `MCRL2_PATH`/real GAP/mCRL2 binaries not exercised in this environment.
- **`mcrl2-macros`**: `#![forbid(unsafe_code)]`, no unsafe present. Read the whole
  `mcrl2_derive_terms` proc-macro implementation; the codegen for the `<Name>`/`<Name>Ref` pair
  (constructors, `Deref`/`Borrow`/`Markable`/`Debug` impls, and the impl-block duplication for the
  `Ref` type) matches how it's consumed in the already-reviewed `tools/mcrl2/crates/mcrl2` wrapper
  types; no functional defect found, though its own `#[cfg(test)] mod tests::test_macro` makes no
  assertions (only prints the generated tokens), so it would not catch a codegen regression itself —
  noted, not filed as a defect since it's a coverage gap rather than a demonstrated wrong result.
- **`permutation.rs`** (beyond Finding 1): `is_valid_permutation`, `Permutation::value`/`domain`/
  `is_identity`/`max_point`/`concat`, `permutation_group`/`permutation_group_size`, and
  `impl Display for Permutation` (including its `visited[value]`-vs-`visited[start]` cycle-marking,
  which is equivalent for a genuine permutation since `π(start)` always lies in the same cycle as
  `start`) — all checked and correct.

## Tests added

- `tools/mcrl2/crates/merc_pbes/tests/permutation_test.rs`:
  - `from_cycle_notation_accepts_singleton_cycle_without_panicking` — Finding 1 (cycle-notation path).
  - `from_mapping_notation_accepts_identity_pair_without_panicking` — Finding 1 (mapping-notation path).

  Run with:
  ```
  cd tools/mcrl2
  cargo test -p merc_pbes --test permutation_test from_cycle_notation_accepts_singleton_cycle_without_panicking
  cargo test -p merc_pbes --test permutation_test from_mapping_notation_accepts_identity_pair_without_panicking
  ```
