---
name: tests
description: How to write, move, review and prune tests in merc, and how to run a test pass over a crate with agents. Use when adding or changing tests, when asked to clean up a crate's tests, or when delegating a test pass.
---

# Tests in merc

A test earns its place by **failing when the behaviour is wrong**. Everything below follows from
that. The `architecture` skill describes the code under test; the `check` skill runs the suite as
CI does; `unsafe-verify` covers miri, loom and kani.

## Where tests live

- A test that only uses a crate's public API is an integration test: `<crate>/tests/<topic>_test.rs`,
  one `use` per item, no globs. Name new files `*_test.rs`.
- An inline `#[cfg(test)] mod tests` is only for tests that need private items (a private module's
  contract, a test hook, `unsafe` internals). If the only reason to stay inline is a helper that
  exists for the test, prefer changing the API (the maintainer accepts API changes) over keeping
  the coupling.
- `unwrap()` is fine in tests. Outside tests use `expect("why this cannot fail")`; a comment that
  justified an `unwrap` becomes the `expect` message.

## What a test asserts

- Test the observable contract: the inferred sort, the lowered equations, the normal form, the
  solution of a PBES, the quotient of an LTS. **Do not pin shape**: exact internal numbering, node
  or block ids, intermediate structures, the exact summand layout of an intermediate form. Such
  tests fail on every refactor and prove nothing about behaviour.
- Delete tests of struct layout, `Send`/`Sync` or anything else the compiler already checks. A
  layout fact that `unsafe` relies on stays as `const _: () = assert!(...)` at module level next to
  that code, not under `cfg(test)`.
- Assert exact values. `contains(..)`, `is_ok()`, `> 0`, `println!` and `debug_assert!` do not fail
  in release builds or on a wrong answer; a round trip alone passes any format that decodes
  itself, so also pin one exact encoding. Compare printed output with the exact expected text.
- Prefer an oracle over a memorised answer: a model (`Vec`, `HashMap`, a naive algorithm, a tiny
  solver) driven by `merc_utilities::random_test`, reproducible with `MERC_SEED=<seed>`. An oracle
  must be independent of the code under test: do not compute the expected value with the same
  formula or function as the implementation.
- Terms are DAGs and can be very deep: test traversals and visitors on a shared (exponentially
  unfolding) term and on a deep chain, and check early exit and pruning (see `architecture`).
- Delete duplicates, trivial one-line helpers, and tests of a constructor that only sets fields.
- Do not narrate history in test comments ("before the fix", "used to"). State the behaviour.

## What a test file looks like

One file per public topic of the crate (`slot_list_test.rs`, `traverse_test.rs`, `srf_test.rs`).
Inside it, in this order:

1. A `//!` line saying what the file covers, in terms of behaviour. No history, no "regression for".
2. One `use` per item: external crates, then the crate under test, then `merc_utilities`.
3. Small fixtures and builders (`term("f(a)")`, `spec()`), only when several tests share them.
   A helper that only renames one call is not a fixture.
4. **Exact, deterministic tests first**: a few hand-checked cases, with the expected value written
   out literally. They are what miri runs and what a failing oracle test is debugged against.
5. **The oracle test**: random operations compared with a model, with a doc comment that names
   the model.
6. Edge and failure cases: empty input, the maximum, deep and shared terms, the exact error
   (`should_panic(expected = "...")` or an exact `Err`), never a bare `should_panic`.

Skeleton (from `crates/core/utilities/tests/slot_list_test.rs`):

```rust
use rand::RngExt;

use merc_utilities::SlotList;
use merc_utilities::random_test;

#[test]
fn test_slot_list_order() {
    let mut list = SlotList::new();
    let a = list.push_back('a');
    list.push_back('b');
    let c = list.push_back('c');

    list.move_to_back(a);
    assert_eq!(list.remove(c), 'c');

    // Slot of `c` is reused.
    assert_eq!(list.push_back('d'), c);

    for expected in ['b', 'a', 'd'] {
        assert_eq!(list.pop_front(), Some(expected));
    }
    assert_eq!(list.pop_front(), None);
}

/// Compares a random sequence of operations against a vector of `(slot, value)` pairs in list
/// order.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_slot_list() {
    random_test(100, |rng| {
        let mut list = SlotList::new();
        let mut model: Vec<(usize, u32)> = Vec::new();
        // ... pick an operation with `rng`, apply it to both, and compare after every step.
    });
}
```

Weak to strong, each taken from a real fix in these passes:

| Weak | Strong |
|---|---|
| `debug_assert_eq!(read_back, written)` in a test | `assert_eq!`: `debug_assert` is gone in release, where CI also runs the tests |
| a round trip `decode(encode(x)) == x` | also `assert_eq!(encode(300u64), [0xac, 0x02])` for a few values and the boundaries |
| `assert!(output.contains("f(a)"))` | `assert_eq!(output, "f(a, b, g(b))")` |
| `assert!(result.is_some())` | `assert_eq!(result, Some(expected))`, and one case that must be `None` |
| `assert!(set.len() > 0)` | the exact length, and the exact members |
| the result checked only for the first element or block | every element, in order, with an independent model |
| a test whose subject also lives in a local variable | drop the local first; otherwise it keeps the thing alive that the test claims the container keeps alive |
| expected value computed with the function under test | a literal, or an independent algorithm (a naive solver, `BigUint`, a model collection) |
| `#[should_panic]` | `#[should_panic(expected = "reserved for internal use")]` |

Good examples to copy, by what they show:

| Pattern | File |
|---|---|
| exact cases plus a model test | `crates/core/utilities/tests/slot_list_test.rs` |
| exact byte format | `crates/core/number/tests/variablewidth_test.rs` (`test_exact_encoding`) |
| a miri-sized test of unsafe code next to slow ones | `crates/core/unsafety/tests/concurrent_indexed_set_test.rs` |
| visitors: early exit, prune, context, shared and deep terms | `crates/rewriter/data/tests/visitor_test.rs` |
| every `Traverse` operation on every node type | `crates/language/syntax/tests/traverse_test.rs` |
| a semantic oracle (a small solver) instead of pinning the output shape | `crates/language/pbes/tests/srf_test.rs` |

## Slow tests, miri, loom, kani

- Slow tests carry `#[cfg_attr(miri, ignore)]` with the reason, and have a small deterministic
  sibling that miri runs. pest-based parsers are not supported by miri, so those tests stay ignored.
- Never remove or weaken a test that miri, loom or a kani harness relies on.
- `nightly.yml` runs the suite with `-- --include-ignored`, so an `#[ignore]`d failing test still
  fails CI. Use `ignore` only for tests that are slow, not for known defects.

## Known defects

Write the test for the correct behaviour first and show it fail. Then either fix the defect, or,
if the fix is a separate decision, keep the test green with `#[should_panic(expected = "...")]`
and a doc paragraph "Known defect: ..." that says what is wrong and that the attribute is to be
removed with the fix. Do not pin
the buggy output as if it were expected.

## Proving a test is useful (mutation check)

For every test you move, strengthen or keep on purpose, break the code in a plausible way and
watch the test fail: reverse an iteration, drop a branch, flip a comparison, skip a step. A
mutant that survives means a weak test or an equivalent mutant; say which, with the reasoning.

- Never mutate the working tree. Make a worktree (`git worktree add /tmp/<name>-mut HEAD`),
  copy the new tests in, mutate there, run, and remove it with `git worktree remove --force`. A
  mutation left in the checkout was once committed, and it broke `BfSharedMutex::try_write` on the
  branch.
- Report the mutation next to each strengthened test.

## A test pass over a crate

1. Classify every inline test: move, keep inline (private items), strengthen, replace by a
   contract test, or delete (with one sentence why). Write the plan down first.
2. Move and rewrite, running `cargo test -p <pkg>` after each crate.
3. Mutation-check the strengthened tests (above).
4. `cargo +nightly fmt -p <pkg>`, `cargo clippy -p <pkg> --all-targets` (also with
   `--features internal` where a crate has it), `cargo check --workspace --all-targets`, and, when
   non-test code changed, `cargo check --all-targets` in `tools/mcrl2` and `tools/gui`.
5. miri on touched crates with a timeout; say if it times out instead of claiming a pass.

Production code changes allowed in a test pass: `unwrap` to `expect`, deleting items that only the
removed tests used, `allow(dead_code)` to `expect(dead_code, reason = "...")` or deletion, and
const assertions. Nothing else; a defect found on the way gets a test and a report, not a silent
fix.

## Reviewing a test pass done by someone else (or an agent)

Take the report as claims. Before committing:

- Production code is unchanged: compare each touched `src` file with `git show HEAD:<file>` after
  removing the test module (match braces) and comment lines. Expect a small number of intended
  differences and read every one.
- Run fmt, clippy (`-D warnings`, with and without optional features), the crates' tests, and the
  workspace check yourself. Do not rely on the report for those.
- Check that deleted tests were duplicates or coupled, not the only guard of a behaviour: ask what
  would now catch the bug. Restore a deleted test that guarded real behaviour only reachable
  through private items.
- Check weak spots by hand: a test whose subject stays alive through another handle proves nothing
  about the container under test; a `should_panic` without `expected` can pass for the wrong
  reason.
- Never commit work that an agent is still editing, and never commit a tree before checking that no
  experiment is left in the sources.
- Stage only the crates you verified (`git add <paths>`), not `git add -A`, while other work is in
  progress.
