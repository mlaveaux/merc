# Phase 1 — Foundation unsafe: `merc_sabre` and `merc_sabre-compiling`

## Verdict

Sound, with the caveat that the crates' unsafe surface is almost entirely a thin, disciplined layer on top of `merc_aterm`'s own protection mechanism (`ProtectedWriteGuard::protect`), not novel pointer/memory manipulation of its own — so most of the risk here is really "did every call site honor `merc_aterm`'s contract", which I checked call-by-call rather than "does this crate reimplement something unsafe from scratch". Every `unsafe` block in `crates/sabre/src` I traced follows the same shape (protect an already-live term, then store the result in the same container on the next statement, with no term/symbol allocation in the gap), and every one I could exercise passed under Miri, including new boundary tests aimed at the paths the existing suite's `#[cfg_attr(miri, ignore)]` annotations leave uncovered. `crates/sabre_compiling` is a different story architecturally: its `innermost_codegen.rs` bakes **raw term-pool addresses** as literal integers into generated Rust source that is compiled to a `cdylib` and loaded back into the same process — the single most novel unsafe pattern in either crate, and one Miri and Kani both cannot reach (Miri has no FFI/dylib support; Kani cannot model "this integer, printed into a string, will later be compiled and dereferenced as a pointer in a separate process invocation"). I could not find a bug in that design — `SabreCompilingRewriter::_spec` is documented, and I confirmed by test, to keep every embedded address reachable via `merc_aterm`'s hash-consing (structural clone ⇒ same pool address) for as long as the rewriter is alive — but it is exactly the kind of invariant that would fail silently (a segfault, or worse, a correct-looking but wrong rewrite from a reused pool slot) rather than loudly, so I gave it the most scrutiny and the strongest test I could construct (a GC-stress regression test, see below). No confirmed defects.

## Findings

No CONFIRMED defects. One PLAUSIBLE-but-not-demonstrated concern, downgraded to "checked and found correct" once I could construct a test for it (see below); I'm still listing it as a finding because the reasoning is load-bearing and worth a reviewer's attention even though it held up.

### 1. Raw pool addresses baked into generated, dynamically-loaded code — PLAUSIBLE risk, held up under test

- **Location**: `crates/sabre_compiling/src/innermost_codegen.rs:383-403` (`generate_rewrite_term_stack_impl`, emitting `DataExpressionRefFFI::from_ptr({:?})` with `symbol.shared().ptr().as_ptr() as *mut () as usize` / `data_expression_ref.shared().ptr().as_ptr() as *mut () as usize` as literal integers), interacting with `crates/sabre_compiling/src/sabre_compiling.rs:26-38` (`SabreCompilingRewriter`'s `_spec: RewriteSpecification` field, documented as "Keeps every term whose raw address is baked into the generated library protected").
- **Scenario**: every `Config::Construct`/`Config::Term` entry of a rule's `rhs_stack`/condition term stacks has its function symbol's or machine-number literal's *current* raw pool address embedded as a plain integer into the generated `lib.rs`. That address is read exactly once, at codegen time, and is dereferenced again on **every call** to the compiled `rewrite`/`match_*` functions for as long as the loaded library is used — with nothing at load time or call time re-validating it. If the term backing that address were ever collected and the pool slot reused for something else, the compiled library would either segfault or (worse) silently rewrite using whatever unrelated term now lives at that address.
- **Why I believe the design is sound**: `RewriteSpecification::clone()` (`Rule { lhs: DataExpression, rhs: DataExpression, .. }`, `#[derive(Clone)]`-shaped) does not create structurally-equal-but-differently-addressed terms — `merc_aterm` is hash-consed (the crate's own comment at `innermost_codegen.rs:498-502`, "With maximal sharing, pointer equality ↔ term equality", is exactly this property), so cloning a live `DataExpression` yields a handle to the *same* pool entry, not a new one at a new address. Every address embedded by `generate_rewrite_term_stack_impl` is reachable (as the root, for a `Construct` symbol, or as a subterm of `rule.rhs`/a condition side, for a `Term` literal — GC in `merc_aterm` is mark-and-sweep from rooted terms, so protecting the root protects every subterm) from a `Rule` inside the `RewriteSpecification` that `SabreCompilingRewriter::new` clones into `_spec` before returning. As long as `_spec` stays alive, every embedded address stays rooted.
- **What I could not verify by static reasoning alone**: whether this actually survives real garbage-collection pressure once the codegen call itself (which no longer holds any of these terms as local variables) has returned and the caller starts driving the compiled library through many rewrite calls, each of which allocates freely and can trigger `merc_aterm`'s automatic GC (`ThreadTermPool::trigger_garbage_collection`, `crates/aterm/src/storage/thread_aterm_pool.rs:520-540`).
- **Evidence**: added `test_sabre_compiling_survives_garbage_collection_between_calls` (`crates/sabre_compiling/src/sabre_compiling.rs:178`), which forces `THREAD_TERM_POOL.force_collect_garbage()` before the compiled library is used at all, and again between every top-level call into it, interleaved with unrelated allocation pressure (64 throwaway `MachineNumber` terms per iteration) specifically to make a collection likely to run while the compiled `rewrite`/`match_*` functions are mid-call. Result:

  ```
  cargo test -p merc_sabre-compiling --lib test_sabre_compiling_survives_garbage_collection_between_calls
  test sabre_compiling::tests::test_sabre_compiling_survives_garbage_collection_between_calls ... ok
  ```

  The rewritten term still matches the expected normal form (120 `s`s for `5!`) after every forced collection. This does not prove the invariant for every possible rule shape, but it is real evidence against the specific failure mode (a collected term whose address is still embedded in running compiled code), not just an argument.
- **Status**: PLAUSIBLE going in; **checked and found correct** by test — I'm keeping it as a listed finding rather than folding it into "checked and found correct" below because the soundness genuinely depends on a cross-crate property (`merc_aterm`'s hash-consing) that this review does not own, and because Miri/Kani are both structurally unable to verify it (see Miri/Kani sections), so a future change to either crate's Clone/interning behavior could silently break it without either tool catching it.
- No fix needed; if anything, the one-line direction would be to assert the invariant more cheaply than a full GC-stress test — e.g. a debug-only check in `generate` that every embedded address is still reachable from `spec.clone()` immediately after cloning — but that's a production-code change outside this review's charter.

## Checked and found correct

- **`term_stack.rs` `unsafe impl Transmutable for Config<'static>`** (`term_stack.rs:91-134`): the transmute only changes a lifetime parameter, which the Rust language guarantees never affects layout; verified this holds for the enum's actual runtime representation (not just argued) with the Kani harnesses below. The real safety-relevant invariant — that callers never request an `'a` that outlives the borrow — is enforced structurally: every call site obtains `'a` only through `ProtectedWriteGuard`/`ProtectedReadGuard`'s own `Deref`/`DerefMut` (`crates/aterm/src/protected.rs:280-313`), whose signature already ties `'a` to the guard's own borrow, so misuse would be a borrow-checker error, not a silent UB opportunity.
- **Every `unsafe { write_*.protect(&x) }` call site in `crates/sabre/src`** (`term_stack.rs:183,259,265`; `configuration_stack.rs:236,266,328,366,374`; `innermost_stack.rs:73,109,115,128,140`; `innermost_rewriter.rs:140,155,175,213,240,249`; `data_substitution.rs:62,66`; `substitution.rs:51,55`): traced each one individually against `merc_aterm::ProtectedWriteGuard::protect`'s actual contract ("the resulting term MUST be inserted into the container", `crates/aterm/src/protected.rs:218-225`, checked in debug builds via `contains_term`/`contains_symbol`). Every call is followed, with no intervening term/symbol allocation, by insertion into the same container it was taken from — confirmed by the fact that the whole existing test suite (debug-mode, so the `contains_term`/`contains_symbol` `debug_assert!`s in `ProtectedWriteGuard::drop` are active) passes, including under Miri. The one call site whose local reasoning is genuinely non-trivial — `configuration_stack.rs`'s `integrate_updated_subterms` running `subterm` through a loop as a bare, momentarily-rootless local between one `protect` and the next store — is safe specifically because nothing on that path allocates in the gap (allocation is the only thing that can exhaust `ThreadTermPool`'s GC budget and trigger a collection), which I traced instruction-by-instruction and then wrote a doc comment and a dedicated boundary test for (see below).
- **`innermost_rewriter.rs:187-198` `Config::Construct` branch, `symbol` used after being popped off `write_configs`**: popping removes `symbol` from the one container that was rooting it, so between the pop and its first use (`symbol.protect()` or `DataApplication::with_iter(&symbol, ...)`) it is a bare, unrooted local. Verified there is no allocation between the pop (`write_configs.pop()`) and that first use — only a `write_terms.write()` lock acquisition, a `len()`, and a slice index, none of which allocate a term or symbol — so no GC can run in the gap.
- **`sabre_compiling.rs:41-49` `SabreCompilingRewriter::rewrite`, `unsafe { into_data_expression(result) }`, and `sabre_compiling.rs:129-137`, loading `initialise`/`rewrite` via `libloading::Symbol`**: both already carried accurate `# Safety` comments; verified the `#[repr(C)]` vtable/function-pointer contract they describe against `library.rs`'s `RuntimeLibrary::compile`, which pins the generated crate's `rust-version` to the host's own `CARGO_PKG_RUST_VERSION`, and against the "not `use_local_workspace`" path pinning the `merc_sabre-ffi` git dependency to the host's exact commit — both are what actually make the generated crate's `#[repr(C)]` layout match the host's.
- **`sabre_rewriter.rs`, `naive_rewriter.rs`, `set_automaton/mod.rs`, `matching/mod.rs`, `matching/nonlinear.rs`, `test_utility.rs`, `rewrite_specification.rs`, `data_position.rs`, `position.rs`, `rewrite_substitution.rs`**: all `#![forbid(unsafe_code)]`; confirmed by grep that this is enforced at the file level and none of them contain an `unsafe` block (the `forbid` lint would refuse to compile otherwise, and the crate builds clean).
- **`sabre_compiling`'s codegen output itself (not the generator)**: the generated `rewrite_arity_N`/`rewrite_arity_generic` functions in the emitted `lib.rs` template (`innermost_codegen.rs:69-195`) wrap their FFI calls (`term.data_arg(i)`, `DataExpressionFFI::create`) in `unsafe` blocks — these are ordinary FFI-boundary calls into `merc_sabre_ffi`'s vtable-forwarded functions, not novel pointer arithmetic; the FFI crate itself (`crates/sabre_compiling/sabre_ffi`) is out of this review's scope (not listed as one of the two crates under review) and was not audited here.

## Miri

Ran against both crates' existing test suites first, then again after adding the new boundary tests, matching the CI invocation exactly:

```
MIRIFLAGS="-Zmiri-disable-isolation --cfg chacha20_force_soft" \
  cargo +nightly miri nextest run --no-fail-fast -p merc_sabre
```

Before any changes: `22 tests run: 22 passed, 30 skipped` (30 skipped are the crate's own pre-existing `#[cfg_attr(miri, ignore)] // Test is too slow under miri` end-to-end tests in `crates/sabre/tests/*.rs`).

After adding the boundary tests below: `26 tests run: 26 passed, 30 skipped` — 4 new tests, all passing, no regressions:

```
PASS [   8.135s] merc_sabre utilities::configuration_stack::tests::test_no_match_leaf_leaves_term_unchanged
PASS [  10.420s] merc_sabre utilities::configuration_stack::tests::test_root_level_match_prunes_at_depth_zero
PASS [  17.245s] merc_sabre utilities::configuration_stack::tests::test_nested_match_forces_multi_level_grow_and_integrate
PASS [  16.101s] merc_sabre utilities::term_stack::tests::test_rhs_stack_machine_number_constant
────────────
     Summary [ 131.547s] 26 tests run: 26 passed, 30 skipped
```

```
MIRIFLAGS="-Zmiri-disable-isolation --cfg chacha20_force_soft" \
  cargo +nightly miri nextest run --no-fail-fast -p merc_sabre-compiling
```

`2 tests run: 2 passed, 1 skipped` (both are `indenter`'s own tests — pure text-formatting logic, no unsafe). The skipped test is the crate's existing `#[cfg_attr(miri, ignore)] // Miri does not support FFI.` end-to-end test, and my new `test_sabre_compiling_survives_garbage_collection_between_calls` carries the same annotation for the same reason (it compiles and `dlopen`s a real `cdylib`). **This means Miri gives zero coverage of `merc_sabre-compiling`'s actual unsafe surface** (`sabre_compiling.rs`'s FFI/dylib-loading unsafe, `library.rs`'s `Library::new`, and the raw-pointer-embedding codegen) — a structural gap, not something this review's new tests can close, since Miri fundamentally does not support dynamic library loading or FFI calls across a real ABI boundary. The GC-stress test (finding #1) and the doc-comment contracts are the substitute evidence for that surface.

## Loom

Not applicable. Neither crate has any concurrent/lock-free data structure of its own — `Protected<C>`'s `!Send` design (`crates/aterm/src/protected.rs:32-35`) confines every container this review touched to a single thread, and `crates/sabre`/`crates/sabre_compiling` do not spawn threads or share state across threads anywhere in the reviewed files. No loom-gated tests exist or were added.

## Kani

### `crates/sabre` — 3 new harnesses added, all passing

Added `[package.metadata.kani] unstable = { function-contracts = true, mem-predicates = true }` to `crates/sabre/Cargo.toml` (verbatim from `crates/unsafety/Cargo.toml`, as instructed).

The only production `unsafe` *definition* owned by this crate (as opposed to a call into `merc_aterm`'s `unsafe fn protect`, defined in a different crate under separate review) is `unsafe impl Transmutable for Config<'static>` in `term_stack.rs:101-134`. That is the one harness target here — there is no bespoke index-based/intrusive-pointer stack structure of the `freelist.rs` shape in this crate's own code (`TermStack`'s `Protected<Vec<Config>>` is a safe `std::vec::Vec` under a lifetime-erasing GC-root wrapper defined in `merc_aterm`, not a hand-rolled pointer structure in `crates/sabre`).

`Config<'a>` has two variants that need no live term pool to construct (`Rewrite(usize)`, `Return()`) and two that do (`Construct(DataFunctionSymbolRef<'a>, ..)`, `Term(DataExpressionRef<'a>, ..)` — both need `merc_aterm`'s thread-local pool, global mutexes and hash-consing machinery initialised, which is not practical to bound cheaply for CBMC). I scoped the harnesses to the two pool-free variants, which still exercise the real `Config<'static>` type and the real `unsafe impl`, proving the property that is actually specific to this impl: a lifetime-only transmute must not corrupt the enum's discriminant or payload, and a write through the transmuted mutable reference must be visible through the original binding (i.e. they genuinely alias, not merely happen to compare equal).

```
cd crates/sabre && cargo kani
```

```
SUMMARY:
 ** 0 of 127 failed (2 unreachable)

VERIFICATION:- SUCCESSFUL
Verification Time: 0.055246897s

Manual Harness Summary:
Complete - 3 successfully verified harnesses, 0 failures, 3 total.
```

Harnesses (`term_stack.rs:517-576`, `mod verification`):
- `config_transmute_lifetime_preserves_rewrite_payload` — `Config::Rewrite(kani::any::<usize>())`, transmute, assert the variant and payload survive for every `usize`.
- `config_transmute_lifetime_preserves_return_variant` — same for the zero-payload `Return()` variant.
- `config_transmute_lifetime_mut_round_trips_writes` — transmutes `&mut`, writes through the transmuted reference, asserts the write is visible through the original binding (proves the two references genuinely alias the same bytes, not just structurally-equal copies).

### `crates/sabre_compiling` — Cargo.toml block added, deliberately zero harnesses

Added the same `[package.metadata.kani]` block to `crates/sabre_compiling/Cargo.toml`. `cargo kani` in that directory compiles cleanly and reports:

```
Manual Harness Summary:
No proof harnesses (functions with #[kani::proof]) were found to verify.
```

This is intentional, not an oversight: every `unsafe` in this crate is either (a) an FFI/dylib-loading call (`library.rs:149`'s `Library::new`, `sabre_compiling.rs:129-137`'s `Symbol`/function-pointer loading) — Kani has no model for "load and later call into a separately-compiled shared object", so there is nothing to state as a `kani::requires`/`modifies` contract; or (b) the raw-pointer-embedding codegen in `innermost_codegen.rs` — which is not itself unsafe *code that runs*, it is a pure, safe, string-generating function; the unsafety is a property of code Kani would have to model being compiled and executed in a *separate* process invocation after this one returns, which is outside what Kani proves about a single compilation unit. Writing a harness here would necessarily either be vacuous (proving something true by construction, like `size_of` equality) or would have to fake the entire compile-and-reload pipeline, defeating the point. I gathered evidence for that surface a different way instead: the GC-stress regression test (finding #1) and the mathematical `# Safety` contract on `generate_rewrite_term_stack_impl` (below).

## Miri boundary tests (added per the mid-task instruction)

All small, deterministic, single-threaded, targeting the actual edges of the `protect`-then-store discipline; none use `merc_utilities::random_test`-style stress patterns. All ran and passed under Miri (see the Miri section above for the combined run; exact per-test lines are in that section's output).

- `configuration_stack.rs:475` `test_root_level_match_prunes_at_depth_zero` — a rule matching at the very root (`a -> b`): the configuration stack never grows past its single initial entry, so `prune` runs with `depth == 0` and `terms_base + depth` is the very first slot `ConfigurationStack::new` ever pushed — the shallowest possible case of the `unsafe { write_terms.protect(...) }` call in `prune` (`configuration_stack.rs:266`).
- `configuration_stack.rs:490` `test_no_match_leaf_leaves_term_unchanged` — a term (`c`) matching no rule at all: `jump_back` runs at `depth == 0` with `oldest_reliable_subterm == 0`, so `integrate_updated_subterms` takes its early-return guard (`up_to_date == 0`, `configuration_stack.rs:318`) without ever calling `protect`. Pins the boundary the guard exists for.
- `configuration_stack.rs:508` `test_nested_match_forces_multi_level_grow_and_integrate` — `f(f(x)) -> x` applied to `f(f(f(f(c))))` and `f(f(f(c)))`: matching the two-deep pattern against a four/three-deep chain forces `ConfigurationStack::grow` to push more than one configuration before a match is found, and each rewrite's `prune`/`jump_back` walks `integrate_updated_subterms`'s loop across more than one stack level — the `Some(position)` branch that runs `data_substitute_with` and re-protects its result (`configuration_stack.rs:342-374`) is not reachable with the crate's existing miri-covered tests (`regression_tests.rs`'s two tests use only shallow, single-argument rules).
- `term_stack.rs:489` `test_rhs_stack_machine_number_constant` — a rewrite rule whose right-hand side is itself a machine-number literal (`f(x) -> 42`), rather than a bare variable (already covered by `test_rhs_stack_variable`) or a `Construct`-only tree (`test_rhs_stack`). This is the only test in the module that reaches `TermStack::from_term`'s `is_data_machine_number` branch (`term_stack.rs:150-160`, pushing a `Config::Term` for the constant) and the corresponding `Config::Term` evaluation arm (`term_stack.rs:244-250`) — both previously unexercised by any test in this file.
- `sabre_compiling.rs:178` `test_sabre_compiling_survives_garbage_collection_between_calls` — see finding #1. **Not run under Miri** (`#[cfg_attr(miri, ignore)] // Miri does not support FFI.`, matching the crate's existing FFI test): Miri cannot load a dynamic library, so this is ordinary-build-only evidence, called out explicitly rather than silently skipped.

## Mathematical `# Safety` contracts (doc-comment-only changes)

All changes are comments only — no production logic changed. No bare `unsafe impl Send`/`Sync` marker impls exist anywhere in `crates/sabre/src` or `crates/sabre_compiling/src` (confirmed by the initial full-crate `unsafe` grep and by re-checking every `unsafe impl` found: the only one in either crate is `Transmutable for Config<'static>`, which is not a marker trait), so there was nothing of that specific shape to write a "which field, why sound to move/share" contract for; noting that explicitly rather than silently having nothing in this section.

- **`term_stack.rs:91-134` `unsafe impl Transmutable for Config<'static>`**: rewrote the impl-level comment and added a `# Safety` section to each of `transmute_lifetime`/`transmute_lifetime_mut` individually. States precisely: *requires* `'a` does not outlive the `&self`/`&mut self` borrow (and, for the `_mut` variant, that no other reference to the same `Config` is live for `'a`, which follows from the input being `&mut`); *guarantees* the result aliases exactly the same bytes as `self` (justified by the language-level fact that a lifetime parameter is never part of a type's runtime representation, so `Config<'static>`/`Config<'a>` are layout-identical for every `'a`), valid for reads (or reads+writes) for at most `'a`.
- **`innermost_stack.rs:18-34` `InnermostStack` struct doc**: added a "Safety invariant maintained by every method below" section stating the general contract every `protect`-then-store call site in the file relies on: `protect`'s own contract requires the result be inserted into the same container (that's what makes it a GC root — the transmute itself grants no protection), and every call site in this file does so with nothing allocating in the gap, which is what makes the momentary rootlessness safe (allocation is the only thing that can exhaust the GC budget counter and trigger a collection).
- **`configuration_stack.rs:315-374` `integrate_updated_subterms`**: replaced the previous "subterm is kept in write_terms throughout this function" comment (imprecise — `subterm` is *not* literally an element of `write_terms` for part of the loop body) with the precise argument: the `None` branch's `subterm` is only ever the already-rooted value read from `write_terms[base + up_to_date]`, and the `Some(position)` branch's `subterm.protect()` (the *safe* `Term::protect`, not the unsafe guard method) is what actually roots it across `data_substitute_with`'s allocations, via the ordinary thread-local protection stack — the `unsafe { write_terms.protect(...) }` calls in this function are never what protects a term across an allocating call; they are only ever a same-statement-or-next-line handoff into `write_terms`.
- **`innermost_rewriter.rs:179-188` `Config::Construct` branch**: added a comment stating precisely why `symbol` (popped off `write_configs`, hence no longer rooted by that container) stays implicitly reachable: no allocation occurs between the pop and its first use, and that first use (`symbol.protect()` or `DataApplication::with_iter`) is what re-roots it.
- **`library.rs:148-161` `unsafe { Ok(Library::new(&path)?) }`**: this call had *no* safety comment before this review (a gap). Added one stating why loading is sound here specifically: `path` names a library this same process just built from source this crate generated moments ago, from a `Cargo.toml` whose `rust-version` is pinned to the host's own `CARGO_PKG_RUST_VERSION`; there is no library-side initialisation run at load time beyond static linking, and the actual ABI-matching argument (why `initialise`/`rewrite`'s signatures are trustworthy) is deferred to `SabreCompilingRewriter::new`'s own comment, which this one now cross-references.
- **`innermost_codegen.rs:344-374` `generate_rewrite_term_stack_impl`**: added a "Safety contract of the emitted code (not this function itself)" section — the most load-bearing new comment in this review — stating the exact liveness requirement discussed in finding #1: every embedded address must stay reachable from `RewriteSpecification` for as long as the compiled library using it can still be invoked, and naming `SabreCompilingRewriter::_spec` as the mechanism that currently guarantees this.

## Tests and proofs added

| File | What | Run with |
|---|---|---|
| `crates/sabre/Cargo.toml` | `[package.metadata.kani]` block (verbatim copy from `merc_unsafety`) | n/a (enables `cargo kani`) |
| `crates/sabre_compiling/Cargo.toml` | `[package.metadata.kani]` block (same) | n/a (enables `cargo kani`, 0 harnesses by design — see Kani section) |
| `crates/sabre/src/utilities/term_stack.rs:489` | `test_rhs_stack_machine_number_constant` (Miri boundary test) | `cargo +nightly miri test -p merc_sabre --lib utilities::term_stack::tests::test_rhs_stack_machine_number_constant` |
| `crates/sabre/src/utilities/term_stack.rs:517-576` | 3 Kani proofs (`config_transmute_lifetime_preserves_rewrite_payload`, `config_transmute_lifetime_preserves_return_variant`, `config_transmute_lifetime_mut_round_trips_writes`) | `cd crates/sabre && cargo kani` |
| `crates/sabre/src/utilities/configuration_stack.rs:440-521` | 3 Miri boundary tests (`test_root_level_match_prunes_at_depth_zero`, `test_no_match_leaf_leaves_term_unchanged`, `test_nested_match_forces_multi_level_grow_and_integrate`) | `cargo +nightly miri nextest run -p merc_sabre` (or `cargo test -p merc_sabre --lib utilities::configuration_stack::tests`) |
| `crates/sabre_compiling/src/sabre_compiling.rs:178` | `test_sabre_compiling_survives_garbage_collection_between_calls` — GC-stress regression test for finding #1 (**not Miri-runnable**, FFI) | `cargo test -p merc_sabre-compiling --lib test_sabre_compiling_survives_garbage_collection_between_calls` |

Doc-comment-only changes (no test to "run"; see previous section for what each states): `crates/sabre/src/utilities/term_stack.rs:91-134`, `crates/sabre/src/utilities/innermost_stack.rs:18-34`, `crates/sabre/src/utilities/configuration_stack.rs:324-341`, `crates/sabre/src/innermost_rewriter.rs:179-198`, `crates/sabre_compiling/src/library.rs:148-161`, `crates/sabre_compiling/src/innermost_codegen.rs:344-374`.

Full commands used for the headline runs in this report:

```
MIRIFLAGS="-Zmiri-disable-isolation --cfg chacha20_force_soft" cargo +nightly miri nextest run --no-fail-fast -p merc_sabre
MIRIFLAGS="-Zmiri-disable-isolation --cfg chacha20_force_soft" cargo +nightly miri nextest run --no-fail-fast -p merc_sabre-compiling
cd crates/sabre && cargo kani
cd crates/sabre_compiling && cargo kani
cargo test -p merc_sabre --lib
cargo test -p merc_sabre-compiling --lib
```
