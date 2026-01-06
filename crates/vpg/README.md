
# Overview

This crate provides functionality for working with variability parity games.
This includes reading and writing for parity games in the
[PGSolver](https://github.com/tcsprojects/pgsolver) `.pg` format. For
variability parity games this format is extended with feature configurations
encoded as BDDs on the edges, with a corresponding `.vpg` format. These games
can be solved using Zielonka's recursive algorithm, displayed in
[Graphviz](https://graphviz.org/) `DOT` format and generated from modal
mu-calculus formulas.

A central `PG` or parity game trait is used to allow writing generic algoritms
for parity games. Various helpers are introduced for working with `strong` types
for priorities, explicitly representing the even and odd players etc. This crate
uses [OxiDD](https://oxidd.net/) for the binary decision diagrams.

## Usage

Reading a `.pg` from disk and subsequently solving it can simply be done as
follows.

```rust
use merc_vpg::read_pg;
use merc_vpg::solve_zielonka;

let parity_game = read_pg(b"parity 3;
0 0 0 1;
1 0 0 2;
2 1 0 2;
" as &[u8]).unwrap();

// Solve the game, produces a full solution for all vertices.
let solution = solve_zielonka(&parity_game);
```

## Changelog

### Current

Added the `clap` feature to conditionally enable the `clap` dependency to derive
some convenience traits.

Optimised the implementation to avoid unnecessary `with_manager_shared` calls
when operating on BDDs. This is important since `oxidd` is otherwise not
efficient when doing many individual operations.

## Authors

The implementation of this crate was developed by Sjef van Loo and Maurice
Laveaux. The theoretical foundations were laid by Maurice Ter Beek, Erik de Vink
and Tim A.C. Willemse, in the following publication:

  > "Family-Based Model Checking Using Variability Parity Games". Maurice Ter Beek, Maurice Laveaux, Sjef van Loo, Erik de Vink and Tim A.C. Willemse. XXX.

## Safety

This crate contains no unsafe code.

## Minimum Supported Rust Version

We do not maintain an official minimum supported rust version (MSRV), and it may be upgraded at any time when necessary.

## License

All MERC crates are licensed under the `BSL-1.0` license. See the [LICENSE](https://raw.githubusercontent.com/MERCorg/merc/refs/heads/main/LICENSE) file in the repository root for more information.