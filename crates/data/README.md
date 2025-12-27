# Overview

This crate defines a higher level representation of data expressions on top of
the first-order terms as provided by the `merc_aterm` crate. This is done by
introducing dedicated function symbols that are used as recognisers for various
types of data expressions, such as (sorted) variables, function applications,
abstractions (lambdas), and quantifiers. 

We use the same representation for data expressions as used in the
[mCRL2](https://mcrl2.org) toolset, since the choice for these function symbols
is rather arbitrary. In the future one could consider adding traits for data
expressions to make the representation independent of the actual representation,
but this has not been done yet.

 Data expressions are typically sorted (or typed), and these are represented by
`SortExpression`s. However, as opposed to the mCRL2 toolset we do not represent
the sort AST as a term, but rather as a dedicated Rust data type in the
`merc_syntax` crate. Furthermore, for rewriting purposes the types are not
important so we can also represent untyped data expressions, which can (easily)
be constructed from terms as shown below.

This crate also demonstrates the use of `merc_syntax` for defining higher-level
structure on top of first-order terms.

```rust
use ahash::AHashSet;

use merc_aterm::ATerm;
use merc_data::to_untyped_data_expression;

let term = ATerm::from_string("f(a, g(x))").unwrap();

// Consider 'x' as a variable, and everything else as function applications (or constants)
let data_expr = to_untyped_data_expression(term, Some(&AHashSet::from_iter(["x".to_string()])));
```

## Safety

This crate contains no unsafe code.

## Minimum Supported Rust Version

We do not maintain an official minimum supported rust version (MSRV), and it may be upgraded at any time when necessary.

## License

All MERC crates are licensed under the `BSL-1.0` license. See the [LICENSE](https://raw.githubusercontent.com/MERCorg/merc/refs/heads/main/LICENSE) file in the repository root for more information.