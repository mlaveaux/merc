# Overview

This repository contains a re-implementation of the core functionality of the [mCRL2](https://mcrl2.org) toolset in the Rust programming language. Its name is an acronym for "**m**CRL2 **e**xcept **R**eliable & **C**oncurrent", which should not be taken literal. The main goal is demonstrate a correct implementation using (mostly) safe Rust, with a secondary goal to achieve similar performance to the C++ toolset.

## Contributing

Compilation requires at least rustc version 1.85.0 and we use 2024 edition rust. By default this will build in `dev` or debug mode, and a release build can be obtained by passing `--release`. Source code documentation can be found at Github [pages](https://mlaveaux.github.io/merc/merc/index.html), and more detailed documentation can be found `doc`.

## Formatting

All source code should be formatted using `cargo fmt`, which can installed using `rustup component add rustfmt`. Individual source files can then be formatted using `cargo +nightly fmt`.

## Third party libraries

We generally strive for using high quality third party dependencies, we use `cargo deny check`, installed with `cargo install cargo-deny` to check the license of third party libraries and to compare them to the `RustSec` advisory db. In general unmaintained dependencies should either be vendored or replaced by own code if possible. However, using third party libraries where applicable is generally not discouraged.

## Related Work

This library is fully inspired by the work on [mCRL2](https://github.com/mCRL2org/mCRL2).