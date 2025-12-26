//!
//! This crate defines several macros to generate code for `ATerm` data types.
//!
//! This crate does not use unsafe code.

#![forbid(unsafe_code)]

use quote::quote;
use syn::ItemFn;
use syn::parse_macro_input;

mod merc_derive_terms;
use merc_derive_terms::merc_derive_terms_impl;

/// This proc macro can be used to automatically generate the boilerplate code
/// for structs to behave as a term.
///
/// # Details
///
/// For example `DataExpression`, `DataApplication` and `DataVariable` from the
/// `merc_data` crate are terms with additional functionality. This is achieved
/// by adding the proc macro to a module that contains both the declaration and
/// implementation of such a type, as shown below.
///
/// For every struct containing an `ATerm` we generate another version for the
/// `ATermRef` implementation, as well as `protect` and `copy` functions to
/// convert between both types. Furthermore, all of these can be converted to
/// and from `ATerm`s.
///
/// # Example
///
/// ```
/// use merc_macros::merc_derive_terms;
///
/// #[merc_derive_terms]
/// mod inner {
///
/// }
///
/// use inner::*;
/// ```
///
/// # Testing
///
/// There are a few procedural macros used to replace the code generation
/// performed in the mCRL2 toolset. Working on procedural macros is typically
/// difficult, but there are unit and integration tests to showcase common
/// patterns. Alternatively, install `cargo install cargo-expand` and run the
/// command `cargo expand` in for example `merc-macros` to print the Rust code
/// with the macros expanded.
#[proc_macro_attribute]
pub fn merc_derive_terms(
    _attributes: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    merc_derive_terms_impl(
        proc_macro2::TokenStream::from(_attributes),
        proc_macro2::TokenStream::from(input),
    )
    .into()
}

/// Marks a struct as a term.
#[proc_macro_attribute]
pub fn merc_term(_attributes: proc_macro::TokenStream, input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    input
}

/// Marks a function to be ignored, meaning the Ref term will not have this function
#[proc_macro_attribute]
pub fn merc_ignore(_attributes: proc_macro::TokenStream, input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    input
}

/// A macro that makes the function marked as a `#[test]` function and also initializes the test_logger().
#[proc_macro_attribute]
pub fn merc_test(_attr: proc_macro::TokenStream, item: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(item as ItemFn);

    let block = &input.block;
    let attrs = &input.attrs;
    let sig = &input.sig;

    let output = quote! {
        #[test]
        #(#attrs)*
        #sig {
            let __logger = merc_utilities::test_logger();
            #block
        }
    };

    output.into()
}
