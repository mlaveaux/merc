use proc_macro2::TokenStream;

use quote::ToTokens;
use quote::format_ident;
use quote::quote;
use syn::Item;
use syn::ItemMod;
use syn::parse_quote;

pub(crate) fn merc_derive_terms_impl(_attributes: TokenStream, input: TokenStream) -> TokenStream {
    // Parse the input tokens into a syntax tree
    let mut ast: ItemMod = syn::parse2(input.clone()).expect("merc_term can only be applied to a module");

    if let Some((_, content)) = &mut ast.content {
        // Generated code blocks are added to this list.
        let mut added = vec![];

        for item in content.iter_mut() {
            match item {
                Item::Struct(object) => {
                    // If the struct is annotated with term we process it as a term.
                    if let Some(attr) = object.attrs.iter().find(|attr| attr.meta.path().is_ident("merc_term")) {
                        // The #term(assertion) annotation must contain an assertion
                        let assertion = match attr.parse_args::<syn::Ident>() {
                            Ok(assertion) => {
                                let assertion_msg = format!("{assertion}");
                                quote!(
                                    debug_assert!(#assertion(&term), "Term {:?} does not satisfy {}", term, #assertion_msg)
                                )
                            }
                            Err(_x) => {
                                quote!()
                            }
                        };

                        // Add the expected derive macros to the input struct.
                        object
                            .attrs
                            .push(parse_quote!(#[derive(Clone, Debug, Hash, PartialEq, Eq, PartialOrd, Ord)]));

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

                            impl #generics Into<ATerm> for #name #generics{
                                fn into(self) -> ATerm {
                                    self.term
                                }
                            }

                            impl #generics Deref for #name #generics{
                                type Target = ATerm;

                                fn deref(&self) -> &Self::Target {
                                    &self.term
                                }
                            }

                            impl #generics Borrow<ATerm> for #name #generics{
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
                                        fn annotation(&self) -> Option<usize>;
                                    }
                                }
                            }

                            #[derive(Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
                            pub struct #name_ref #generics_ref {
                                pub(crate) term: ATermRef<'a>,
                                _marker: PhantomData #generics_phantom,
                            }

                            impl #generics_ref  #name_ref #generics_ref  {
                                pub fn copy<'b>(&'b self) -> #name_ref #generics_ref_b{
                                    self.term.copy().into()
                                }

                                pub fn protect(&self) -> #name {
                                    self.term.protect().into()
                                }
                            }

                            impl #generics_ref  From<ATermRef<'a>> for #name_ref #generics_ref {
                                fn from(term: ATermRef<'a>) -> #name_ref #generics_ref  {
                                    #assertion;
                                    #name_ref {
                                        term,
                                        _marker: PhantomData,
                                    }
                                }
                            }

                            impl #generics_ref  Into<ATermRef<'a>> for #name_ref #generics_ref  {
                                fn into(self) -> ATermRef<'a> {
                                    self.term
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
                                        fn annotation(&self) -> Option<usize>;
                                    }
                                }
                            }

                            impl #generics_ref Borrow<ATermRef<'a>> for #name_ref #generics_ref {
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

                            impl Transmutable for #name_ref #generics_static {
                                type Target #generics_ref = #name_ref #generics_ref;

                                fn transmute_lifetime<'a>(&self) -> &'a Self::Target #generics_ref {
                                    unsafe { transmute::<&Self, &'a #name_ref #generics_ref>(self) }
                                }

                                fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target #generics_ref {
                                    unsafe { transmute::<&mut Self, &'a mut #name_ref #generics_ref>(self) }
                                }
                            }
                        );

                        added.push(Item::Verbatim(generated));
                    }
                }
                Item::Impl(implementation) => {
                    if !implementation
                        .attrs
                        .iter()
                        .any(|attr| attr.meta.path().is_ident("merc_ignore"))
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

                        if let syn::Type::Path(path) = ref_implementation.self_ty.as_ref() {
                            let path = if let Some(identifier) = path.path.get_ident() {
                                // Build an identifier with the postfix Ref<'_>
                                let name_ref = format_ident!("{}Ref", identifier);
                                parse_quote!(#name_ref <'_>)
                            } else {
                                let path_segments = &path.path.segments;

                                let _name_ref = format_ident!(
                                    "{}Ref",
                                    path_segments
                                        .first()
                                        .expect("Path should at least have an identifier")
                                        .ident
                                );
                                // let segments: Vec<syn::PathSegment> = path_segments.iter().skip(1).collect();
                                // parse_quote!(#name_ref #segments)
                                unimplemented!()
                            };

                            ref_implementation.self_ty = Box::new(syn::Type::Path(syn::TypePath { qself: None, path }));

                            added.push(Item::Verbatim(ref_implementation.into_token_stream()));
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn test_macro() {
        let input = "
            mod anything {

                #[merc_term(test)]
                #[derive(Debug)]
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

        println!("{result}");
    }
}
