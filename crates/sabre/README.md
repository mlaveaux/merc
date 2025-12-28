# Overview

The Set Automaton Based Rewrite Engine (abbreviated Sabre) is a library that
implements rewriting on top of the (sorted) data expressions defined in the
`merc_data` crate. For the purpose of rewriting, these sorts are irrelevant, but
it is important that we can have function symbols with the same name and arity
but different sorts.

## Usage

Typically one parses the rewrite rules from a file writting the mCRL2 language,
or the Rewrite Engine Competition (REC) format, but it can also be constructed
programmatically for demonstration purposes, or for testing.

```rust
use merc_aterm::ATerm;

use merc_sabre::test_utility::create_rewrite_rule;
use merc_sabre::RewriteSpecification;
use merc_sabre::SabreRewriter;
use merc_sabre::RewriteEngine;

use merc_data::to_untyped_data_expression;

// Peano arithmetic rewrite rules
let rule_zero = create_rewrite_rule("plus(x, 0)", "x", &["x"]).unwrap();
let rule_succ = create_rewrite_rule("plus(x, S(y))", "S(plus(x, y))", &["x", "y"]).unwrap();

let spec = RewriteSpecification::new(vec![rule_zero, rule_succ]);

let mut rewriter = SabreRewriter::new(&spec);
let term = to_untyped_data_expression(ATerm::from_string("plus(S(S(0)), S(0))").unwrap(), None);

let rewritten_term = rewriter.rewrite(&term);
assert_eq!(rewritten_term.to_string(), "S(S(S(0)))");
```

This crate implements a `NaiveRewriter` for reference testing, an
`InnermostRewriter` that is strictly innermost and uses Adaptive Pattern
Matching, and the full `SabreRewriter` that uses the Set Automaton construction
for matching.

## Safety

This crate contains minimal `unsafe` code, but modules that don't use `unsafe` code
are clearly marked as such.

## Related work

The Set Automaton Based Rewrite Engine (abbreviated Sabre) was first described
in the following article:

  > "Term Rewriting Based On Set Automaton Matching". Mark Bouwman, Rick Erkens. [DOI](https://arxiv.org/abs/2202.08687).

The Set automaton construction that is used for matching is based on the following article:
  
 > "Erkens, R., Groote, J.F. (2021). A Set Automaton to Locate All Pattern Matches in a Term". In: Cerone, A., Ölveczky, P.C. (eds) Theoretical Aspects of Computing – ICTAC 2021. ICTAC 2021. Lecture Notes in Computer Science, vol 12819. Springer, Cham. [DOI](https://doi.org/10.1007/978-3-030-85315-0_5)

## Authors

The original `sabre` crate was implemented by Mark Bouwman, with theoretical contributions by Rick Erkens and Jan Friso Groote. This version has been adapted to use the `merc_aterm` crate for the term representation, by Maurice Laveaux.

## Minimum Supported Rust Version

We do not maintain an official minimum supported rust version (MSRV), and it may be upgraded at any time when necessary.

## License

All MERC crates are licensed under the BSL-1.0 license. See the [LICENSE](https://raw.githubusercontent.com/MERCorg/merc/refs/heads/main/LICENSE) file in the repository root for more information.