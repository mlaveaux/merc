# Overview

The `merc_linearisation` crate turns a `merc_process::ProcessSpecification`,
obtained by lowering a type-checked specification, into a
`merc_lps::LinearProcessSpecification` (LPS): a single process whose
behaviour is a set of summands `sum d. c -> a(e) . P(g)` over one vector of
parameters. It follows the "regular" method of mCRL2's `mcrl22lps`, for
specifications without time and without stochastic operators.

The specification must be in parallel pCRL format: parallel composition and
`allow`, `block`, `hide`, `rename` and `comm` may only be used outside
sequential and alternative composition. Successful termination is made
observable by the action `Terminate`, a reserved name.

Linearisation is a sequence of passes over the process terms of
`merc_process`, whose data are `merc_data` terms. The sequential processes are
brought into Greibach normal form, sequences of calls are replaced by single
calls, and every equation becomes a value of a control parameter. The
resulting linear processes are then combined along the parallel layer, where
`allow`, `block`, communication and sum elimination are applied while the
summands are generated.

## Usage

```rust
use merc_linearisation::LinearisationOptions;
use merc_syntax::UntypedProcessSpecification;
use merc_typecheck::ProcessSpecification;

let untyped = UntypedProcessSpecification::parse(
    "act a, b;
     proc P(n: Nat) = (n < 3) -> a . P(n + 1) <> b . delta;
     init P(0);",
)
.unwrap();

let spec = ProcessSpecification::from_untyped(untyped).unwrap().lower().unwrap();
let lps = merc_linearisation::linearise(&spec, merc_linearisation::rewriter, &LinearisationOptions::default()).unwrap();

let mut bytes = Vec::new();
merc_lps::write_lps(&lps, &mut bytes).unwrap();
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
