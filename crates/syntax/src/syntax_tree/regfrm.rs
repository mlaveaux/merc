use merc_pest_consume::Node;
use merc_utilities::Span;
use pest::iterators::Pairs;
use pest::pratt_parser::PrattParser;
use std::fmt;
use std::sync::LazyLock;

use crate::Mcrl2Parser;
use crate::ParseResult;
use crate::Rule;
use crate::spanned::Spanned;

use super::ActFrm;
use super::Assoc;
use super::Fixity;
use super::Operator;
use super::ParseNode;
use super::RuleFixity;
use super::build_pratt_parser;

/// The kind of a [RegFrm] node, without its source span. Every recursive
/// child is a [RegFrm] (a [Spanned] wrapper), so each node carries its own
/// location.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub enum RegFrmKind {
    Action(ActFrm),
    Iteration(Box<RegFrm>),
    Plus(Box<RegFrm>),
    Sequence { lhs: Box<RegFrm>, rhs: Box<RegFrm> },
    Choice { lhs: Box<RegFrm>, rhs: Box<RegFrm> },
}

// `#[derive(Default)]`'s `#[default]` attribute only accepts a unit variant, and `RegFrmKind` has
// none; a manual impl is the only way to give it the cheap, non-recursive default
// `TakeRecursiveChildren`'s generated impl (see `crates/syntax/src/traverse.rs`) needs.
impl Default for RegFrmKind {
    fn default() -> Self {
        RegFrmKind::Action(ActFrm::default())
    }
}

/// A regular formula: a [RegFrmKind] paired with the source [Span] it was
/// parsed from. Synthetic formulas built by later passes use [Span::default].
pub type RegFrm = Spanned<RegFrmKind>;

impl RegFrmKind {
    /// Wraps this kind together with a source `span` into a [RegFrm].
    pub fn spanned(self, span: Span) -> RegFrm {
        Spanned { node: self, span }
    }
}

impl From<RegFrmKind> for RegFrm {
    /// Wraps a kind into a [RegFrm] with a default (empty) span, for
    /// synthetic formulas that have no source location.
    fn from(kind: RegFrmKind) -> Self {
        Spanned {
            node: kind,
            span: Span::default(),
        }
    }
}

impl fmt::Display for RegFrm {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.node {
            RegFrmKind::Action(action) => write!(f, "{action}"),
            RegFrmKind::Iteration(body) => write!(f, "({body})*"),
            RegFrmKind::Plus(body) => write!(f, "({body})+"),
            RegFrmKind::Choice { lhs, rhs } => write!(f, "({lhs} + {rhs})"),
            RegFrmKind::Sequence { lhs, rhs } => write!(f, "({lhs} . {rhs})"),
        }
    }
}

/// Precedence table for [RegFrmKind], lowest level first — see [build_pratt_parser] and
/// [Operator].
const REGFRM_OPERATORS: &[RuleFixity] = &[
    RuleFixity {
        rule: Rule::RegFrmAlternative,
        fixity: Fixity::Infix(0, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::RegFrmComposition,
        fixity: Fixity::Infix(1, Assoc::Right),
    },
    RuleFixity {
        rule: Rule::RegFrmIteration,
        fixity: Fixity::Postfix(2),
    },
    RuleFixity {
        rule: Rule::RegFrmPlus,
        fixity: Fixity::Postfix(2),
    },
];

/// Defines the operator precedence for regular expressions using a Pratt parser.
static REGFRM_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| build_pratt_parser(REGFRM_OPERATORS));

/// Parses a sequence of `Rule` pairs into an [RegFrm] using a Pratt parser defined in [REGFRM_PRATT_PARSER] for operator precedence.
///
/// # Panics
///
/// Panics if `pairs` were not produced by the `RegFrm` grammar rule.
#[allow(clippy::result_large_err)]
pub fn parse_regfrm(pairs: Pairs<Rule>) -> ParseResult<RegFrm> {
    REGFRM_PRATT_PARSER
        .map_primary(|primary| {
            let span: Span = primary.as_span().into();
            match primary.as_rule() {
                Rule::ActFrm => Ok(RegFrmKind::Action(Mcrl2Parser::ActFrm(Node::new(primary))?).spanned(span)),
                Rule::RegFrmBackets => {
                    // Handle parentheses by recursively parsing the inner expression
                    let inner = primary
                        .into_inner()
                        .next()
                        .expect("Expected inner expression in brackets");
                    parse_regfrm(inner.into_inner())
                }
                _ => unimplemented!("Unexpected rule: {:?}", primary.as_rule()),
            }
        })
        .map_infix(|lhs, op, rhs| {
            let lhs = lhs?;
            let rhs = rhs?;
            let span = Span {
                start: lhs.span.start,
                end: rhs.span.end,
            };
            match op.as_rule() {
                Rule::RegFrmAlternative => Ok(RegFrmKind::Choice {
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                }
                .spanned(span)),
                Rule::RegFrmComposition => Ok(RegFrmKind::Sequence {
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                }
                .spanned(span)),
                _ => unimplemented!("Unexpected binary operator: {:?}", op.as_rule()),
            }
        })
        .map_postfix(|expr, postfix| {
            let expr = expr?;
            let span = Span {
                start: expr.span.start,
                end: postfix.as_span().end(),
            };
            match postfix.as_rule() {
                Rule::RegFrmIteration => Ok(RegFrmKind::Iteration(Box::new(expr)).spanned(span)),
                Rule::RegFrmPlus => Ok(RegFrmKind::Plus(Box::new(expr)).spanned(span)),
                _ => unimplemented!("Unexpected rule: {:?}", postfix.as_rule()),
            }
        })
        .parse(pairs)
}

impl Operator for RegFrmKind {
    fn fixity(&self) -> Fixity {
        match self {
            RegFrmKind::Choice { .. } => Fixity::Infix(0, Assoc::Left),
            RegFrmKind::Sequence { .. } => Fixity::Infix(1, Assoc::Right),
            RegFrmKind::Iteration(_) | RegFrmKind::Plus(_) => Fixity::Postfix(2),
            RegFrmKind::Action(_) => Fixity::Primary,
        }
    }

    fn operand(&self) -> Option<&RegFrm> {
        match self {
            RegFrmKind::Iteration(inner) | RegFrmKind::Plus(inner) => Some(inner),
            _ => None,
        }
    }
}

#[merc_pest_consume::parser_methods]
impl Mcrl2Parser {
    pub(crate) fn RegFrm(input: ParseNode) -> ParseResult<RegFrm> {
        parse_regfrm(input.children().as_pairs().clone())
    }
}
