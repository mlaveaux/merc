# Overview

This crate contains a complete [Pest](https://pest.rs/) grammar for the
[mCRL2](https://www.mcrl2.org/) specification language, along with functionality
to consume the resulting parse tree into an abstract syntax tree (AST). As
opposed to the mCRL2 toolset where the AST is represented by terms, we store the
AST in a Rust-idiomatic way using enums and structs. This means that we
potentially use more memory because there is no term sharing of expressions, but
manipulating the AST is much easier.

## Usage

The mCRL2 specifications can simply be parsed using the untyped variants of the
various objects: `UntypedProcessSpecification`, `UntypedDataSpecification`, etc.

```Rust
use merc_syntax::UntypedProcessSpecification;

let mcrl2_spec = UntypedProcessSpecification::parse("
    act
        a, b: Nat;

    proc
        P(n: Nat) = a(n).P(n + 1) + b(n).P(n - 1);

    init
        P(0);

").unwrap();
```

## Safety

This crate contains no unsafe code.

## Minimum Supported Rust Version

We do not maintain an official minimum supported rust version (MSRV), and it may be upgraded at any time when necessary.

## License

All MERC crates are licensed under the `BSL-1.0` license. See the [LICENSE](https://raw.githubusercontent.com/MERCorg/merc/refs/heads/main/LICENSE) file in the repository root for more information.