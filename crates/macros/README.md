# Overview

 > ⚠️ **important** This is an internal crate and is not intended for public use.

This internal crate defines procedural macros for the MERC project. The main
macro provided is the `merc_derive_terms` macro which generates the necessary
boilerplate for `ATerm` data types.

In Rust there is no inheritance mechanism like in object-oriented languages such
as `C++` or `Java` in which the ATerm library has been implemented originally.
However, one typically wants to add additional structure on top of the basic
`ATerm` type, for example to represent (typed) data expressions, variables,
sorts etc. 

The `merc_derive_terms` macro automatically generates the necessary boilerplate
code to convert between the custom data types and the underlying `ATerm`
representation, as well as implementing common traits such as `Clone`, `Debug`,
`PartialEq` and `Eq`. Furthermore, the (arguably) most important feature is that
it implements the `Ref<'_>` variant, similarly to `ATermRef`, which allows for
references without taking ownership (and as such incurring a protection) of the
underlying data. This avoids the need for `UB` casts as done in the original ATerm
library.

There is also a small utility macro called `merc_test` that can be used in place
of `#[test]` to define unit tests that automatically enable the logging
infrastructure used throughout MERC.

# Details

The proc macro must be added to a module that contains the definitions for the underlying
data types for which the boilerplate code should be generated, this is typically done
as followed:

```rust
use merc_macros::merc_derive_terms;
use merc_aterm::ATerm;

#[merc_derive_terms]
mod inner {

    #[merc_aterm(is_data_expression)] // is_data_expression is used to generate assertions that the term matches the expected value.
    struct DataExpression {
        aterm: ATerm, // Must contain exactly one ATerm field
    }

    impl DataExpression {
        #[merc_ignore] // Ignore this method for code generation
        pub fn with_sort(expr: ATerm, sort: DataSort) -> Self {
            Self {
                aterm: ATerm::with_args(&Symbol::new("DataExpr", 2), &[expr, sort.into()]),
            }
        }

        // Custom methods can be added here
    }
}
use inner::*;

// Here we can now use the generated code:
let expr = DataExpression::with_sort(ATerm::constant(42), DataSort::int());
let expr_ref: DataExpressionRef = expr.copy();
```

## Testing

Working with procedural macros is typically difficult, but there are unit and integration tests to showcase common patterns. Alternatively, install `cargo-expand` using `cargo install cargo-expand` and run the command `cargo expand` in for example `merc-macros` to print the Rust code with the macros expanded for debugging purposes.

## Safety

This crate does not use unsafe code.

## Minimum Supported Rust Version

We do not maintain an official minimum supported rust version (MSRV), and it may be upgraded at any time when necessary.

## License

All MERC crates are licensed under the `BSL-1.0` license. See the [LICENSE](https://raw.githubusercontent.com/MERCorg/merc/refs/heads/main/LICENSE) file in the repository root for more information.