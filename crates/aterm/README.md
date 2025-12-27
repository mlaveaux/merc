# Overview

A thread-safe library to manipulate first-order terms. An first-order term is
defined by the following grammar:

    t := c | f(t1, ..., tn) | u64

where `f` is a function symbol with arity `n > 0` and a name, `c` is a function
symbol with arity zero and `u64` is a numerical term. As such `f(a, g(b))` is an
example of a term with constants `a` and `b`. However, in practice we can also
represent expressions such as `5 + 7 * 3 > 2` as terms or even computations such
as `sort([3, 1, 2])`, using appropriate function symbols for list concatenation
and the integers. These expressions are then typically manipulated by a term rewrite
engine, such as the one provided in `merc_sabre`.

Terms are stored maximally shared in the global aterm pool, meaning that two
terms are structurally equivalent if and only if they have the same memory
address. This allows for very efficient equality checking and compact memory
usage.

Terms are immutable, but can be accessed concurrently in different threads. They
are periodically garbage collected when they are no longer reachable. This is ensured
by thread-local protection sets that keep track of reachable terms.

The main trait of the library is the `Term` trait, which is implemented by every
struct that behaves as a first-order term, and can be used to generically deal
with terms. The main implementations of this trait are the `ATerm` and
`ATermRef` structs, which represent owned and borrowed terms respectively. The
`ATermRef` struct carries a lifetime to ensure that borrowed terms are not used
after they are no longer protected, and as such avoid use-after-free errors.

The crate is heavily optimised for performance, avoiding unnecessary allocations
for looking up terms that already exist, and avoiding protections when possible,
by using the `ATermRef` struct and the `Return` struct to cheaply return terms
without taking ownership. Furthermore, the `Protected` struct can be used to
cheaply store many terms in a single protection, for example by using
`Protected<Vec<ATermRef<'static>>>` to store a vector of terms.

The crate also provides serialization of terms to the same binary format that is
used in the mCRL2 toolset (implemented in the `aterm_binary_stream` module),
allowing compact storage of terms.

## Safety

This crate does use `unsafe` for some of the more intricrate parts of the
library, but every module that only uses safe Rust is marked with
`#![forbid(unsafe_code)]`. This crate is a full reimplementation of the ATerm
library used in the [mCRL2](https://mcrl2.org) toolset.

## Related work

Further details on the implementation are explained in the following paper:

  > Using the Parallel ATerm Library for Parallel Model Checking and State
 Space Generation. Jan Friso Groote, Kevin H.J. Jilissen, Maurice Laveaux,
 Flip van Spaendonck. [DOI](https://doi.org/10.1007/978-3-031-15629-8_16).

The initial ATerm library was presented by the following article:

  > Efficient annotated terms. M. G. J. van den Brand, H. A. de Jong, P.
 Klint, P. A. Olivier.
 [DOI](https://doi.org/10.1002/(SICI)1097-024X(200003)30:3<259::AID-SPE298>3.0.CO;2-Y).
 
## Authors

This crate was heavily inspired by the original ATerm library, and many ideas from the original authors.

## Minimum Supported Rust Version

We do not maintain an official minimum supported rust version (MSRV), and it may be upgraded at any time when necessary.

## License

All MERC crates are licensed under the BSL-1.0 license. See the [LICENSE](https://raw.githubusercontent.com/MERCorg/merc/refs/heads/main/LICENSE) file in the repository root for more information.