# Phase 4 — Kani coverage extension for `crates/aterm`, `crates/sharedmutex`, `crates/sabre`, `crates/sabre_compiling`

## Scope and starting state (re-verified independently, not taken from the README)

The README's "Existing kani proof harnesses" line (only `crates/unsafety` and
`crates/number`) is stale relative to this branch's tip: `crates/aterm` (5 harnesses,
`symbol.rs`/`storage/aterm_storage.rs`/`storage/shared_term.rs`) and `crates/sabre`
(3 harnesses, `utilities/term_stack.rs`) already had Kani coverage added by the phase-1
foundation-unsafe review, landed on `skills` before this branch forked from it. I
re-confirmed this by grepping for `cfg(kani)` across `crates/`/`tools/` and running
`cargo kani` in each of the four target crates before making any change:

| Crate | Harnesses before this phase | Confirmed by |
|---|---|---|
| `crates/aterm` | 5 (passing) | `cd crates/aterm && cargo kani` |
| `crates/sabre` | 3 (passing) | `cd crates/sabre && cargo kani` |
| `crates/sharedmutex` | 0 | `grep cfg(kani) crates/sharedmutex/src/*.rs` — none; `[package.metadata.kani]` block already present in `Cargo.toml` from the phase-1 review's (abandoned) attempt |
| `crates/sabre_compiling` | 0 | `cd crates/sabre_compiling && cargo kani` → "No proof harnesses (functions with #[kani::proof]) were found to verify." |

`crates/sharedmutex` and `crates/sabre_compiling` were the two genuine gaps. Both gaps
were already investigated and explained by the phase-1 review
(`phase-1-foundation-unsafe-sharedmutex-unsafety.md`, `phase-1-foundation-unsafe-sabre.md`):
`sharedmutex`'s `RecursiveLock`/`BfSharedMutex` guards need the real, futex-based
`std::sync::Mutex` to be locked to construct a write guard or (along the contended
branch) a read guard, and CBMC cannot bound `std::sync::Mutex`'s internal retry loop —
even along a provably uncontended path — without exhausting memory; `sabre_compiling`'s
own unsafe surface is entirely FFI/dylib-loading and code-generation-that-runs-later,
neither of which Kani can model.

This phase's job was to find the highest-value **new**, genuinely non-vacuous harnesses
still obtainable within those constraints, and to independently re-confirm rather than
just repeat the prior conclusion for `sabre_compiling`.

## New harnesses added

### `crates/sharedmutex` (0 → 2 harnesses)

Both harnesses deliberately avoid ever calling `BfSharedMutex::read`/`write`/`try_write`/
`acquire_shared`/`RecursiveLock::read_recursive`/`write` themselves — every one of those
locks the real `shared.other: std::sync::Mutex<..>`, which is the exact code path the
phase-1 review found unviable. They also had to be written to **`std::mem::forget`** the
`BfSharedMutex`/`RecursiveLock` value they construct: an earlier version of both harnesses
let the value drop normally at the end of the function, and `Drop for BfSharedMutex`
*itself* unconditionally locks `shared.other` to deregister the instance — CBMC hit the
identical `Mutex::lock_contended` futex-retry-loop non-termination the phase-1 review
already documented, confirming that finding a second, independent way (see "Dead end
found and worked around" below).

1. **`crates/sharedmutex/src/bf_sharedmutex.rs`**, `data_ptr_is_non_null_and_round_trips_reads_and_writes`
   — proves `BfSharedMutex::data_ptr()` (`self.shared.object.get()`) is never null and that a
   read/write through it behaves like an ordinary `&T`/`&mut T`, for arbitrary `u64`. This is
   the exact raw-pointer expression every guard's `Deref`/`DerefMut` performs once the
   busy-forbidden protocol has granted access (`BfSharedMutexReadGuard::deref`,
   `BfSharedMutexWriteGuard::deref`/`deref_mut`, and the non-loom branch of
   `RecursiveLockReadGuard::deref`), and every one of those `# Safety` comments asserts
   non-nullity and soundness without a machine check backing it — this now backs it.
   `BfSharedMutex::new` only allocates two `Arc`s and constructs an *unlocked* `Mutex`, so it
   needed none of the locking machinery.

2. **`crates/sharedmutex/src/recursive_lock.rs`**, `read_guard_deref_reads_correct_value_when_not_mutating`
   — proves the *legitimate* half of the `RecursiveLock` aliasing fix (the Critical finding
   this crate's phase-1 review recorded CONFIRMED and FIXED: `with_mut` replaced a bare
   `DerefMut` because a caller could stash the resulting `&mut T`, call `read_recursive()`
   afterwards, and deref the result, aliasing the two): whenever `mutating` is false (no
   `with_mut` closure is executing), `RecursiveLockReadGuard::deref`'s raw pointer chase
   (`self.mutex.inner.data_ptr().as_ref().unwrap_unchecked()`) is memory-safe and returns
   exactly the value stored in the lock, for arbitrary `i64` and for **every** possible
   `recursive_depth` value (left fully symbolic, since `deref` never consults it — proving that
   independence too). It bypasses `read_recursive()`'s own construction path (which would lock
   `BfSharedMutex`, per above) by directly setting the private `recursive_depth`/`mutating`
   `Cell`s and building the guard struct in place — legal only because the harness lives in the
   same module (`recursive_lock.rs`) as the private fields, exactly like the crate's own
   `#[cfg(test)] mod tests` already does for its counter assertions.

   The other half of the fix — that `deref` *panics* instead of aliasing when `mutating` is
   true — is not expressible as a Kani safety proof (a reachable panic is a verification
   failure, not something Kani has a "must panic" mode for); it remains covered by the crate's
   existing `test_read_recursive_deref_during_with_mut_panics` (`#[should_panic]`, run under
   both plain `cargo test` and Miri per the phase-1 report). The Kani harness and that test
   together cover both branches of the one `assert!`.

Run (from `crates/sharedmutex`):

```
cargo kani
```

Output (full run, no `--harness` filter):

```
Checking harness bf_sharedmutex::verification::data_ptr_is_non_null_and_round_trips_reads_and_writes...
...
VERIFICATION:- SUCCESSFUL
Verification Time: 1.0253854s

Checking harness recursive_lock::verification::read_guard_deref_reads_correct_value_when_not_mutating...
...
VERIFICATION:- SUCCESSFUL
Verification Time: ...

Manual Harness Summary:
Complete - 2 successfully verified harnesses, 0 failures, 2 total.
```

No genuine counterexample was found by either harness — both are new, real verification
coverage of previously only-argued `# Safety` claims, not a bug report.

#### Dead end found and worked around (documented, not a defect)

The first version of each harness above dropped the constructed `BfSharedMutex`/
`RecursiveLock` normally at the end of the function. Both runs hit:

```
Unwinding loop _RNvMNtNtNtNtCs3GJ6w2eqr8A_3std3sys4sync5mutex5futexNtB2_5Mutex14lock_contended.0
  iteration 288 ... function std::sys::sync::mutex::futex::Mutex::lock_contended
```

repeating without terminating — the identical non-termination the phase-1 review reported for
`write()`/`create_read_guard_unchecked`, but reached here through an *implicit* `Drop` at the
end of the harness rather than an explicit call: `Drop for BfSharedMutex` unconditionally
locks `shared.other` to deregister the instance, and `RecursiveLock<T>` has no custom `Drop`
of its own, so its `inner: BfSharedMutex<T>` field drops the same way. This reconfirms, via a
second independent code path, that **no execution reaching `shared.other` terminates under
CBMC in this toolchain**, not just the two call sites the phase-1 review tried. Both harnesses
now `std::mem::forget` the value they construct, which is sound for the property being proved
(neither property depends on the mutex ever being deregistered) and is called out explicitly
in each harness's doc comment so a future reader does not mistake the `forget` for sloppiness.

### `crates/aterm` (5 → 6 harnesses)

**`crates/aterm/src/storage/shared_term.rs`**,
`shared_term_construct_then_read_roundtrips_two_distinct_arguments` — the crate's existing
`shared_term_construct_then_read_roundtrips_one_argument` harness only ever populated
argument index 0, so it could not distinguish a correct per-index offset computation in
`SharedTerm::construct` (`ptr.byte_offset(slice_offset).cast::<ATermRef>().add(index).write(..)`)
from one that silently aliased every argument slot onto the same address — an off-by-one or
stride bug at index ≥ 1 would have passed the existing single-argument proof undetected. This
harness builds two distinct leaked leaf terms, constructs a `SharedTerm` of arity 2 from
them, and proves `arguments()[0]`/`arguments()[1]` each observe the *specific* argument written
to that index (not each other's), and that the two slots do not alias — the invariant every
multi-argument term built through `ATermStorage::insert`'s unbounded-arity path depends on.

Run (from `crates/aterm`):

```
cargo kani
```

Output (full run, no `--harness` filter, confirming all 6 and no regressions in the
crate's 5 pre-existing harnesses):

```
Checking harness storage::aterm_storage::verification::value_unchecked_pattern_reads_back_stored_annotation...
VERIFICATION:- SUCCESSFUL
Checking harness storage::aterm_storage::verification::cast_to_shared_term_ptr_with_one_argument_preserves_argument_identity...
VERIFICATION:- SUCCESSFUL
Checking harness storage::aterm_storage::verification::cast_to_shared_term_ptr_arity_zero_preserves_address_and_length...
VERIFICATION:- SUCCESSFUL
Checking harness storage::shared_term::verification::shared_term_construct_then_read_roundtrips_two_distinct_arguments...
VERIFICATION:- SUCCESSFUL
Checking harness storage::shared_term::verification::shared_term_construct_then_read_roundtrips_one_argument...
VERIFICATION:- SUCCESSFUL
Checking harness symbol::verification::symbol_ref_from_index_preserves_identity_and_reads...
VERIFICATION:- SUCCESSFUL

Manual Harness Summary:
Complete - 6 successfully verified harnesses, 0 failures, 6 total.
```

No counterexample found: `SharedTerm::construct`'s per-index offset arithmetic is correct
for the two-distinct-argument case (and, by the existing harnesses, for zero and one
argument).

### `crates/sabre` (3 → 3 harnesses; no new harness added, existing coverage re-verified)

I looked for a genuinely new, non-vacuous target beyond the existing 3 `Config::Rewrite`/
`Config::Return` transmute-lifetime harnesses (`term_stack.rs`). The two variants those
harnesses deliberately excluded, `Config::Construct(DataFunctionSymbolRef<'a>, ..)` and
`Config::Term(DataExpressionRef<'a>, ..)`, need a live `DataFunctionSymbolRef`/
`DataExpressionRef` — macro-generated (`#[merc_term(is_data_function_symbol)]` /
`#[merc_term(is_data_application)]` in `crates/data/src/data_expression.rs`) wrapper types
over `merc_aterm`'s `SymbolRef`/`ATermRef` whose validity predicates are not simple
structural checks I could fabricate correctly within this phase's time budget without risking
a harness that "passes" only because it does not actually exercise the real
`is_data_function_symbol`/`is_data_application` invariant — which would be worse than no
harness at all (a hallucinated proof). I re-ran the existing suite to confirm no regression
from this phase's other changes:

```
cd crates/sabre && cargo kani
```

```
SUMMARY:
 ** 0 of 127 failed (2 unreachable)
VERIFICATION:- SUCCESSFUL
Manual Harness Summary:
Complete - 3 successfully verified harnesses, 0 failures, 3 total.
```

### `crates/sabre_compiling` (0 harnesses; independently reconfirmed, not a gap I could close)

Grepped every `unsafe` block in the crate's own source (not the FFI crate, out of scope):

```
crates/sabre_compiling/src/innermost_codegen.rs: initialise/rewrite extern "C-unwind" fn
  bodies and DataExpressionRefFFI::from_ptr calls -- all inside a Rust *string template*
  emitted to a generated lib.rs, never executed by this crate's own compiled code.
crates/sabre_compiling/src/library.rs:154: unsafe { Library::new(&path) } -- loads a .so
  this same process just compiled from source; Kani has no dynamic-library-loading model.
crates/sabre_compiling/src/sabre_compiling.rs:48,129: into_data_expression(..)/Symbol
  loading -- FFI-boundary calls into a separately-compiled cdylib.
```

Every one of these is either (a) a pure string-generating function whose *emitted* code is
what is actually unsafe (verified as safe indirectly via the GC-stress regression test the
phase-1 review added, `test_sabre_compiling_survives_garbage_collection_between_calls` —
Kani cannot model "this integer, printed into a string, is later compiled and dereferenced
in a separate process invocation"), or (b) real dylib-loading/FFI, which Kani does not
support crossing at all (no filesystem, no separate compilation unit, no ABI-boundary call).
Confirmed independently (not by trusting the phase-1 report) by running:

```
cd crates/sabre_compiling && cargo kani
```

```
Manual Harness Summary:
No proof harnesses (functions with #[kani::proof]) were found to verify.
```

I did not add a harness here. Writing one against `generate_rewrite_term_stack_impl`'s
string-formatting logic (e.g. proving `virtual_stack.drain(len - arity..)`'s index arithmetic
never underflows) would not be testing `unsafe` code at all — that function is ordinary,
already-bounds-checked safe Rust (`Vec::drain` panics rather than reading out of bounds on a
bad range), so a Kani proof of it would not be verifying a documented unsafe invariant, it
would be padding the harness count with a vacuous target chosen because it happens to compile
under `#[cfg(kani)]`. Per the review's own instructions, I am reporting this as a confirmed
non-viable target rather than manufacturing one.

## Summary of Kani harness counts after this phase

| Crate | Before | After | New this phase |
|---|---|---|---|
| `crates/sharedmutex` | 0 | 2 | 2 |
| `crates/aterm` | 5 | 6 | 1 |
| `crates/sabre` | 3 | 3 | 0 (re-verified, no viable new target found) |
| `crates/sabre_compiling` | 0 | 0 | 0 (independently reconfirmed non-viable) |

## CONFIRMED defects found by a harness

None. Every harness — the 2 new ones in `crates/sharedmutex`, the 1 new one in
`crates/aterm`, and the 3 pre-existing ones re-run in `crates/sabre` — verified
successfully with no counterexample. This phase's value is new verification coverage of
previously only-argued `# Safety` claims (the `RecursiveLock`/`BfSharedMutex` raw-pointer
soundness, and `SharedTerm::construct`'s multi-argument offset arithmetic), not a new bug
report. The one implementation issue this phase surfaced — dropping a `BfSharedMutex`/
`RecursiveLock` normally inside a Kani harness re-triggers the same CBMC
`Mutex::lock_contended` non-termination the phase-1 review found for `write()`/
`create_read_guard_unchecked` — is a Kani/CBMC tooling limitation on `std::sync::Mutex`,
not a defect in the production code; both harnesses were fixed with a documented
`std::mem::forget` rather than left failing.

## Checked and found correct (re-confirmed, not new claims)

- `crates/sabre`'s existing 3 `Config` transmute-lifetime harnesses and `crates/aterm`'s
  existing 5 harnesses: re-ran clean, no regression from this phase's additions.
- `crates/sabre_compiling` has no unsafe surface a Kani harness can meaningfully target,
  independently reconfirmed by inspection of every `unsafe` block in the crate plus a fresh
  `cargo kani` run.

## Tests / proofs added

| File | What | Run with |
|---|---|---|
| `crates/sharedmutex/src/bf_sharedmutex.rs` | `data_ptr_is_non_null_and_round_trips_reads_and_writes` | `cd crates/sharedmutex && cargo kani` |
| `crates/sharedmutex/src/recursive_lock.rs` | `read_guard_deref_reads_correct_value_when_not_mutating` | same |
| `crates/aterm/src/storage/shared_term.rs` | `shared_term_construct_then_read_roundtrips_two_distinct_arguments` | `cd crates/aterm && cargo kani` |

No production code was changed; all additions are `#[cfg(kani)] mod verification` blocks,
following the exact pattern already used in `crates/unsafety/src/{slice_dst,freelist,
protection_set,index_edge}.rs`, `crates/number/src/{power_of_two,bits_for_value}.rs`,
`crates/aterm`'s existing 5 harnesses, and `crates/sabre`'s existing 3. Confirmed with a
full-crate, no-`--harness`-filter `cargo kani` run in each of the four target crates
(`crates/sharedmutex`, `crates/aterm`, `crates/sabre`, `crates/sabre_compiling`), plus
`cargo check -p merc_sharedmutex -p merc_aterm --all-targets`, `cargo clippy -p
merc_sharedmutex -p merc_aterm --all-targets`, `cargo +nightly fmt -p merc_sharedmutex -p
merc_aterm -- --check`, and `cargo test -p merc_sharedmutex --lib` (21/21 pass, including
the pre-existing `RecursiveLock`/`with_mut` regression tests) — all clean, no regressions.
`crates/aterm --lib`'s test suite was not re-run in full: it is already documented
(phase-1-foundation-unsafe-aterm.md, finding #1) as aborting the process on an unrelated,
pre-existing, confirmed-and-unfixed GC-reentrancy defect unaffected by anything in this
phase (no production code was touched, and the new Kani harness never touches
`THREAD_TERM_POOL` or `collect_garbage`).
