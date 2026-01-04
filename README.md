# Overview

The goal of the `MERC` project is to provide a generic set of libraries and tools for (specification language-agnostic) model checking, written in the [Rust](https://rust-lang.org/) language. The name is an acronym for "[**m**CRL2](https://www.mcrl2.org/web/index.html) **e**xcept **R**eliable & **C**oncurrent", which should not be taken literally. 

We aim to demonstrate efficient and correct implementations using Rust. The main focus is on clean interfaces to allow the libraries to be reused as well. The toolset supports and is tested on all major platforms: Linux, macOS and Windows.

## Contributing

The toolset is still in quite early stages, but contributions and ideas are more than welcome so feel free to contact the authors or open discussion. Compilation requires at least rustc version 1.85.0 and we use 2024 edition rust. The toolset can be built using `cargo build`. By default this will build in `dev` or debug mode, and a release build can be obtained by passing the `--release` flag. Several tools will be built that can be found in the `target/debug` (or `release`) directory. See [`CONTRIBUTING.md`](./CONTRIBUTING.md) for more information on the testing and formatting of code. Copilot is used for reviewing and occasionally boiler plate code can be written by AI, but slop is strictly forbidden. Extensive (random) testing under various [sanitizers](https://github.com/google/sanitizers/wiki/addresssanitizer) and [miri](https://github.com/rust-lang/miri) is used to gain confidence in the `unsafe` parts of the implementation.

Bugs and issues can be reported in the [issue tracker](https://github.com/MERCorg/merc/issues).

## Tools

Various tools have been implemented so far:
 - `merc-lts` implement various (signature-based) bisimulation algorithms for labelled transition systems in the mCRL2 binary [`.lts`](https://www.mcrl2.org/web/user_manual/tools/lts.html) format and the AUTomaton (or ALDEBARAN) [`.aut`](https://cadp.inria.fr/man/aut.html) format.
 - `merc-rewrite` allows rewriting of Rewrite Engine Competition specifications ([REC](https://doi.org/10.1007/978-3-030-17502-3_6)) using [Sabre](https://arxiv.org/abs/2202.08687) (**S**et **A**utomaton **B**ased **RE**writing).
 - `merc-vpg` can be used to solve (variability) parity games in the [PGSolver](https://github.com/tcsprojects/pgsolver) `.pg` format, and a slightly extended variability parity game `.vpg` format. Furthermore, it can generate variability parity games for model checking modal mu-calculus on LTSs.
 - `merc-pbes` can identify symmetries in parameterised boolean equation systems [PBES](https://doi.org/10.1016%2Fj.tcs.2005.06.016), located in the `tools/mcrl2` workspace.
 - `merc-ltsgraph` is a GUI tool to visualize LTSs, located in the `tools/GUI` workspace.

Various crates are also published on [crates.io](https://crates.io/users/mlaveaux), see the [crates](./crates) directory for an overview.

## License

The work is licensed under the Boost Software License, see the [`LICENSE`](./LICENSE) for details. Third party dependencies have additional license terms, which are included in the `3rd-party` directory. Furthermore, `cargo deny` is used to ensure that no `crates.io` dependencies with incompatible licenses are added.

## Related Work

This tool set is inspired by the work on the [mCRL2](https://github.com/mCRL2org/mCRL2) toolset, the work on a specification language agnostic toolset [ltsmin](https://ltsmin.utwente.nl/) and the work on [CADP](https://cadp.inria.fr/).

This project is developed at the department of Mathematics and Computer Science of the [Technische Universiteit Eindhoven](https://fsa.win.tue.nl/).