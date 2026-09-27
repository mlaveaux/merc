use itertools::Itertools;
use merc_pest_consume::Node;
use merc_pest_consume::match_nodes;
use merc_utilities::Span;
use pest::iterators::Pair;
use pest::iterators::Pairs;
use pest::pratt_parser::PrattParser;
use std::fmt;
use std::sync::LazyLock;

use crate::IdDecl;
use crate::Mcrl2Parser;
use crate::ParseResult;
use crate::Quantifier;
use crate::Rule;
use crate::spanned::Spanned;

use super::Assoc;
use super::DataExpr;
use super::Fixity;
use super::MultiAction;
use super::Operator;
use super::ParseNode;
use super::RuleFixity;
use super::build_pratt_parser;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ActFrmBinaryOp {
    Implies,
    Union,
    Intersect,
}

/// The kind of an [ActFrm] node, without its source span. Every recursive
/// child is an [ActFrm] (a [Spanned] wrapper), so each node carries its own
/// location.
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub enum ActFrmKind {
    #[default]
    True,
    False,
    MultAct(MultiAction),
    DataExprVal(DataExpr),
    Negation(Box<ActFrm>),
    Quantifier {
        quantifier: Quantifier,
        variables: Vec<IdDecl>,
        body: Box<ActFrm>,
    },
    Binary {
        op: ActFrmBinaryOp,
        lhs: Box<ActFrm>,
        rhs: Box<ActFrm>,
    },
    /// `expr@operand`: `expr` restricted to the instant `operand`, mirroring
    /// [`crate::ProcessExprKind::At`].
    At {
        expr: Box<ActFrm>,
        operand: DataExpr,
    },
}

/// An action formula: an [ActFrmKind] paired with the source [Span] it was
/// parsed from. Synthetic formulas built by later passes use [Span::default].
pub type ActFrm = Spanned<ActFrmKind>;

impl ActFrmKind {
    /// Wraps this kind together with a source `span` into an [ActFrm].
    pub fn spanned(self, span: Span) -> ActFrm {
        Spanned { node: self, span }
    }
}

impl From<ActFrmKind> for ActFrm {
    /// Wraps a kind into an [ActFrm] with a default (empty) span, for
    /// synthetic formulas that have no source location.
    fn from(kind: ActFrmKind) -> Self {
        Spanned {
            node: kind,
            span: Span::default(),
        }
    }
}

impl fmt::Display for ActFrm {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.node {
            ActFrmKind::False => write!(f, "false"),
            ActFrmKind::True => write!(f, "true"),
            ActFrmKind::MultAct(action) => write!(f, "{action}"),
            ActFrmKind::Binary { op, lhs, rhs } => {
                // Wrap the whole expression (not just the operands) so that a
                // surrounding tighter operator such as `!` cannot re-associate.
                write!(f, "({lhs} {op} {rhs})")
            }
            ActFrmKind::DataExprVal(expr) => write!(f, "val({expr})"),
            ActFrmKind::Quantifier {
                quantifier,
                variables,
                body,
            } => write!(f, "({} {} . {})", quantifier, variables.iter().format(", "), body),
            ActFrmKind::Negation(expr) => write!(f, "(!{expr})"),
            ActFrmKind::At { expr, operand } => write!(f, "({expr})@({operand})"),
        }
    }
}

impl fmt::Display for ActFrmBinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ActFrmBinaryOp::Implies => write!(f, "=>"),
            ActFrmBinaryOp::Intersect => write!(f, "&&"),
            ActFrmBinaryOp::Union => write!(f, "||"),
        }
    }
}

/// Precedence table for [ActFrmKind], lowest level first — see [build_pratt_parser] and
/// [Operator].
const ACTFRM_OPERATORS: &[RuleFixity] = &[
    RuleFixity {
        rule: Rule::ActFrmExists,
        fixity: Fixity::Prefix(0),
    },
    RuleFixity {
        rule: Rule::ActFrmForall,
        fixity: Fixity::Prefix(0),
    },
    RuleFixity {
        rule: Rule::ActFrmImplies,
        fixity: Fixity::Infix(1, Assoc::Right),
    },
    RuleFixity {
        rule: Rule::ActFrmUnion,
        fixity: Fixity::Infix(2, Assoc::Right),
    },
    RuleFixity {
        rule: Rule::ActFrmIntersect,
        fixity: Fixity::Infix(3, Assoc::Right),
    },
    RuleFixity {
        rule: Rule::ActFrmAt,
        fixity: Fixity::Postfix(4),
    },
    RuleFixity {
        rule: Rule::ActFrmNegation,
        fixity: Fixity::Prefix(5),
    },
];

/// Defines the operator precedence for action formulas using a Pratt parser.
static ACTFRM_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| build_pratt_parser(ACTFRM_OPERATORS));

fn actfrm_primary(primary: Pair<'_, Rule>) -> ParseResult<ActFrm> {
    let span: Span = primary.as_span().into();
    match primary.as_rule() {
        Rule::ActFrmTrue => Ok(ActFrmKind::True.spanned(span)),
        Rule::ActFrmFalse => Ok(ActFrmKind::False.spanned(span)),
        Rule::MultAct => Ok(ActFrmKind::MultAct(Mcrl2Parser::MultAct(Node::new(primary))?).spanned(span)),
        Rule::DataValExpr => Ok(ActFrmKind::DataExprVal(Mcrl2Parser::DataValExpr(Node::new(primary))?).spanned(span)),
        Rule::ActFrmBrackets => {
            // Handle parentheses by recursively parsing the inner expression
            let inner = primary
                .into_inner()
                .next()
                .expect("Expected inner expression in brackets");
            parse_actfrm(inner.into_inner())
        }
        _ => unimplemented!("Unexpected rule: {:?}", primary.as_rule()),
    }
}

fn actfrm_prefix(prefix: Pair<'_, Rule>, expr: ParseResult<ActFrm>) -> ParseResult<ActFrm> {
    let start = prefix.as_span().start();
    let expr = expr?;
    let span = Span {
        start,
        end: expr.span.end,
    };
    match prefix.as_rule() {
        Rule::ActFrmExists => Ok(ActFrmKind::Quantifier {
            quantifier: Quantifier::Exists,
            variables: Mcrl2Parser::ActFrmExists(Node::new(prefix))?,
            body: Box::new(expr),
        }
        .spanned(span)),
        Rule::ActFrmForall => Ok(ActFrmKind::Quantifier {
            quantifier: Quantifier::Forall,
            variables: Mcrl2Parser::ActFrmForall(Node::new(prefix))?,
            body: Box::new(expr),
        }
        .spanned(span)),
        Rule::ActFrmNegation => Ok(ActFrmKind::Negation(Box::new(expr)).spanned(span)),
        _ => unimplemented!("Unexpected prefix operator: {:?}", prefix.as_rule()),
    }
}

fn actfrm_infix(lhs: ParseResult<ActFrm>, op: Pair<'_, Rule>, rhs: ParseResult<ActFrm>) -> ParseResult<ActFrm> {
    let lhs = lhs?;
    let rhs = rhs?;
    let span = Span {
        start: lhs.span.start,
        end: rhs.span.end,
    };
    let op = match op.as_rule() {
        Rule::ActFrmUnion => ActFrmBinaryOp::Union,
        Rule::ActFrmIntersect => ActFrmBinaryOp::Intersect,
        Rule::ActFrmImplies => ActFrmBinaryOp::Implies,
        _ => unimplemented!("Unexpected binary operator: {:?}", op.as_rule()),
    };
    Ok(ActFrmKind::Binary {
        op,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    }
    .spanned(span))
}

/// Parses a sequence of `Rule` pairs into an `ActFrm` using a Pratt parser defined in [ACTFRM_PRATT_PARSER] for operator precedence.
///
/// # Panics
///
/// Panics if `pairs` were not produced by the `ActFrm` grammar rule.
#[allow(clippy::result_large_err)]
pub fn parse_actfrm(pairs: Pairs<Rule>) -> ParseResult<ActFrm> {
    ACTFRM_PRATT_PARSER
        .map_primary(actfrm_primary)
        .map_prefix(actfrm_prefix)
        .map_infix(actfrm_infix)
        .map_postfix(|expr, postfix| {
            let expr = expr?;
            let span = Span {
                start: expr.span.start,
                end: postfix.as_span().end(),
            };
            match postfix.as_rule() {
                Rule::ActFrmAt => Ok(ActFrmKind::At {
                    expr: Box::new(expr),
                    operand: Mcrl2Parser::ActFrmAt(Node::new(postfix))?,
                }
                .spanned(span)),
                _ => unimplemented!("Unexpected postfix rule: {:?}", postfix.as_rule()),
            }
        })
        .parse(pairs)
}

impl Operator for ActFrmKind {
    fn fixity(&self) -> Fixity {
        match self {
            ActFrmKind::Quantifier { .. } => Fixity::Prefix(0),
            ActFrmKind::Binary { op, .. } => match op {
                ActFrmBinaryOp::Implies => Fixity::Infix(1, Assoc::Right),
                ActFrmBinaryOp::Union => Fixity::Infix(2, Assoc::Right),
                ActFrmBinaryOp::Intersect => Fixity::Infix(3, Assoc::Right),
            },
            ActFrmKind::Negation(_) => Fixity::Prefix(5),
            ActFrmKind::At { .. } => Fixity::Postfix(4),
            ActFrmKind::True | ActFrmKind::False | ActFrmKind::MultAct(_) | ActFrmKind::DataExprVal(_) => {
                Fixity::Primary
            }
        }
    }

    fn operand(&self) -> Option<&ActFrm> {
        match self {
            ActFrmKind::Negation(inner) => Some(inner),
            ActFrmKind::Quantifier { body, .. } => Some(body),
            ActFrmKind::At { expr, .. } => Some(expr),
            _ => None,
        }
    }
}

#[merc_pest_consume::parser_methods]
impl Mcrl2Parser {
    pub(crate) fn ActFrmExists(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn ActFrmAt(input: ParseNode) -> ParseResult<DataExpr> {
        match_nodes!(input.into_children();
            [DataExprUnit(expr)] => {
                Ok(expr)
            },
        )
    }

    pub(crate) fn ActFrmForall(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn ActFrm(input: ParseNode) -> ParseResult<ActFrm> {
        parse_actfrm(input.children().as_pairs().clone())
    }
}
