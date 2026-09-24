use proc_macro2::Group;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;

use quote::ToTokens;
use quote::format_ident;
use quote::quote;
use syn::Item;
use syn::ItemMod;
use syn::parse_quote;

pub(crate) fn merc_derive_terms_impl(_attributes: TokenStream, input: TokenStream) -> TokenStream {
    // Parse the input tokens into a syntax tree
    let mut ast: ItemMod = syn::parse2(input.clone()).expect("merc_derive_terms can only be applied to a module");

    if let Some((_, content)) = &mut ast.content {
        // Generated code blocks are added to this list.
        let mut added = vec![];

        for item in content.iter_mut() {
            match item {
                Item::Struct(object) => {
                    // If the struct is annotated with term we process it as a term.
                    if let Some(attr) = object.attrs.iter().find(|attr| attr.meta.path().is_ident("merc_term")) {
                        // The #[merc_term(assertion)] annotation may name an
                        // assertion function. When present it must be a bare
                        // identifier; anything else is reported as a compile
                        // error rather than silently dropping the check.
                        let assertion = if attr.meta.require_list().is_ok() {
                            match attr.parse_args::<syn::Ident>() {
                                Ok(assertion) => {
                                    let assertion_msg = format!("{assertion}");
                                    quote!(
                                        debug_assert!(#assertion(&term), "Term {:?} does not satisfy {}", term, #assertion_msg)
                                    )
                                }
                                Err(error) => {
                                    let message =
                                        format!("merc_term expects a single assertion function identifier: {error}");
                                    quote!(compile_error!(#message))
                                }
                            }
                        } else {
                            // Bare `#[merc_term]` without arguments: no assertion.
                            quote!()
                        };

                        // Add the expected derive macros to the input struct.
                        object
                            .attrs
                            .push(parse_quote!(#[derive(Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]));

                        // ALL structs in this module must contain the term.
                        assert!(
                            object.fields.iter().any(|field| {
                                if let Some(name) = &field.ident {
                                    name == "term"
                                } else {
                                    false
                                }
                            }),
                            "The struct {} in mod {} has no field 'term: ATerm'",
                            object.ident,
                            ast.ident
                        );

                        let name = format_ident!("{}", object.ident);

                        // Simply the generics from the struct.
                        let generics = object.generics.clone();

                        // Helper to create generics with added lifetimes.
                        fn create_generics_with_lifetimes(
                            base_generics: &syn::Generics,
                            lifetime_names: &[&str],
                        ) -> syn::Generics {
                            let mut generics = base_generics.clone();
                            for &lifetime_name in lifetime_names {
                                generics.params.push(syn::GenericParam::Lifetime(syn::LifetimeParam {
                                    attrs: vec![],
                                    lifetime: syn::Lifetime::new(lifetime_name, proc_macro2::Span::call_site()),
                                    bounds: syn::punctuated::Punctuated::new(),
                                    colon_token: None,
                                }));
                            }
                            generics
                        }

                        // The generics from the struct with <'a, 'b> added for the Term trait.
                        let generics_term = create_generics_with_lifetimes(&object.generics, &["'a", "'b"]);

                        // Only 'a prepended for the Ref<'a> struct.
                        let generics_ref = create_generics_with_lifetimes(&object.generics, &["'a"]);

                        // Only 'b prepended for the Ref<'b> struct.
                        let generics_ref_b = create_generics_with_lifetimes(&object.generics, &["'b"]);

                        // Only 'static prepended for the Ref<'static> struct.
                        let generics_static = create_generics_with_lifetimes(&object.generics, &["'static"]);

                        // Handle PhantomData generics - use void type if no generics exist
                        let generics_phantom = if object.generics.params.is_empty() {
                            quote!(<()>)
                        } else {
                            generics.to_token_stream()
                        };

                        // Add a <name>Ref struct that contains the ATermRef<'a> and
                        // the implementation and both protect and borrow. Also add
                        // the conversion from and to an ATerm.
                        let name_ref = format_ident!("{}Ref", object.ident);
                        let generated: TokenStream = quote!(
                            impl #generics #name #generics {
                                pub fn copy #generics_ref(&'a self) -> #name_ref #generics_ref {
                                    self.term.copy().into()
                                }
                            }

                            impl #generics From<ATerm> for #name #generics {
                                fn from(term: ATerm) -> #name {
                                    #assertion;
                                    #name {
                                        term
                                    }
                                }
                            }

                            impl #generics ::std::convert::From<#name #generics> for ATerm {
                                fn from(value: #name #generics) -> ATerm {
                                    value.term
                                }
                            }

                            impl #generics ::std::ops::Deref for #name #generics{
                                type Target = ATerm;

                                fn deref(&self) -> &Self::Target {
                                    &self.term
                                }
                            }

                            impl #generics ::std::borrow::Borrow<ATerm> for #name #generics{
                                fn borrow(&self) -> &ATerm {
                                    &self.term
                                }
                            }

                            impl #generics Markable for #name #generics{
                                fn mark(&self, marker: &mut Marker) {
                                    self.term.mark(marker);
                                }

                                fn contains_term(&self, term: &ATermRef<'_>) -> bool {
                                    &self.term.copy() == term
                                }

                                fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool {
                                    self.get_head_symbol() == *symbol
                                }

                                fn len(&self) -> usize {
                                    1
                                }
                            }

                            impl ::std::fmt::Debug for #name #generics {
                                fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                                    write!(f, "{:?}", self.term)
                                }
                            }

                            impl #generics_term Term<'a, 'b> for #name #generics where 'b: 'a {
                                delegate! {
                                    to self.term {
                                        fn protect(&self) -> ATerm;
                                        fn arg(&'b self, index: usize) -> ATermRef<'a>;
                                        fn arguments(&'b self) -> ATermArgs<'a>;
                                        fn copy(&'b self) -> ATermRef<'a>;
                                        fn get_head_symbol(&'b self) -> SymbolRef<'a>;
                                        fn iter(&'b self) -> TermIterator<'a>;
                                        fn index(&self) -> usize;
                                        fn shared(&self) -> &ATermIndex;
                                    }
                                }
                            }

                            #[derive(Eq, Hash, Ord, PartialEq, PartialOrd)]
                            pub struct #name_ref #generics_ref {
                                pub(crate) term: ATermRef<'a>,
                                _marker: ::std::marker::PhantomData #generics_phantom,
                            }

                            impl #generics_ref  #name_ref #generics_ref  {
                                pub fn copy<'b>(&'b self) -> #name_ref #generics_ref_b{
                                    self.term.copy().into()
                                }

                                pub fn protect(&self) -> #name {
                                    self.term.protect().into()
                                }
                            }

                            impl #generics_ref ::std::convert::From<ATermRef<'a>> for #name_ref #generics_ref {
                                fn from(term: ATermRef<'a>) -> #name_ref #generics_ref  {
                                    #assertion;
                                    #name_ref {
                                        term,
                                        _marker: ::std::marker::PhantomData,
                                    }
                                }
                            }

                            impl #generics_ref ::std::convert::From<#name_ref #generics_ref> for ATermRef<'a> {
                                fn from(value: #name_ref #generics_ref) -> ATermRef<'a> {
                                    value.term
                                }
                            }

                            impl #generics_ref ::std::fmt::Debug for #name_ref #generics_ref {
                                fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                                    write!(f, "{:?}", self.term)
                                }
                            }

                            impl #generics_term Term<'a, '_> for #name_ref #generics_ref  {
                                delegate! {
                                    to self.term {
                                        fn protect(&self) -> ATerm;
                                        fn arg(&self, index: usize) -> ATermRef<'a>;
                                        fn arguments(&self) -> ATermArgs<'a>;
                                        fn copy(&self) -> ATermRef<'a>;
                                        fn get_head_symbol(&self) -> SymbolRef<'a>;
                                        fn iter(&self) -> TermIterator<'a>;
                                        fn index(&self) -> usize;
                                        fn shared(&self) -> &ATermIndex;
                                    }
                                }
                            }

                            impl #generics_ref ::std::borrow::Borrow<ATermRef<'a>> for #name_ref #generics_ref {
                                fn borrow(&self) -> &ATermRef<'a> {
                                    &self.term
                                }
                            }

                            impl #generics_ref Markable for #name_ref #generics_ref {
                                fn mark(&self, marker: &mut Marker) {
                                    self.term.mark(marker);
                                }

                                fn contains_term(&self, term: &ATermRef<'_>) -> bool {
                                    &self.term == term
                                }

                                fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool {
                                    self.get_head_symbol() == *symbol
                                }

                                fn len(&self) -> usize {
                                    1
                                }
                            }

                            // SAFETY: `#name_ref` is a `#[repr(Rust)]` wrapper whose only
                            // non-zero-sized field is `ATermRef<'a>`, which is itself a
                            // lifetime-erasable handle into the global term pool.
                            unsafe impl Transmutable for #name_ref #generics_static {
                                type Target #generics_ref = #name_ref #generics_ref;

                                unsafe fn transmute_lifetime<'a>(&self) -> &'a Self::Target #generics_ref {
                                    // SAFETY: see the trait impl comment above.
                                    unsafe { ::std::mem::transmute::<&Self, &'a #name_ref #generics_ref>(self) }
                                }

                                unsafe fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target #generics_ref {
                                    // SAFETY: see the trait impl comment above.
                                    unsafe { ::std::mem::transmute::<&mut Self, &'a mut #name_ref #generics_ref>(self) }
                                }
                            }
                        );

                        added.push(Item::Verbatim(generated));
                    }
                }
                Item::Impl(implementation)
                    if !implementation
                        .attrs
                        .iter()
                        .any(|attr| attr.meta.path().is_ident("merc_ignore")) =>
                {
                    // Duplicate the implementation for the Ref struct that is generated above.
                    let mut ref_implementation = implementation.clone();

                    // Remove ignored functions
                    ref_implementation.items.retain(|item| match item {
                        syn::ImplItem::Fn(func) => {
                            !func.attrs.iter().any(|attr| attr.meta.path().is_ident("merc_ignore"))
                        }
                        _ => true,
                    });

                    // Only `impl Name { .. }` blocks with a bare-identifier self
                    // type are duplicated; generic or path-qualified self types
                    // (e.g. `impl<T> Name<T>` or `impl module::Name`) are not yet
                    // supported and are reported as a clear compile error rather
                    // than panicking the macro or silently dropping the block.
                    match ref_implementation.self_ty.as_ref() {
                        syn::Type::Path(path) if path.path.get_ident().is_some() => {
                            let identifier = path.path.get_ident().expect("checked by the match guard");

                            // Build an identifier with the postfix Ref<'a>
                            let name_ref = format_ident!("{}Ref", identifier);

                            // Results borrowed from the underlying term outlive the
                            // `Ref` wrapper itself, so tie elided output lifetimes to
                            // `'a` rather than to `&self`.
                            ref_implementation.generics = parse_quote!(<'a>);
                            ref_implementation.self_ty = Box::new(parse_quote!(#name_ref <'a>));
                            for item in &mut ref_implementation.items {
                                if let syn::ImplItem::Fn(func) = item
                                    && let syn::ReturnType::Type(_, output) = &mut func.sig.output
                                {
                                    let tokens = replace_anonymous_lifetime(output.to_token_stream());
                                    **output = syn::parse2(tokens).expect("replacing a lifetime keeps the type valid");
                                }
                            }

                            added.push(Item::Verbatim(ref_implementation.into_token_stream()));
                        }
                        _ => {
                            let message = "merc_derive_terms can only duplicate impl blocks whose self type is a \
                                 bare identifier; generic or path-qualified self types are not yet supported. \
                                 Annotate the impl with #[merc_ignore] to skip it.";
                            added.push(Item::Verbatim(quote!(compile_error!(#message);)));
                        }
                    }
                }
                _ => {
                    // Ignore the rest.
                }
            }
        }

        content.append(&mut added);
    }

    // Hand the output tokens back to the compiler
    ast.into_token_stream()
}

/// Replaces every `'_` in `tokens` by `'a`.
fn replace_anonymous_lifetime(tokens: TokenStream) -> TokenStream {
    let mut result = Vec::new();
    let mut after_quote = false;
    for token in tokens {
        let token = match token {
            TokenTree::Ident(ident) if after_quote && ident == "_" => {
                TokenTree::Ident(proc_macro2::Ident::new("a", ident.span()))
            }
            TokenTree::Group(group) => {
                let mut replaced = Group::new(group.delimiter(), replace_anonymous_lifetime(group.stream()));
                replaced.set_span(group.span());
                TokenTree::Group(replaced)
            }
            other => other,
        };
        after_quote = matches!(&token, TokenTree::Punct(punct) if punct.as_char() == '\'');
        result.push(token);
    }
    result.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use proc_macro2::TokenStream;

    use crate::merc_derive_terms_impl;

    #[test]
    fn test_macro() {
        let input = "
            mod anything {

                #[merc_term(test)]
                struct Test {
                    term: ATerm,
                }

                impl Test {
                    fn a_function() {

                    }
                }
            }
        ";

        let tokens = TokenStream::from_str(input).unwrap();
        let result = merc_derive_terms_impl(TokenStream::default(), tokens);

        // The generated module must parse back as valid Rust and mention the
        // generated `TestRef` type.
        let rendered = result.to_string();
        syn::parse2::<syn::File>(result).expect("generated code should be valid Rust");
        assert!(
            rendered.contains("TestRef"),
            "expected a generated TestRef type, got: {rendered}"
        );
    }
}
