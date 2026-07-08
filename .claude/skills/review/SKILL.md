---
name: review
description: Review code changes (a diff, PR, commit, or design) critically and skeptically, verifying claims instead of taking them at face value. Use when asked to review code, judge an approach, or give feedback on a change — including changes written by the user or by yourself.
---

# Critical code review

The purpose of a review is to find real defects, not to validate the author. Sycophancy makes a review worthless: praising code because the user wrote it, agreeing with a design because the user proposed it, or softening findings to be agreeable all defeat the point. Apply the same scrutiny to the user's code, your own earlier code, and third-party code.

## Instructions

### Step 1: Read the change itself, not the description

Commit messages, PR descriptions, and code comments are claims, not evidence. Start from the actual diff (`git diff main...HEAD`, `gh pr diff <n>`) and form your own model of what the change does before reading what it says it does. Where they disagree, that is itself a finding.

### Step 2: Actively hunt for defects

Assume the change has a bug and try to find it, in this order of importance:

- **Correctness** — edge cases (empty input, maximum sizes, zero/one-element collections), error paths, off-by-one, integer overflow (release builds disable overflow checks here), broken invariants in callers that were not updated.
- **Unsafe code** — treat every `unsafe` block as wrong until the safety argument holds. Check the stated invariants (aliasing, lifetimes, `Send`/`Sync` bounds, atomic orderings) against what the code does, not what the `SAFETY:` comment claims. Insist on the `unsafe-verify` skill (miri, loom, sanitizers, kani) for evidence.
- **Concurrency** — reason through interleavings explicitly; ask whether a loom test covers the new interleaving, not just whether tests pass.
- **Performance claims** — "should be faster" is not evidence. Require a criterion baseline comparison (`benchmark` skill) before endorsing any performance-motivated change; be prepared to report that the optimization does not measurably help.
- **Tests** — do the added tests actually fail without the fix? A test that passes on both sides of the change verifies nothing.

### Step 3: Verify instead of assuming

When the author (including the user) asserts "this is safe because X" or "callers never pass Y", check it: grep the callers, read the invariant, run the test. Run the relevant checks (`check` skill) rather than predicting they pass. A claim you did not verify is reported as unverified, not as fact.

### Step 4: Demonstrate each defect with a failing test

A finding is only confirmed when it fails executably. For every defect you intend to report, write a minimal test that fails on the reviewed code and would pass once fixed.

Prefer a randomized test (`merc_utilities::random_test`, seed reproducible via `MERC_SEED=<seed>`) over a single hard-coded example when possible: it forces the postcondition to be written as an explicit assertion over arbitrary inputs instead of one memorized output. Tests that must run under miri are the exception — keep those small and deterministic (randomized stress tests are far too slow for miri), asserting the same postcondition on a tiny case.

- **Logic bugs** — add a `#[test]` in the affected crate encoding the failure scenario, and show it failing: `cargo nextest run -p <crate> <test-name>` (or `cargo test`).
- **Undefined behaviour in unsafe code** — write a test that exercises the suspect path and run it under miri with the CI flags: `MIRIFLAGS="-Zmiri-disable-isolation --cfg chacha20_force_soft" cargo +nightly miri test -p <crate> <test-name>`. A miri error report is proof; quote it.
- **Data races / interleaving bugs** — write a loom test in the crate's loom-gated tests and run it with `RUSTFLAGS="--cfg loom" cargo test --release <test-name>`.

The same test becomes the regression test committed alongside the fix — do not delete it after demonstrating the failure. If you cannot make the defect fail in a test, report the finding as *plausible but not demonstrated*, never as confirmed.

### Step 5: Report findings honestly

- Rank by severity. Every defect claim needs a concrete failure scenario: which input or state leads to which wrong result — demonstrated by the failing test from Step 4 where one could be constructed. If you can construct neither, downgrade the finding to a question or drop it.
- State disagreement plainly. If the approach is wrong, say it is wrong and why — do not bury it under compliments or hedge it into "you might possibly consider".
- Do not manufacture nitpicks to appear thorough. If the code is correct, say so and list what you checked to conclude that; skepticism means being accurate, not negative.
- No praise padding. Skip "great work" openers entirely; the first sentence should be the verdict.
- If the user pushes back on a finding, re-examine the evidence. Change your position only if the evidence changes — and if it does not, say so and keep the finding.
