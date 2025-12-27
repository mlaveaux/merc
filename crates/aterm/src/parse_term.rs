#![forbid(unsafe_code)]
#![allow(clippy::result_large_err)]

use pest_derive::Parser;

use merc_pest_consume::Error;
use merc_pest_consume::match_nodes;

use crate::ATerm;
use crate::Symbol;

#[derive(Parser)]
#[grammar = "term_grammar.pest"]
pub struct TermParser;

type ParseResult<T> = std::result::Result<T, Error<Rule>>;
type ParseNode<'i> = merc_pest_consume::Node<'i, Rule, ()>;

/// Parse a term from a string.
///
/// TODO: Parse integer terms and aterm list as in the mCRL2 toolset.
///
/// Grammar:  f(t_1, ..., t_n) | c
#[merc_pest_consume::parser]
impl TermParser {
    pub fn TermSpec(spec: ParseNode) -> ParseResult<ATerm> {
        TermParser::Term(spec.children().next().unwrap())
    }

    fn Id(input: ParseNode) -> Result<String, Error<Rule>> {
        Ok(input.as_str().to_string())
    }

    fn Term(term: ParseNode) -> Result<ATerm, Error<Rule>> {
        match_nodes!(term.into_children();
            [Id(identifier)] => {
                let symbol = Symbol::new(identifier, 0);

                Ok(ATerm::constant(&symbol))
            },
            [Id(identifier), Args(args)] => {
                let symbol = Symbol::new(identifier, args.len());

                Ok(ATerm::with_iter(&symbol, args))
            }
        )
    }

    fn Args(args: ParseNode) -> Result<Vec<ATerm>, Error<Rule>> {
        match_nodes!(args.into_children();
            [Term(term)..] => {
                Ok(term.collect())
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use pest::Parser;

    use super::*;

    #[test]
    fn test_parse_term() {
        let term = "f(a, b)";

        let result = TermParser::parse(Rule::TermSpec, term).unwrap();
        print!("{}", result);
    }
}
