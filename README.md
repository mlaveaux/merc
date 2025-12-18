# Overview

This repository contains implementations of various model checking related algorithm. For now this includes inspecting, reducing and solving requirements on labelled transitions systems. Its name is an acronym for "**m**CRL2 **e**xcept **R**eliable & **C**oncurrent", which should not be taken literal. The main goal is demonstrate efficient and correct implementations using safe Rust, where applicable. Furthermore, it should serve as a basis for wider experimentation of model checking implemented in the Rust language. The toolset is developed at the department of Mathematics and Computer Science of the [Technische Universiteit Eindhoven](https://fsa.win.tue.nl/).

## Contributing

The toolset is still in quite early stages, but contributions and ideas are more than welcome. Compilation requires at least rustc version 1.85.0 and we use 2024 edition rust. Then the toolset can be build using `cargo build`, by default this will build in `dev` or debug mode, and a release build can be obtained by passing `--release`. Several tools will be build that can be found in the `target/{debug, release}` directory. Some tools are implemented in different workspaces since their dependencies do not match the general tool set. See `CONTRIBUTING.md` for more information on the source code. Report bugs in the [issue tracker](https://github.com/mlaveaux/merc/issues).

## Tools

Various tools have been implemented so far:
 - `merc-lts` implement various bisimulation reductions for labelled transition systems in the mCRL2 binary `.lts` format and the Aldebaran `.aut` format.
 - `merc-rewrite` allows rewriting of [REC](https://doi.org/10.1007/978-3-030-17502-3_6) specifications.
 - `merc-vpg` can be used to solve (variability) parity games in the `.(v)pg` format.
 - `merc-pbes` can identify symmetries in paramerised boolean equation systems (PBES), located in the `tools/mcrl2` workspace
 - `merc-ltsgraph` is a GUI tool to visualize LTSs, located in the `tools/GUI` workspace.

## License

The work is licensed under the Boost Software License, see the `LICENSE` for details.

## Related Work

This tool set is inspired by the work on [mCRL2](https://github.com/mCRL2org/mCRL2).
