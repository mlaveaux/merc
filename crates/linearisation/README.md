# Overview

The `merc_linearisation` crate turns a type-checked
`merc_typecheck::ProcessSpecification` into a `LinearProcessSpecification`
(LPS): the condition/action/effect summand form the rest of merc — `merc_lts`,
`merc_explore`, `merc_symbolic` — is built around. It is a native, from-scratch
port of the linearisation algorithm mCRL2's `libraries/lps`
(`source/linearise.cpp`) implements, following the design and phasing recorded
in [`docs/linearisation-implementation-plan.md`](https://github.com/MERCorg/merc/blob/main/docs/linearisation-implementation-plan.md)
in the repository root.

## Status

This crate is under active development. Implemented so far:

- The `LinearProcessSpecification`/`ActionSummand`/`DeadlockSummand` data
  model (`specification.rs`) and a pretty-printer back to mCRL2 concrete
  syntax (`print.rs`).
- `translate::linearise`, covering the "regular", untimed, non-stochastic
  subset of the process algebra for a **single, self-recursive** process (no
  mutual recursion between distinct `proc` declarations, no `||`/`comm`, no
  `at`/`dist`; `allow`/`block`/`hide`/`rename` are all supported). See
  `translate.rs`'s module documentation for the exact supported grammar and
  the errors returned for everything else.
- `alphabet::simplify`, run automatically before translation: an
  alphabet-based static analysis (porting mCRL2's `process::alphabet_operations`
  / `process::alphabet_reduce` and the book's laws for these operators) that
  drops a `hide`/`block`/`allow`/`rename`/`comm` wrapper wherever it can prove
  the wrapper has no effect on its operand — including recognising a `comm`
  that can never actually fire, which `translate` alone does not support.
  See `alphabet.rs`'s module documentation for the precision/soundness
  argument and the safeguard against the wide-parallel-composition blow-up
  this kind of analysis is prone to.
- `compositional::linearise_compositional`, recognising a top-level `init`
  shaped as `hide(H, allow(A, comm(C, P1(...) || ... || Pn(...))))` (each of
  `H`/`A`/`C` optional) and linearising each `Pi` independently with the same
  core, producing a `tools/lts combine`-ready JSON manifest
  (`CompositionalSpecification::manifest`). See `compositional.rs`'s module
  documentation for why this needs no "cleave" step.

Not yet implemented: mutual recursion across multiple process equations
(needs a shared parameter vector / program-counter construction), and full
monolithic linearisation of `||`/`comm` anywhere but the compositional mode's
recognised top-level shape (state-operator elimination). These are the next
milestones in the implementation plan linked above.

## Usage

```rust
use merc_syntax::UntypedProcessSpecification;
use merc_typecheck::ProcessSpecification;
use merc_linearisation::linearise;

let untyped = UntypedProcessSpecification::parse(
    "act a;
     proc P(n: Nat) = (n > 0) -> a . P(Int2Nat(n - 1)) <> delta;
     init P(3);",
)
.unwrap();

let spec = ProcessSpecification::from_untyped(untyped).unwrap();
let lps = linearise(&spec).unwrap();

println!("{lps}");
```

## Safety

This crate contains no `unsafe` code.

## Minimum Supported Rust Version

We do not maintain an official minimum supported rust version (MSRV), and it may
be upgraded at any time when necessary.

## License

All MERC crates are licensed under the `BSL-1.0` license. See the
[LICENSE](https://raw.githubusercontent.com/MERCorg/merc/refs/heads/main/LICENSE)
file in the repository root for more information.
