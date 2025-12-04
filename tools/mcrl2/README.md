# Readme 

This is an experiment of mixing Rust code with the mCRL2 toolset directly. First
the submodules must initialised to obtain the 3rd-party libraries, as shown
below. Furthermore, we need a C++ compiler to build the mCRL2 toolset. This can
be Visual Studio on Windows, AppleClang on MacOS or either GCC or Clang on
Linux. In the latter case it uses whatever compiler is provided by the `CC` and
`CXX` environment variables.

    git submodule update --init --recursive
    cargo build

By default this will build in dev or debug mode, and a release build can be
obtained by passing --release. Note that it is necessary to run `git submodule update --init` 
after switching branches or pulling from the remote whenever any
of the modules have been changed.

# Overview

The `mcrl2-sys` crate uses `cxx` to set up a `C`-like FFI between the Rust code
and C++ code, with static assertions to ensure that the FFI matches and some
helpers for standard functionality such as `std::unique_ptr`. The resulting
interface is generally unpleasant to work with so the crate `mcrl2` has Rust
wrappers around the resulting functionality.

# Stack traces

The `mcrl2_cpptrace` feature can be enabled to provide stack traces on
exceptions and assertions in the mCRL2 code. This requires `cmake` to be
found on `PATH` to build the necessary components.

# IDEs

The `cc` crate used to build the mCRL2 toolset unfortunately does not generate a 
`compile_commands.json` that IDEs typically use to provide IDE support for C++ 
programs. There is a third-party tool called `bear` that can produce such a file
for Rust projects, including ones that build internal `C` libraries. From a fresh
`cargo clean` it can generate the necessary file by running the following command
from this directory:

    bear -- cargo build

It is also convenient to open the current directory directly in `vscode` since
opening the root directory can make it confused by the different workspaces.

# Caching

Since the C++ code is slow to compile due to the heavy usage of templated code
in header files it can be useful to install `sccache` with `cargo install
sccache` and define the `RUSTC_WRAPPER=sccache` environment variable. This can
be put into `.bashrc` for a more permanent solution. This cache will speed up 
both Rust and C++ compilation.
