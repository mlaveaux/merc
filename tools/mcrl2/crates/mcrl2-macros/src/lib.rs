//!
//! This crate defines several macros to make ATerm data types work.
//!
//! This crate does not use unsafe code.

#![forbid(unsafe_code)]

mod mcrl2_derive_terms;

use mcrl2_derive_terms::mcrl2_derive_terms_impl;

/// This proc macro can be used to generate implementations for the types stored
/// in an ATerm, for example data_expressions, applications, variables. This is
/// achieved by adding the proc macro to a module that contains both the
/// declaration and implementation of such a type.
///
/// For every struct containing an ATerm we generate another version for the
/// ATermRef implementation, as well as `protect` and `borrow` functions to
/// convert between both types. Furthermore, all of these can be converted to
/// and from ATerms.
///
/// # Example
#[proc_macro_attribute]
pub fn mcrl2_derive_terms(
    _attributes: proc_macro::TokenStream,
    input: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    mcrl2_derive_terms_impl(
        proc_macro2::TokenStream::from(_attributes),
        proc_macro2::TokenStream::from(input),
    )
    .into()
}

/// Marks a struct as a term.
#[proc_macro_attribute]
pub fn mcrl2_term(_attributes: proc_macro::TokenStream, input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    input
}

/// Marks a function to be ignored, meaning the Ref term will not have this function
#[proc_macro_attribute]
pub fn mcrl2_ignore(_attributes: proc_macro::TokenStream, input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    input
}