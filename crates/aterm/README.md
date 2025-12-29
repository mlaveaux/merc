# Overview

A thread-safe library to manipulate first-order terms. An first-order term is
defined by the following grammar:

```plain
    t := c | f(t1, ..., tn) | u64
```

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
are periodically garbage collected when they are no longer reachable. This is
ensured by thread-local protection sets that keep track of reachable terms. Note
that the name stands for **A**nnotated terms, but this is only a left over from
the history, as of today terms can no longer be annotated with additional
information.

## Usage

The main trait of the library is the `Term` trait, which is implemented by every
struct that behaves as a first-order term, and can be used to generically deal
with terms. The `Symb` trait does the same for function symbols.

The main implementations of this trait are the `ATerm` and `ATermRef` structs,
which represent owned and borrowed terms respectively. The `ATermRef` struct
carries a lifetime to ensure that borrowed terms are not used after they are no
longer protected, and as such avoid use-after-free errors. 

In general terms can simply be created by using their constructors:

```rust
use merc_aterm::{ATerm, Term, Symbol, Symb};

let a = ATerm::constant(&Symbol::new("a", 0));
let f = ATerm::with_args(&Symbol::new("f", 2), &[a.clone(), a]); // Creates f(a, a)

let g = ATerm::from_string("f(a, g(b))").unwrap(); // Parses the term from a string

assert!(g.arg(0).get_head_symbol().name() == "a"); // Access first argument of g
```

## Details

The crate is heavily optimised for performance, avoiding unnecessary allocations
for looking up terms that already exist, and avoiding protections when possible,
by using the `ATermRef` struct and the `Return` struct to cheaply return terms
without taking ownership. Furthermore, the `Protected` struct can be used to
cheaply store many terms in a single protection, for example by using
`Protected<Vec<ATermRef<'static>>>` to store a vector of terms.

The crate also provides serialization of terms to the same binary format that is
used in the mCRL2 toolset (implemented in the `aterm_binary_stream` module),
allowing compact storage of terms.

## Macros

The `merc_derive_terms` proc macro can be used to generate the necessary boiler
plate code for structs to behave as aterms with additional structure. This is
heavily used in the `merc_data` crate, but also in the term library to define
lists and integers. The macro must be added to a module that contains the
definitions for the underlying data types for which the boilerplate code should
be generated, as shown in the example below:

```rust
use merc_macros::merc_derive_terms;
use merc_aterm::ATerm;
use merc_aterm::Term;
use merc_aterm::Symbol;

#[merc_derive_terms]
mod inner {
    // Normally this can be `use super::*`, but that does not seem to work in doctests.
    use merc_aterm::ATerm;
    use merc_aterm::Term;

    use merc_macros::merc_term;
    use merc_macros::merc_ignore;
    use delegate::delegate;

    // These imports are used by the macro, but we cannot use the $crate trick since the macro is used
    // within the crate that defines these types.
    use merc_aterm::ATermRef;
    use merc_aterm::ATermArgs;
    use merc_aterm::ATermIndex;
    use merc_aterm::TermIterator;
    use merc_aterm::Symbol;
    use merc_aterm::SymbolRef;
    use merc_aterm::Markable;
    use merc_aterm::storage::Marker;
    use merc_aterm::Transmutable;

    // Uses the Term trait to specify a predicate for terms that are data expressions
    #[merc_ignore]
    fn is_data_expression<'a, 'b>(term: &'b impl Term<'a, 'b>) -> bool {
      true
    }

    #[merc_term(is_data_expression)]
    pub struct DataExpression {
        term: ATerm, // Must contain exactly one ATerm field
    }

    impl DataExpression {
        #[merc_ignore] // Ignore this method for code generation
        pub fn with_sort(expr: ATerm, sort: ATerm) -> Self {
            Self {
                term: ATerm::with_args(&Symbol::new("DataExpr", 2), &[expr, sort.into()]).protect(),
            }
        }

        // Custom methods can be added here
    }
}

use inner::DataExpression;
use inner::DataExpressionRef;

// Here we can now use the generated code:
let expr = DataExpression::with_sort(ATerm::constant(&Symbol::new("42", 0)), ATerm::constant(&Symbol::new("42", 0)));
let expr_ref: DataExpressionRef = expr.copy();
```

## Changelog

### Current

Removed the `ahash`, `arbitrary`, `arbtest`, and `rayon` dependencies since their use was only minimal.

## Safety

This crate does use `unsafe` for some of the more intricrate parts of the
library, but every module that only uses safe Rust is marked with
`#![forbid(unsafe_code)]`. This crate is a full reimplementation of the ATerm
library used in the [mCRL2](https://mcrl2.org) toolset.

## Related work

Further details on the implementation are explained in the following paper:

  > "Using the Parallel ATerm Library for Parallel Model Checking and State Space Generation". Jan Friso Groote, Kevin H.J. Jilissen, Maurice Laveaux, Flip van Spaendonck. [DOI](https://doi.org/10.1007/978-3-031-15629-8_16).

The initial ATerm library was presented by the following article:

  > "Efficient annotated terms". M. G. J. van den Brand, H. A. de Jong, P. Klint, P. A. Olivier. [DOI](https://doi.org/10.1002/(SICI)1097-024X(200003)30:3<259::AID-SPE298>3.0.CO;2-Y).
 
## Authors

This crate was heavily inspired by the original ATerm library, and many ideas from the original authors.

## Minimum Supported Rust Version

We do not maintain an official minimum supported rust version (MSRV), and it may be upgraded at any time when necessary.

## License

All MERC crates are licensed under the BSL-1.0 license. See the [LICENSE](https://raw.githubusercontent.com/MERCorg/merc/refs/heads/main/LICENSE) file in the repository root for more information.