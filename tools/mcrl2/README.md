# Readme 

This is an experiment of mixing Rust code with the mCRL2 toolset directly. First
the submodules must initialised to obtain the 3rd-party libraries. Furthermore,
we need a C++ compiler to build the mCRL2 toolset. This can be Visual Studio on
Windows, AppleClang on MacOS or either GCC or Clang on Linux. In the latter case
it uses whatever compiler is provided by the CXX environment variable. After
that the cargo workspace can be build. This will also build the necessary
components of the mCRL2 toolset, which can take some time.

    git submodule update --init --recursive
    cargo build

By default this will build in dev or debug mode, and a release build can be
obtained by passing --release. Note that it is necessary to run `git submodule
update` after switching branches or pulling from the remote whenever any of the
modules have been changed.

# Setup

This workspace uses the crate `cxx` in `mcrl2-sys` to set up a `C`-like FFI
between the Rust code and C++ code, with static assertions and some helpers for
standard functionality such as `std::unique_ptr`. The resulting interface is
generally unpleasant to work with so the crate `mcrl2` has Rust wrappers around
the resulting functionality.

# Contributing

The `cc` crate used to build the mCRL2 toolset unfortunately does not generate a 
`compile_commands.json` that IDEs typically use to provide IDE support for C++ 
programs. There is a third-party tool called `bear` that can produce such a file
for Rust projects, including ones that build internal `C` libraries.

    bear -- cargo build