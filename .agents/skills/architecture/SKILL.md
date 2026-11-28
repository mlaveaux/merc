---
name: architecture
description: How merc is built and which idioms its code uses: the aterm term library (maximal sharing, ATerm vs ATermRef, protection and garbage collection, Return, Protected, TermBuilder, merc_derive_terms), the visitor and traversal APIs (Visit/Step, DataExpressionVisitor, PbesExpressionVisitor, Traverse), crate layout, and code conventions. Read before writing, reviewing or auditing code in crates/, so that new code matches the existing idioms.
---

# Architecture and idioms

Verify specifics against the source before relying on them; the crate READMEs
(`crates/core/aterm/README.md` first) are the maintained reference, this skill is the map.

## Crate groups

| Group | Crates | Role |
|---|---|---|
| `crates/core` | `unsafety`, `sharedmutex`, `utilities`, `collections`, `number`, `io`, `tools`, `aterm` | Low level building blocks. `unsafety` (stable pointers, block allocator, protection sets) and `sharedmutex` (recursive read/write lock) carry the invariants the term library rests on. `tools` holds the shared `clap` helpers for every binary (verbosity, version, reporting). |
| `crates/rewriter` | `data`, `macros`, `sabre`, `sabre_compiling`, `enumerate`, `rec-tests` | mCRL2 data expressions as terms (`merc_data`), the `merc_derive_terms` proc macro, the Sabre rewriter (set automaton) and the innermost rewriter. |
| `crates/language` | `syntax`, `typecheck`, `lowering`, `process`, `pbes`, `pres`, `formula` | Parser (pest grammar, syntax tree), type checker, lowering to term representations. |
| `crates/transition_systems` | `lts`, `reduction`, `refinement`, `explore`, `symbolic`, `vpg` | LTS formats and algorithms, symbolic (LDD/BDD) exploration, variability parity games. |

Dependencies point downwards: `core` knows nothing of the others. `tools/*` are binaries on top.
`tools/mcrl2` is different: it wraps the C++ mCRL2 toolset through `mcrl2-sys` and has its own
term wrappers (see below).

## The aterm library (`merc_aterm`)

A term is `c | f(t1, ..., tn) | u64`. Terms live in one global, maximally shared pool.

- **Equality is pointer equality.** Structurally equal terms are the same `SharedTerm`, so `==`,
  hashing and `index()` are O(1). Consequence: a term is a DAG node, and the *tree* it unfolds to
  can be exponentially larger. Any algorithm must be linear in the DAG, not the tree (see
  traversal below).
- **Owned and borrowed.** `ATerm` is an owned, protected term. `ATermRef<'a>` is a borrowed term
  that is only valid while something protects it for `'a`. `Term<'a, 'b>` is the trait both
  implement (`arg`, `arguments`, `copy`, `borrow`, `protect`, `get_head_symbol`, `iter`). Write
  generic code against `Term`, take `&impl Term` or `ATermRef<'_>`, and call `protect()` only
  when the term must outlive the borrow. Typed wrappers come in pairs, `DataExpression` /
  `DataExpressionRef<'a>`.
- **Garbage collection and protection.** The pool is collected when terms are unreachable.
  Reachability is tracked through thread-local protection sets: owned terms and `Protected`
  containers register themselves. A term not protected by anything may disappear at the next
  collection, which is why a borrowed `ATermRef` must never outlive its protector, and why
  `unsafe` that transmutes a lifetime needs a `// SAFETY:` argument naming the protector.
- **`Return<T>`** returns a term from a function without protecting it. It holds a read guard
  on the pool, which blocks collection until it is dropped: keep it short-lived, never store it.
- **`Protected<C>`** stores many terms under one protection, e.g.
  `Protected<Vec<ATermRef<'static>>>`. Contents implement `Markable`. Mutation goes through a
  scoped closure (`write`) so removed terms cannot escape unprotected; reads use a guard.
  `ProtectedSend` / `ATermSend` are the variants that can cross threads.
- **`GarbageCollectionPause`** postpones automatic collection while nearly every new term stays
  live (e.g. reading a term stream). It nests.
- **`TermBuilder`** builds or rewrites a term from an inductive input without native recursion
  (a `Yield` callback either returns a finished term or pushes sub-inputs and a symbol to
  construct). `merc_aterm::apply` rewrites every subterm with it. Use it instead of recursive
  functions on terms of unbounded depth: deep terms are normal here and the tests raise
  `RUST_MIN_STACK` for the ones that still recurse.
- **`merc_derive_terms`** (crate `merc_macros`, proc macro) generates the term boilerplate for
  structs wrapping exactly one `ATerm` field marked `#[merc_term(predicate)]`; `#[merc_ignore]`
  skips a method. Used for `ATermList`/`ATermInt` and heavily in `merc_data` and `merc_pbes`.
  Edit the struct, not generated output.
- **Safety stance.** Modules with only safe Rust carry `#![forbid(unsafe_code)]`; keep it when
  you add a module. The binary stream (`aterm_binary_stream`) is compatible with the mCRL2
  format, do not change the encoding.

`tools/mcrl2/crates/mcrl2` re-implements the same names (`ATerm`, `ATermRef`, `DataExpression`)
over the C++ atermpp library. Same shape, different pool: do not mix the two.

## Traversal and visitors

Terms and syntax trees are DAGs or deep trees, so traversal has three rules: no unbounded native
recursion, early exit, and no repeated work on shared subterms. The shared vocabulary is in
`merc_utilities` (`traversal.rs`):

- `Step<N, C>`: what to do below the node just seen: `Into(context)` descend carrying a context,
  `Prune` skip the children, `Replace(N)` substitute (the replacement is not descended into).
  `N = Infallible` makes `Replace` impossible, so read-only traversals share the type.
- `Visit<N, C, T, E> = Result<ControlFlow<T, Step<N, C>>, E>`: error `E`, stop with result `T`
  (`ControlFlow::Break`), or continue with a `Step`. The context `C` must be `Copy` so threading
  it costs nothing; state that grows along a path lives in the callback (push on entry, truncate
  on exit) or use the scoped variants.

Three families implement it. Prefer extending one over inventing a fourth:

| API | Where | Use |
|---|---|---|
| `DataExpressionVisitor<'a>` and `try_visit_data_expr` / `visit_data_expr` | `merc_data` (`visitor.rs`) | Top-down over data expressions. One method per node kind with a default that keeps descending; override only the kinds you need. The closure forms cover the common case. Binder variables are not visited as variables. |
| `PbesExpressionVisitor`, `try_visit_pbes_expr` / `visit_pbes_expr`, `fold_pbes_expr` | `merc_pbes` (`visitor.rs`) | Same for PBES expressions. Data expressions are leaves here; look inside with a `DataExpressionVisitor`. `fold_pbes_expr` is bottom-up with a memoising cache keyed by the shared term, folding each distinct subexpression once. |
| `Traverse` trait (+ `MixedNode`) | `merc_syntax` (`traverse.rs`) | Over the syntax trees (`DataExpr`, `StateFrm`, `RegFrm`, `ProcessExpr`, `PbesExpr`, ...). A node type implements only `push_children`/`push_children_mut`; `visit`, `try_visit`, `visit_with`, `apply`, `transform`, scoped enter/exit and cross-type `visit_mixed` come for free, all with an explicit heap stack. |

Read-only visitors return `Step::Prune` for subterms already seen (keep a set of visited
`ATermRef`s or use the pointer identity) so the work stays linear in the DAG. Rebuilding a term
bottom-up is `TermBuilder`, not a visitor: `Step::Replace` is uninhabited on the term visitors.

`tools/mcrl2/crates/mcrl2/src/visitor.rs` has an older, different `DataExpressionVisitor`
(returns `Option<DataExpression>`, no context, no early exit) next to newer `*ContextVisitor`
traits. The older one is not the model for new code; see `docs/review-pass.md`.

## Conventions

- **Imports:** one `use` per item (`imports_granularity = "Item"`, needs nightly rustfmt), no glob
  imports in production code, `max_width = 120`.
- **`unwrap`/`expect`:** outside tests, no bare `unwrap`; use `expect("why this cannot fail")`, and a comment justifying an `unwrap` becomes the `expect` message. Input-dependent failures return an error.
- **Helpers:** no trivial one-line helpers that only rename another call.
- **Errors:** `merc_utilities::MercError` for boundary errors (it boxes), `thiserror` enums for
  typed library errors, no `anyhow`. Binaries use `clap` derive plus the `merc_tools` helpers.
- **Dependencies:** the workspace pins versions in the root `Cargo.toml`; new dependencies need
  `cargo deny check` and a reason.
- **Tests:** see the `tests` skill. In short: public-API (integration) tests live in the crate's `tests/` directory; an inline `#[cfg(test)]` module is only for tests that need private items. `unwrap` is fine in tests. Do not test struct layout, `Send`/`Sync` or other compiler checks (keep layout invariants as `const _: () = assert!(...)` next to the `unsafe` relying on them), and do not pin private helpers or internal state: such tests churn on every refactor. A test earns its place by failing when behaviour is wrong. Property-style with `merc_utilities::random_test` (reproduce with
  `MERC_SEED=<seed>`); `test-case` for tables; small deterministic cases for miri. Expected
  outputs live in `snapshot/` files next to the test.
- **Docs:** rustdoc for the contract (`# Panics`, `# Safety`), crate README for the architecture
  (included as crate docs), the website for algorithms and measurements. See `documentation`.
- **Unsafe:** every block has a `// SAFETY:` comment stating the invariant upheld, and is covered
  by the `unsafe-verify` skill.
- **Comments:** describe the code as it is now; no change history, no TODO apologies. See `review`.
- **Public API of `crates/*`:** these are published; breaking changes need `cargo semver-checks`
  and a version bump.
