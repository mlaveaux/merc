use proc_macro2::Span;
use proc_macro2::TokenStream;
use proc_macro2::TokenTree;
use quote::quote;
use syn::Expr;
use syn::Ident;
use syn::Pat;
use syn::Type;
use syn::bracketed;
use syn::parenthesized;
use syn::parse::Parse;
use syn::parse::ParseStream;
use syn::parse::Result;
use syn::parse_quote;
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::token;

/// Pattern for matching a node in a pattern matching expression.
///
/// Represents an individual pattern element with optional tag, rule name,
/// binding pattern, and multiplicity flag.
struct Pattern {
    tag: Option<String>,      // Optional tag for the pattern
    rule_name: Option<Ident>, // Optional rule name for parsing
    binder: Pat,              // Pattern to bind the matched node to
    multiple: bool,           // Whether this pattern matches multiple nodes
}

/// Alternative in a pattern matching expression.
///
/// Represents a sequence of patterns that can be matched against nodes.
struct Alternative {
    patterns: Punctuated<Pattern, token::Comma>, // Comma-separated patterns
}

/// Branch in a pattern matching expression.
///
/// Represents a set of alternatives and their associated body expression.
struct MatchBranch {
    alternatives: Punctuated<Alternative, token::Or>, // Alternatives separated by |
    body: Expr,                                       // Body expression to evaluate on match
}

/// Input for the match_nodes macro.
///
/// Contains parser type, input expression, and pattern matching branches.
struct MacroInput {
    parser: Type,                                    // Parser type
    input_expr: Expr,                                // Expression to match against
    branches: Punctuated<MatchBranch, token::Comma>, // Pattern matching branches
}

impl Parse for MacroInput {
    fn parse(input: ParseStream) -> Result<Self> {
        let parser = if input.peek(token::Lt) {
            let _: token::Lt = input.parse()?;
            let parser = input.parse()?;
            let _: token::Gt = input.parse()?;
            let _: token::Semi = input.parse()?;
            parser
        } else {
            parse_quote!(Self)
        };

        let input_expr = input.parse()?;
        let _: token::Semi = input.parse()?;
        let branches = Punctuated::parse_terminated(input)?;

        Ok(MacroInput {
            parser,
            input_expr,
            branches,
        })
    }
}

impl Parse for MatchBranch {
    fn parse(input: ParseStream) -> Result<Self> {
        let alternatives = Punctuated::parse_separated_nonempty(input)?;
        let _: token::FatArrow = input.parse()?;
        let body = input.parse()?;

        Ok(MatchBranch { alternatives, body })
    }
}

impl Parse for Alternative {
    fn parse(input: ParseStream) -> Result<Self> {
        let contents;
        let _: token::Bracket = bracketed!(contents in input);
        let patterns = Punctuated::parse_terminated(&contents)?;
        Ok(Alternative { patterns })
    }
}

impl Parse for Pattern {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut tag = None;
        let binder: Pat;
        let multiple;
        let rule_name;

        let ahead = input.fork();
        let _: TokenTree = ahead.parse()?;
        if ahead.peek(token::Pound) {
            let tag_ident: Ident = input.parse()?;
            tag = Some(tag_ident.to_string());
            let _: token::Pound = input.parse()?;
        }

        let ahead = input.fork();
        let _: TokenTree = ahead.parse()?;
        if ahead.peek(token::Paren) {
            // If `input` starts with `foo(`
            rule_name = Some(input.parse()?);
            let contents;
            parenthesized!(contents in input);
            binder = Pat::parse_multi(&contents)?;
        } else {
            // A plain pattern captures the node itself without parsing anything.
            rule_name = None;
            binder = Pat::parse_multi(input)?;
        }

        if input.peek(token::DotDot) {
            let _: token::DotDot = input.parse()?;
            multiple = true;
        } else if input.is_empty() || input.peek(token::Comma) {
            multiple = false;
        } else {
            return Err(input.error("expected `..` or nothing"));
        }

        Ok(Pattern {
            tag,
            rule_name,
            binder,
            multiple,
        })
    }
}

/// Traverses a pattern and generates code to match against nodes.
fn traverse_pattern(
    mut patterns: &[Pattern],
    i_iter: &Ident,
    matches_pat: impl Fn(&Pattern, TokenStream) -> TokenStream,
    process_item: impl Fn(&Pattern, TokenStream) -> TokenStream,
    error: TokenStream,
) -> TokenStream {
    let mut steps = Vec::new();

    // Handle trailing single patterns first for correct non-greedy matching
    while patterns.last().is_some_and(|pat| !pat.multiple) {
        let [remaining_pats @ .., pat] = patterns else {
            unreachable!()
        };

        patterns = remaining_pats;
        let this_node = process_item(pat, quote!(node));
        steps.push(quote!(
            let Some(node) = #i_iter.next_back() else { #error };
            #this_node;
        ));
    }

    // Process remaining patterns
    for pat in patterns {
        if !pat.multiple {
            // Single pattern - match exactly one node
            let this_node = process_item(pat, quote!(node));
            steps.push(quote!(
                let Some(node) = #i_iter.next() else { #error };
                #this_node;
            ));
        } else {
            // Multiple pattern - match greedily as long as nodes match
            let matches_node = matches_pat(pat, quote!(node));
            let this_slice = process_item(pat, quote!(matched));
            steps.push(quote!(
                let matched = <_ as ::merc_pest_consume::Itertools>::peeking_take_while(&mut #i_iter, |node| #matches_node);
                #this_slice;
            ))
        }
    }

    debug_assert!(
        !steps.is_empty() || patterns.is_empty(),
        "Must generate steps for non-empty patterns"
    );

    quote!(
        #[allow(unused_mut)]
        let mut #i_iter = #i_iter.peekable();
        #(#steps)*
    )
}

/// Generates code for a single pattern matching alternative.
fn make_alternative(
    alternative: Alternative,
    body: &Expr,
    i_nodes: &Ident,
    i_node_namer: &Ident,
    parser: &Type,
) -> TokenStream {
    let i_nodes_iter = Ident::new("___nodes_iter", Span::call_site());
    let name_enum = quote!(<#parser as ::merc_pest_consume::NodeMatcher>::NodeName);
    let node_namer_ty = quote!(<_ as ::merc_pest_consume::NodeNamer<#parser>>);
    let patterns: Vec<_> = alternative.patterns.into_iter().collect();

    // Function to generate code for checking if a pattern matches a node
    let matches_pat = |pat: &Pattern, x| {
        let rule_cond = match &pat.rule_name {
            Some(rule_name) => {
                quote!(#node_namer_ty::node_name(&#i_node_namer, &#x) == #name_enum::#rule_name)
            }
            None => quote!(true),
        };
        let tag_cond = match &pat.tag {
            Some(tag) => {
                quote!(#node_namer_ty::tag(&#i_node_namer, &#x) == Some(#tag))
            }
            None => quote!(true),
        };
        quote!(#rule_cond && #tag_cond)
    };

    // Generate code for checking if this alternative matches
    let process_item = |pat: &Pattern, i_matched| {
        if !pat.multiple {
            let cond = matches_pat(pat, i_matched);
            quote!(
                if !(#cond) { return false; }
            )
        } else {
            quote!(
                // Consume the iterator.
                #i_matched.count();
            )
        }
    };

    let conditions = traverse_pattern(
        patterns.as_slice(),
        &i_nodes_iter,
        matches_pat,
        process_item,
        quote!(return false),
    );

    // Generate code for parsing nodes when the alternative matches
    let parse_rule = |rule: &Option<_>, node| match rule {
        Some(rule_name) => quote!(#parser::#rule_name(#node)),
        None => quote!(Ok(#node)),
    };

    let process_item = |pat: &Pattern, i_matched| {
        if !pat.multiple {
            let parse = parse_rule(&pat.rule_name, quote!(#i_matched));
            let binder = &pat.binder;
            quote!(
                let #binder = #parse?;
            )
        } else {
            let parse_node = parse_rule(&pat.rule_name, quote!(node));
            let binder = &pat.binder;
            quote!(
                let #binder = #i_matched
                    .map(|node| #parse_node)
                    .collect::<::std::result::Result<::std::vec::Vec<_>, _>>()?
                    .into_iter();
            )
        }
    };

    let parses = traverse_pattern(
        patterns.as_slice(),
        &i_nodes_iter,
        matches_pat,
        process_item,
        quote!(unreachable!()),
    );

    debug_assert!(!patterns.is_empty(), "Alternative must have at least one pattern");

    quote!(
        _ if {
            let check_condition = |slice: &[_]| -> bool {
                let #i_nodes_iter = slice.iter();
                #conditions
                #i_nodes_iter.next().is_none()
            };
            check_condition(#i_nodes.as_slice())
        } => {
            let #i_nodes_iter = #i_nodes.into_iter();
            #parses
            #body
        }
    )
}

/// Implements the match_nodes macro.
pub fn match_nodes(input: proc_macro::TokenStream) -> Result<proc_macro2::TokenStream> {
    let input: MacroInput = syn::parse(input)?;

    let i_nodes = Ident::new("___nodes", input.input_expr.span());
    let i_node_rules = Ident::new("___node_rules", Span::call_site());
    let i_node_namer = Ident::new("___node_namer", Span::call_site());

    let input_expr = &input.input_expr;
    let parser = &input.parser;

    // Generate code for each alternative in each branch
    let branches = input
        .branches
        .into_iter()
        .flat_map(|br| {
            let body = br.body;
            let i_nodes = &i_nodes;
            let i_node_namer = &i_node_namer;
            br.alternatives
                .into_iter()
                .map(move |alt| make_alternative(alt, &body, i_nodes, i_node_namer, parser))
        })
        .collect::<Vec<_>>();

    debug_assert!(!branches.is_empty(), "Must generate at least one branch");

    let node_list_ty = quote!(<_ as ::merc_pest_consume::NodeList<#parser>>);
    let node_namer_ty = quote!(<_ as ::merc_pest_consume::NodeNamer<#parser>>);
    Ok(quote!({
        let (#i_nodes, #i_node_namer) = #node_list_ty::consume(#input_expr);

        #[allow(unreachable_code, clippy::int_plus_one)]
        match () {
            #(#branches,)*
            _ => {
                // Collect the rule names to display.
                let #i_node_rules: ::std::vec::Vec<_> =
                        #i_nodes.iter().map(|n| #node_namer_ty::node_name(&#i_node_namer, n)).collect();
                return ::std::result::Result::Err(
                    #node_namer_ty::error(
                        #i_node_namer,
                        format!("Nodes didn't match any pattern: {:?}", #i_node_rules)
                    )
                );
            }
        }
    }))
}
