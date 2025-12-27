use std::collections::HashMap;
use std::iter;

use quote::quote;
use syn::Error;
use syn::FnArg;
use syn::Ident;
use syn::ImplItem;
use syn::ImplItemFn;
use syn::ItemImpl;
use syn::LitBool;
use syn::Pat;
use syn::Path;
use syn::parse::Parse;
use syn::parse::ParseStream;
use syn::parse::Result;
use syn::parse_quote;
use syn::spanned::Spanned;
use syn::token;

mod kw {
    syn::custom_keyword!(shortcut);
    syn::custom_keyword!(rule);
    syn::custom_keyword!(parser);
}

/// Attributes for the parser macro
struct MakeParserAttrs {
    parser: Path,
    rule_enum: Path,
}

/// Arguments for an alias attribute
struct AliasArgs {
    target: Ident,
    is_shortcut: bool,
}

/// Source of an alias, including identifier and shortcut status.
struct AliasSrc {
    ident: Ident,      // Identifier
    is_shortcut: bool, // Whether it's a shortcut
}

/// Parsed function metadata including function body, name, input argument, and aliases.
struct ParsedFn<'a> {
    // Body of the function
    function: &'a mut ImplItemFn,
    // Name of the function.
    fn_name: Ident,
    // Name of the first argument of the function, which should be of type `Node`.
    input_arg: Ident,
    // List of aliases pointing to this function
    alias_srcs: Vec<AliasSrc>,
}

impl Parse for MakeParserAttrs {
    fn parse(input: ParseStream) -> Result<Self> {
        // By default, the pest parser is the same type as the pest_consume one
        let mut parser = parse_quote!(Self);
        // By default, use the `Rule` type in scope
        let mut rule_enum = parse_quote!(Rule);

        while !input.is_empty() {
            let lookahead = input.lookahead1();
            if lookahead.peek(kw::parser) {
                let _: kw::parser = input.parse()?;
                let _: token::Eq = input.parse()?;
                parser = input.parse()?;
            } else if lookahead.peek(kw::rule) {
                let _: kw::rule = input.parse()?;
                let _: token::Eq = input.parse()?;
                rule_enum = input.parse()?;
            } else {
                return Err(lookahead.error());
            }

            if input.peek(token::Comma) {
                let _: token::Comma = input.parse()?;
            } else {
                break;
            }
        }

        Ok(MakeParserAttrs { parser, rule_enum })
    }
}

impl Parse for AliasArgs {
    fn parse(input: ParseStream) -> Result<Self> {
        let target = input.parse()?;
        let is_shortcut = if input.peek(token::Comma) {
            // #[alias(rule, shortcut = true)]
            let _: token::Comma = input.parse()?;
            let _: kw::shortcut = input.parse()?;
            let _: token::Eq = input.parse()?;
            let b: LitBool = input.parse()?;
            b.value
        } else {
            // #[alias(rule)]
            false
        };
        Ok(AliasArgs { target, is_shortcut })
    }
}

/// Collects and maps aliases from an implementation block.
fn collect_aliases(imp: &mut ItemImpl) -> Result<HashMap<Ident, Vec<AliasSrc>>> {
    let functions = imp.items.iter_mut().flat_map(|item| match item {
        ImplItem::Fn(m) => Some(m),
        _ => None,
    });

    let mut alias_map = HashMap::new();
    for function in functions {
        let fn_name = function.sig.ident.clone();
        let mut alias_attrs = function.attrs.iter().filter(|attr| attr.path().is_ident("alias"));

        if let Some(attr) = alias_attrs.next() {
            let args: AliasArgs = attr.parse_args()?;
            alias_map.entry(args.target).or_insert_with(Vec::new).push(AliasSrc {
                ident: fn_name,
                is_shortcut: args.is_shortcut,
            });
        } else {
            // Self entry
            alias_map
                .entry(fn_name.clone())
                .or_insert_with(Vec::new)
                .push(AliasSrc {
                    ident: fn_name,
                    is_shortcut: false,
                });
        }
        if let Some(attr) = alias_attrs.next() {
            return Err(Error::new(attr.span(), "expected at most one alias attribute"));
        }
    }

    debug_assert!(!alias_map.is_empty(), "Alias map should not be empty after collection");
    Ok(alias_map)
}

/// Extracts an identifier from a function argument.
fn extract_ident_argument(input_arg: &FnArg) -> Result<Ident> {
    match input_arg {
        FnArg::Receiver(_) => Err(Error::new(input_arg.span(), "this argument should not be `self`")),
        FnArg::Typed(input_arg) => match &*input_arg.pat {
            Pat::Ident(pat) => Ok(pat.ident.clone()),
            _ => Err(Error::new(
                input_arg.span(),
                "this argument should be a plain identifier instead of a pattern",
            )),
        },
    }
}

/// Parses a function to extract metadata for rule method processing.
fn parse_fn<'a>(function: &'a mut ImplItemFn, alias_map: &mut HashMap<Ident, Vec<AliasSrc>>) -> Result<ParsedFn<'a>> {
    // Rule methods must have exactly one argument
    if function.sig.inputs.len() != 1 {
        return Err(Error::new(
            function.sig.inputs.span(),
            "A rule method must have 1 argument",
        ));
    }

    let fn_name = function.sig.ident.clone();
    // Get the name of the first function argument
    let input_arg = extract_ident_argument(&function.sig.inputs[0])?;
    let alias_srcs = alias_map.remove(&fn_name).unwrap_or_default();

    debug_assert!(
        alias_srcs.iter().any(|src| src.ident == fn_name),
        "Function should have at least a self-reference in alias sources"
    );

    Ok(ParsedFn {
        function,
        fn_name,
        input_arg,
        alias_srcs,
    })
}

/// Applies special attributes to parsed functions.
fn apply_special_attrs(f: &mut ParsedFn, rule_enum: &Path) -> Result<()> {
    let function = &mut *f.function;
    let fn_name = &f.fn_name;
    let input_arg = &f.input_arg;

    // `alias` attr
    // f.alias_srcs has always at least 1 element because it has an entry pointing from itself.
    let aliases = f.alias_srcs.iter().map(|src| &src.ident).filter(|i| i != &fn_name);
    let block = &function.block;
    let self_ty = quote!(<Self as ::merc_pest_consume::Parser>);

    // Modify function block to handle shortcuts and aliases
    function.block = parse_quote!({
        let mut #input_arg = #input_arg;
        // While the current rule allows shortcutting, and there is a single child, and the
        // child can still be parsed by the current function, then skip to that child.
        while #self_ty::allows_shortcut(#input_arg.as_rule()) {
            if let ::std::result::Result::Ok(child)
                    = #input_arg.children().single() {
                if child.as_aliased_rule::<Self>() == #self_ty::rule_alias(#rule_enum::#fn_name) {
                    #input_arg = child;
                    continue;
                }
            }
            break
        }

        match #input_arg.as_rule() {
            #(#rule_enum::#aliases => Self::#aliases(#input_arg),)*
            #rule_enum::#fn_name => #block,
            r => panic!(
                "merc_pest_consume::parser: called the `{}` method on a node with rule `{:?}`",
                stringify!(#fn_name),
                r
            )
        }
    });

    debug_assert!(
        !f.alias_srcs.is_empty(),
        "Function must have at least one alias source (itself)"
    );
    Ok(())
}

/// Main function for generating the parser implementation.
pub fn make_parser(attrs: proc_macro::TokenStream, input: proc_macro::TokenStream) -> Result<proc_macro2::TokenStream> {
    let attrs: MakeParserAttrs = syn::parse(attrs)?;
    let parser = &attrs.parser;
    let rule_enum = &attrs.rule_enum;
    let mut imp: ItemImpl = syn::parse(input)?;

    // Collect aliases and build rule matching logic
    let mut alias_map = collect_aliases(&mut imp)?;
    let rule_alias_branches: Vec<_> = alias_map
        .iter()
        .flat_map(|(tgt, srcs)| iter::repeat(tgt).zip(srcs))
        .map(|(tgt, src)| {
            let ident = &src.ident;
            quote!(
                #rule_enum::#ident => Self::AliasedRule::#tgt,
            )
        })
        .collect();
    let aliased_rule_variants: Vec<_> = alias_map.keys().cloned().collect();
    let shortcut_branches: Vec<_> = alias_map
        .iter()
        .flat_map(|(_tgt, srcs)| srcs)
        .map(|AliasSrc { ident, is_shortcut }| {
            quote!(
                #rule_enum::#ident => #is_shortcut,
            )
        })
        .collect();

    // Process functions and apply attributes
    let fn_map: HashMap<Ident, ParsedFn> = imp
        .items
        .iter_mut()
        .flat_map(|item| match item {
            ImplItem::Fn(m) => Some(m),
            _ => None,
        })
        .map(|method| {
            *method = parse_quote!(
                #[allow(non_snake_case)]
                #method
            );

            let mut f = parse_fn(method, &mut alias_map)?;
            apply_special_attrs(&mut f, rule_enum)?;
            Ok((f.fn_name.clone(), f))
        })
        .collect::<Result<_>>()?;

    // Create functions for any remaining aliases
    let extra_fns: Vec<_> = alias_map
        .iter()
        .map(|(tgt, srcs)| {
            // Get the signature of one of the functions that has this alias
            let f = fn_map.get(&srcs.first().unwrap().ident).unwrap();
            let input_arg = f.input_arg.clone();
            let mut sig = f.function.sig.clone();
            sig.ident = tgt.clone();
            let srcs = srcs.iter().map(|src| &src.ident);

            Ok(parse_quote!(
                #sig {
                    match #input_arg.as_rule() {
                        #(#rule_enum::#srcs => Self::#srcs(#input_arg),)*
                        // We can't match on #rule_enum::#tgt since `tgt` might be an arbitrary
                        // identifier.
                        r if &format!("{:?}", r) == stringify!(#tgt) =>
                            return ::std::result::Result::Err(#input_arg.error(format!(
                                "merc_pest_consume::parser: missing method for rule {}",
                                stringify!(#tgt),
                            ))),
                        r => return ::std::result::Result::Err(#input_arg.error(format!(
                            "merc_pest_consume::parser: called method `{}` on a node with rule `{:?}`",
                            stringify!(#tgt),
                            r
                        ))),
                    }
                }
            ))
        })
        .collect::<Result<_>>()?;
    imp.items.extend(extra_fns);

    // Generate the final implementation
    let ty = &imp.self_ty;
    let (impl_generics, _, where_clause) = imp.generics.split_for_impl();

    debug_assert!(
        !aliased_rule_variants.is_empty(),
        "Must have at least one aliased rule variant"
    );

    Ok(quote!(
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[allow(non_camel_case_types)]
        pub enum AliasedRule {
            #(#aliased_rule_variants,)*
        }

        impl #impl_generics ::merc_pest_consume::Parser for #ty #where_clause {
            type Rule = #rule_enum;
            type AliasedRule = AliasedRule;
            type Parser = #parser;
            fn rule_alias(rule: Self::Rule) -> Self::AliasedRule {
                match rule {
                    #(#rule_alias_branches)*
                    // TODO: return a proper error ?
                    r => panic!("Rule `{:?}` does not have a corresponding parsing method", r),
                }
            }
            fn allows_shortcut(rule: Self::Rule) -> bool {
                match rule {
                    #(#shortcut_branches)*
                    _ => false,
                }
            }
        }

        #imp
    ))
}
