use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use pest::Parser;
use pest_derive::Parser;

use merc_aterm::ATerm;
use merc_aterm::storage::THREAD_TERM_POOL;
use merc_pest_consume::Error;
use merc_pest_consume::Node;
use merc_pest_consume::match_nodes;
use merc_utilities::MercError;

use crate::syntax::ConditionSyntax;
use crate::syntax::RewriteRuleSyntax;
use crate::syntax::RewriteSpecificationSyntax;

#[derive(Parser)]
#[grammar = "rec_grammar.pest"]
pub struct RecParser;

type ParseResult<T> = Result<T, Error<Rule>>;
type ParseNode<'i> = Node<'i, Rule, ()>;

/// Result of parsing a REC specification containing all extracted components
#[derive(Debug)]
struct RecSpecResult {
    /// Name of the specification
    _name: String,
    /// List of included files
    include_files: Vec<String>,
    /// Constructor symbols with their arities
    constructors: Vec<(String, usize)>,
    /// Variable declarations
    variables: Vec<String>,
    /// Rewrite rules
    rewrite_rules: Vec<RewriteRuleSyntax>,
    /// Terms to evaluate
    eval_terms: Vec<ATerm>,
}

/// Load a REC specification from a specified file.
pub fn load_rec_from_file(file: PathBuf) -> Result<(RewriteSpecificationSyntax, Vec<ATerm>), MercError> {
    let contents = fs::read_to_string(file.clone())?;
    parse_rec(&contents, Some(file))
}

/// Load and join multiple REC specifications
pub fn load_rec_from_strings(specs: &[&str]) -> Result<(RewriteSpecificationSyntax, Vec<ATerm>), MercError> {
    let mut rewrite_spec = RewriteSpecificationSyntax::default();
    let mut terms = vec![];

    for spec in specs {
        let (include_spec, include_terms) = parse_rec(spec, None)?;
        rewrite_spec.merge(&include_spec);
        terms.extend_from_slice(&include_terms);
    }

    Ok((rewrite_spec, terms))
}

/// Parses a REC specification. REC files can import other REC files.
/// Returns a RewriteSpec containing all the rewrite rules and a list of terms that need to be rewritten.
fn parse_rec(contents: &str, path: Option<PathBuf>) -> Result<(RewriteSpecificationSyntax, Vec<ATerm>), MercError> {
    // Initialize return result
    let mut rewrite_spec = RewriteSpecificationSyntax::default();
    let mut terms = vec![];

    // Use Pest parser (generated automatically from the grammar.pest file)
    let mut parse_result = RecParser::parse(Rule::rec_spec, contents)?;
    let root = parse_result.next().ok_or("Could not parse REC specification")?;
    let parse_node = ParseNode::new(root);

    // Parse using the consumed-based implementation
    let result = RecParser::rec_spec(parse_node)?;

    rewrite_spec.rewrite_rules = result.rewrite_rules;
    rewrite_spec.constructors = result.constructors;
    rewrite_spec.variables = result.variables;

    if !result.eval_terms.is_empty() {
        terms.extend_from_slice(&result.eval_terms);
    }

    // REC files can import other REC files. Import all referenced by the header.
    for file in result.include_files {
        if let Some(p) = &path {
            let include_path = p.parent().unwrap();
            let file_name = PathBuf::from_str(&(file.to_lowercase() + ".rec")).unwrap();
            let load_file = include_path.join(file_name);
            let contents = fs::read_to_string(load_file)?;
            let (include_spec, include_terms) = parse_rec(&contents, path.clone())?;

            // Add rewrite rules and terms to the result.
            terms.extend_from_slice(&include_terms);
            rewrite_spec
                .rewrite_rules
                .extend_from_slice(&include_spec.rewrite_rules);
            rewrite_spec.constructors.extend_from_slice(&include_spec.constructors);
            for s in include_spec.variables {
                if !rewrite_spec.variables.contains(&s) {
                    rewrite_spec.variables.push(s);
                }
            }
        }
    }

    Ok((rewrite_spec, terms))
}

#[merc_pest_consume::parser]
impl RecParser {
    /// Parse a REC specification, returns structured result with all components
    fn rec_spec(spec: ParseNode) -> ParseResult<RecSpecResult> {
        // Extract all sections of the REC file
        match_nodes!(spec.into_children();
            [header((name, include_files)), _sorts, cons(constructors), _opns, vars(variables), rules(rewrite_rules), eval(eval_terms), EOI(_)] => {
                Ok(RecSpecResult {
                    _name: name,
                    include_files,
                    constructors,
                    variables,
                    rewrite_rules,
                    eval_terms,
                })
            },
            [header((name, include_files)), _sorts, cons(constructors), _opns, vars(variables), rules(rewrite_rules), EOI(_)] => {
                Ok(RecSpecResult {
                    _name: name,
                    include_files,
                    constructors,
                    variables,
                    rewrite_rules,
                    eval_terms: Vec::new(),
                })
            }
        )
    }

    /// Extracts data from parsed header of REC spec. Returns name and include files.
    fn header(header: ParseNode) -> ParseResult<(String, Vec<String>)> {
        match_nodes!(header.into_children();
            [identifier(name), identifier(include_files)..] => {
                Ok((name, include_files.collect()))
            }
        )
    }

    /// Extracts data from parsed constructor section, derives the arity of symbols. Types are ignored.
    fn cons(cons: ParseNode) -> ParseResult<Vec<(String, usize)>> {
        let mut constructors = Vec::new();

        match_nodes!(cons.into_children();
            [cons_decl(decls)..] => {
                constructors.extend(decls);
                Ok(constructors)
            }
        )
    }

    /// Parse a constructor declaration
    fn cons_decl(decl: ParseNode) -> ParseResult<(String, usize)> {
        match_nodes!(decl.into_children();
            [identifier(symbol), identifier(params).., identifier(_)] => {
                Ok((symbol, params.len()))
            }
        )
    }

    /// Extracts data from parsed rewrite rules. Returns list of rewrite rules
    fn rules(rules: ParseNode) -> ParseResult<Vec<RewriteRuleSyntax>> {
        match_nodes!(rules.into_children();
            [rewrite_rule(rule_nodes)..] => {
                Ok(rule_nodes.collect())
            }
        )
    }

    /// Parse a rewrite rule
    fn rewrite_rule(rule: ParseNode) -> ParseResult<RewriteRuleSyntax> {
        match_nodes!(rule.into_children();
            [term(lhs), term(rhs), condition(conditions)..] => {
                Ok(RewriteRuleSyntax {
                    lhs,
                    rhs,
                    conditions: conditions.collect(),
                })
            },
            [term(lhs), term(rhs)] => {
                Ok(RewriteRuleSyntax {
                    lhs,
                    rhs,
                    conditions: vec![],
                })
            }
        )
    }

    /// Parse a single rewrite rule
    fn single_rewrite_rule(rule: ParseNode) -> ParseResult<RewriteRuleSyntax> {
        match_nodes!(rule.into_children();
            [rewrite_rule(rule), EOI(_)] => {
                Ok(rule)
            },
        )
    }

    /// Parse a condition in a rewrite rule
    fn condition(condition: ParseNode) -> ParseResult<ConditionSyntax> {
        match_nodes!(condition.into_children();
            [term(lhs), comparison(equality), term(rhs)] => {
                Ok(ConditionSyntax {
                    lhs,
                    rhs,
                    equality,
                })
            }
        )
    }

    /// Parse a comparison operator
    fn comparison(comparison: ParseNode) -> ParseResult<bool> {
        match comparison.as_str() {
            "=" => Ok(true),
            "<>" => Ok(false),
            _ => panic!("Unknown comparison operator"),
        }
    }

    /// Extracts data from the variable VARS block. Types are ignored.
    fn vars(vars: ParseNode) -> ParseResult<Vec<String>> {
        let mut variables = vec![];

        match_nodes!(vars.into_children();
            [var_decl(var_lists)..] => {
                for var_list in var_lists {
                    variables.extend(var_list);
                }
                Ok(variables)
            }
        )
    }

    /// Parse a variable declaration
    fn var_decl(var_decl: ParseNode) -> ParseResult<Vec<String>> {
        match_nodes!(var_decl.into_children();
            [identifier(vars).., identifier(_type)] => {
                // The last identifier is the type, so we exclude it
                Ok(vars.collect())
            }
        )
    }

    /// Extracts data from parsed EVAL section, returns a list of terms that need to be rewritten.
    fn eval(eval: ParseNode) -> ParseResult<Vec<ATerm>> {
        match_nodes!(eval.into_children();
            [term(terms)..] => {
                Ok(terms.collect())
            }
        )
    }

    /// Parse a term
    fn term(term: ParseNode) -> ParseResult<ATerm> {
        match_nodes!(term.into_children();
            [identifier(head_symbol), args(arguments)] => {
                THREAD_TERM_POOL.with_borrow(|tp| {
                    let symbol = tp.create_symbol(&head_symbol, arguments.len());
                    Ok(tp.create_term_iter(&symbol, arguments))
                })
            },
            [identifier(head_symbol)] => {
                THREAD_TERM_POOL.with_borrow(|tp| {
                    let symbol = tp.create_symbol(&head_symbol, 0);
                    Ok(tp.create_constant(&symbol))
                })
            }
        )
    }

    /// Parse arguments of a term
    fn args(args: ParseNode) -> ParseResult<Vec<ATerm>> {
        match_nodes!(args.into_children();
            [term(term_args)..] => {
                Ok(term_args.collect())
            }
        )
    }

    /// Parse an identifier
    fn identifier(id: ParseNode) -> ParseResult<String> {
        Ok(id.as_str().to_string())
    }

    /// Ignored rules
    fn EOI(_eof: ParseNode) -> ParseResult<()> {
        Ok(())
    }

    fn sorts(_sorts: ParseNode) -> ParseResult<()> {
        Ok(())
    }

    fn opns(_opns: ParseNode) -> ParseResult<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_parsing() {
        assert!(RecParser::parse(Rule::single_term, "f(a").is_err());
        assert!(RecParser::parse(Rule::single_term, "f()").is_err());
        assert!(RecParser::parse(Rule::single_term, "f(a,)").is_err());
        assert!(RecParser::parse(Rule::single_term, "f").is_ok());
        assert!(RecParser::parse(Rule::single_term, "f(a)").is_ok());
        assert!(RecParser::parse(Rule::single_term, "f(a,b)").is_ok());
        assert!(RecParser::parse(Rule::single_rewrite_rule, "f(a,b) = g(x)").is_ok());
        assert!(RecParser::parse(Rule::single_rewrite_rule, "f(a,b) = g(x) if x = a").is_ok());
        assert!(RecParser::parse(Rule::single_rewrite_rule, "f(a,b) = g(x) if x<> a").is_ok());
        assert!(RecParser::parse(Rule::single_rewrite_rule, "f(a,b) = g(x) if x <= a").is_err());
        assert!(RecParser::parse(Rule::single_rewrite_rule, "f(a,b) = ").is_err());
    }

    #[test]
    fn test_parsing_rewrite_rule() {
        let expected = RewriteRuleSyntax {
            lhs: ATerm::from_string("f(x,b)").unwrap(),
            rhs: ATerm::from_string("g(x)").unwrap(),
            conditions: vec![
                ConditionSyntax {
                    lhs: ATerm::from_string("x").unwrap(),
                    rhs: ATerm::from_string("a").unwrap(),
                    equality: true,
                },
                ConditionSyntax {
                    lhs: ATerm::from_string("b").unwrap(),
                    rhs: ATerm::from_string("b").unwrap(),
                    equality: true,
                },
            ],
        };

        let mut parse_result =
            RecParser::parse(Rule::single_rewrite_rule, "f(x,b) = g(x) if x = a and-if b = b").unwrap();
        let node = ParseNode::new(parse_result.next().unwrap());
        let actual = RecParser::single_rewrite_rule(node).unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn test_variable_parsing() {
        let mut parse_result = RecParser::parse(Rule::var_decl, "X Y Val Max : Nat").unwrap();
        let node = ParseNode::new(parse_result.next().unwrap());
        let result = RecParser::var_decl(node).unwrap();

        assert_eq!(result, vec!["X", "Y", "Val", "Max"]);
    }

    #[test]
    fn test_parsing_rec() {
        assert!(
            RecParser::parse(
                Rule::rec_spec,
                include_str!("../../../examples/REC/rec/missionaries.rec")
            )
            .is_ok()
        );
    }

    #[test]
    fn loading_rec() {
        let _ = parse_rec(include_str!("../../../examples/REC/rec/missionaries.rec"), None);
    }
}
