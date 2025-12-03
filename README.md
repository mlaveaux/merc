# Overview

This repository contains a re-implementation of the core functionality of the [mCRL2](https://mcrl2.org) toolset in the Rust programming language. Its name is an acronym for "**m**CRL2 **e**xcept **R**eliable & **C**oncurrent", which should not be taken literal. The main goal is demonstrate a correct implementation using (mostly) safe Rust, with a secondary goal to achieve similar performance to the C++ toolset.

## Building

Compilation requires at least rustc version 1.85.0 and we use 2024 edition rust. Then the toolset can be build using `cargo build`, by default this will build in `dev` or debug mode, and a release build can be obtained by passing `--release`. Several tools will be build that can be found in the `target/{debug, release}` directory. The `GUI` tools have to be build separatedly by running the build from the `tools/gui` directory.

## Tools

 - merc-lts implement various bisimulation reductions.
 - merc-ltsgraph is a GUI to visualize LTSs.

## Related Work

This library is fully inspired by the work on [mCRL2](https://github.com/mCRL2org/mCRL2).