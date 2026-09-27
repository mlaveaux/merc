use itertools::Itertools;
use merc_pest_consume::Node;
use merc_pest_consume::match_nodes;
use merc_utilities::Span;
use pest::iterators::Pair;
use pest::iterators::Pairs;
use pest::pratt_parser::PrattParser;
use std::fmt;
use std::sync::LazyLock;

use crate::Bound;
use crate::DataExpr;
use crate::IdDecl;
use crate::Mcrl2Parser;
use crate::ParseResult;
use crate::Quantifier;
use crate::RegFrm;
use crate::Rule;
use crate::spanned::Spanned;

use super::Assoc;
use super::Fixity;
use super::Operator;
use super::ParseNode;
use super::RuleFixity;
use super::SortExpression;
use super::StateVarId;
use super::VarId;
use super::build_pratt_parser;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum StateFrmUnaryOp {
    Minus,
    Negation,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum StateFrmOp {
    Addition,
    Implies,
    Disjunction,
    Conjunction,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum FixedPointOperator {
    Least,
    Greatest,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct StateVarDecl {
    pub identifier: String,
    pub arguments: Vec<StateVarAssignment>,
    pub span: Span,
    /// Assigned during variable resolution; see [StateVarId].
    pub id: Option<StateVarId>,
}

impl StateVarDecl {
    /// Creates a new state variable declaration.
    pub fn new(identifier: String, arguments: Vec<StateVarAssignment>) -> Self {
        StateVarDecl {
            identifier,
            arguments,
            span: Span::default(),
            id: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct StateVarAssignment {
    /// The parameter's own name.
    pub identifier: Spanned<String>,
    pub sort: SortExpression,
    pub expr: DataExpr,
    /// Assigned during variable resolution; see [VarId].
    pub id: Option<VarId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum ModalityOperator {
    Diamond,
    Box,
}

/// The kind of a [StateFrm] node, without its source span. Every recursive
/// child is a [StateFrm] (a [Spanned] wrapper), so each node carries its own
/// location.
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub enum StateFrmKind {
    #[default]
    True,
    False,
    /// `delay` or `delay@t`; the optional time is `None` for a bare `delay`.
    Delay(Option<DataExpr>),
    /// `yaled` or `yaled@t`; the optional time is `None` for a bare `yaled`.
    Yaled(Option<DataExpr>),
    Id(String, Vec<DataExpr>),
    /// A fixpoint-variable reference resolved to its declaring `mu`/`nu`'s own [StateVarId].
    Resolved(String, Vec<DataExpr>, StateVarId),
    DataValExprLeftMult(DataExpr, Box<StateFrm>),
    DataValExprRightMult(Box<StateFrm>, DataExpr),
    DataValExpr(DataExpr),
    Modality {
        operator: ModalityOperator,
        formula: RegFrm,
        expr: Box<StateFrm>,
    },
    Unary {
        op: StateFrmUnaryOp,
        expr: Box<StateFrm>,
    },
    Binary {
        op: StateFrmOp,
        lhs: Box<StateFrm>,
        rhs: Box<StateFrm>,
    },
    Quantifier {
        quantifier: Quantifier,
        variables: Vec<IdDecl>,
        body: Box<StateFrm>,
    },
    Bound {
        bound: Bound,
        variables: Vec<IdDecl>,
        body: Box<StateFrm>,
    },
    FixedPoint {
        operator: FixedPointOperator,
        variable: StateVarDecl,
        body: Box<StateFrm>,
    },
}

/// A state formula: a [StateFrmKind] paired with the source [Span] it was
/// parsed from. Synthetic formulas built by later passes use [Span::default].
pub type StateFrm = Spanned<StateFrmKind>;

impl StateFrmKind {
    /// Wraps this kind together with a source `span` into a [StateFrm].
    pub fn spanned(self, span: Span) -> StateFrm {
        Spanned { node: self, span }
    }
}

impl From<StateFrmKind> for StateFrm {
    /// Wraps a kind into a [StateFrm] with a default (empty) span, for
    /// synthetic formulas that have no source location.
    fn from(kind: StateFrmKind) -> Self {
        Spanned {
            node: kind,
            span: Span::default(),
        }
    }
}

impl fmt::Display for StateFrmUnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            StateFrmUnaryOp::Minus => write!(f, "-"),
            StateFrmUnaryOp::Negation => write!(f, "!"),
        }
    }
}

impl fmt::Display for FixedPointOperator {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FixedPointOperator::Greatest => write!(f, "nu"),
            FixedPointOperator::Least => write!(f, "mu"),
        }
    }
}

impl fmt::Display for StateFrm {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.node {
            StateFrmKind::True => write!(f, "true"),
            StateFrmKind::False => write!(f, "false"),
            StateFrmKind::DataValExpr(expr) => write!(f, "val({expr})"),
            StateFrmKind::Id(identifier, args) | StateFrmKind::Resolved(identifier, args, _) => {
                if args.is_empty() {
                    write!(f, "{identifier}")
                } else {
                    write!(f, "{}({})", identifier, args.iter().format(", "))
                }
            }
            StateFrmKind::Unary { op, expr } => write!(f, "({op} {expr})"),
            StateFrmKind::Modality {
                operator,
                formula,
                expr,
            } => match operator {
                ModalityOperator::Box => write!(f, "[{formula}]{expr}"),
                ModalityOperator::Diamond => write!(f, "<{formula}>{expr}"),
            },
            StateFrmKind::Quantifier {
                quantifier,
                variables,
                body,
            } => {
                write!(f, "({} {} . {})", quantifier, variables.iter().format(", "), body)
            }
            StateFrmKind::Bound {
                bound: quantifier,
                variables,
                body,
            } => {
                write!(f, "({} {} . {})", quantifier, variables.iter().format(", "), body)
            }
            StateFrmKind::Binary { op, lhs, rhs } => {
                write!(f, "({lhs} {op} {rhs})")
            }
            StateFrmKind::FixedPoint {
                operator,
                variable,
                body,
            } => {
                write!(f, "({operator} {variable} . {body})")
            }
            StateFrmKind::Delay(Some(expr)) => write!(f, "delay@({expr})"),
            StateFrmKind::Delay(None) => write!(f, "delay"),
            StateFrmKind::Yaled(Some(expr)) => write!(f, "yaled@({expr})"),
            StateFrmKind::Yaled(None) => write!(f, "yaled"),
            StateFrmKind::DataValExprLeftMult(value, expr) => write!(f, "(val({value}) * {expr})"),
            StateFrmKind::DataValExprRightMult(expr, value) => write!(f, "({expr} * val({value}))"),
        }
    }
}

impl fmt::Display for StateVarDecl {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.arguments.is_empty() {
            write!(f, "{}", self.identifier)
        } else {
            write!(f, "{}({})", self.identifier, self.arguments.iter().format(","))
        }
    }
}

impl fmt::Display for StateVarAssignment {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} : {} = {}", self.identifier.node, self.sort, self.expr)
    }
}

impl fmt::Display for StateFrmOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            StateFrmOp::Implies => write!(f, "=>"),
            StateFrmOp::Conjunction => write!(f, "&&"),
            StateFrmOp::Disjunction => write!(f, "||"),
            StateFrmOp::Addition => write!(f, "+"),
        }
    }
}

/// Precedence table for [StateFrmKind], lowest level first — see [build_pratt_parser] and
/// [Operator].
const STATEFRM_OPERATORS: &[RuleFixity] = &[
    RuleFixity {
        rule: Rule::StateFrmMu,
        fixity: Fixity::Prefix(0),
    },
    RuleFixity {
        rule: Rule::StateFrmNu,
        fixity: Fixity::Prefix(0),
    },
    RuleFixity {
        rule: Rule::StateFrmForall,
        fixity: Fixity::Prefix(1),
    },
    RuleFixity {
        rule: Rule::StateFrmExists,
        fixity: Fixity::Prefix(1),
    },
    RuleFixity {
        rule: Rule::StateFrmInf,
        fixity: Fixity::Prefix(1),
    },
    RuleFixity {
        rule: Rule::StateFrmSup,
        fixity: Fixity::Prefix(1),
    },
    RuleFixity {
        rule: Rule::StateFrmSum,
        fixity: Fixity::Prefix(1),
    },
    RuleFixity {
        rule: Rule::StateFrmAddition,
        fixity: Fixity::Infix(2, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::StateFrmImplication,
        fixity: Fixity::Infix(3, Assoc::Right),
    },
    RuleFixity {
        rule: Rule::StateFrmDisjunction,
        fixity: Fixity::Infix(4, Assoc::Right),
    },
    RuleFixity {
        rule: Rule::StateFrmConjunction,
        fixity: Fixity::Infix(5, Assoc::Right),
    },
    RuleFixity {
        rule: Rule::StateFrmLeftConstantMultiply,
        fixity: Fixity::Prefix(6),
    },
    RuleFixity {
        rule: Rule::StateFrmRightConstantMultiply,
        fixity: Fixity::Postfix(6),
    },
    RuleFixity {
        rule: Rule::StateFrmBox,
        fixity: Fixity::Prefix(7),
    },
    RuleFixity {
        rule: Rule::StateFrmDiamond,
        fixity: Fixity::Prefix(7),
    },
    RuleFixity {
        rule: Rule::StateFrmNegation,
        fixity: Fixity::Prefix(8),
    },
    RuleFixity {
        rule: Rule::StateFrmUnaryMinus,
        fixity: Fixity::Prefix(8),
    },
];

/// Defines the operator precedence for state formulas using a Pratt parser.
static STATEFRM_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| build_pratt_parser(STATEFRM_OPERATORS));

#[allow(clippy::result_large_err)]
pub fn parse_statefrm(pairs: Pairs<Rule>) -> ParseResult<StateFrm> {
    STATEFRM_PRATT_PARSER
        .map_primary(statefrm_primary)
        .map_prefix(statefrm_prefix)
        .map_infix(statefrm_infix)
        .map_postfix(statefrm_postfix)
        .parse(pairs)
}

fn statefrm_primary(primary: Pair<'_, Rule>) -> ParseResult<StateFrm> {
    let span: Span = primary.as_span().into();
    match primary.as_rule() {
        Rule::StateFrmId => Mcrl2Parser::StateFrmId(Node::new(primary)),
        Rule::StateFrmTrue => Ok(StateFrmKind::True.spanned(span)),
        Rule::StateFrmFalse => Ok(StateFrmKind::False.spanned(span)),
        Rule::StateFrmDelay => Mcrl2Parser::StateFrmDelay(Node::new(primary)),
        Rule::StateFrmYaled => Mcrl2Parser::StateFrmYaled(Node::new(primary)),
        Rule::StateFrmNegation => Mcrl2Parser::StateFrmNegation(Node::new(primary)),
        Rule::StateFrmDataValExpr => {
            // `StateFrmDataValExpr` only wraps a `DataValExpr` child; unwrap before
            // consuming it, the same way `StateFrmBrackets` unwraps its own child below.
            let inner = primary
                .into_inner()
                .next()
                .expect("StateFrmDataValExpr always wraps a DataValExpr child");
            Ok(StateFrmKind::DataValExpr(Mcrl2Parser::DataValExpr(Node::new(inner))?).spanned(span))
        }
        Rule::StateFrmBrackets => {
            // Handle parentheses by recursively parsing the inner expression
            let inner = primary
                .into_inner()
                .next()
                .expect("Expected inner expression in brackets");
            parse_statefrm(inner.into_inner())
        }
        _ => unimplemented!("Unexpected rule: {:?}", primary.as_rule()),
    }
}

fn statefrm_prefix(prefix: Pair<'_, Rule>, expr: ParseResult<StateFrm>) -> ParseResult<StateFrm> {
    let start = prefix.as_span().start();
    let expr = expr?;
    let span = Span {
        start,
        end: expr.span.end,
    };
    match prefix.as_rule() {
        Rule::StateFrmLeftConstantMultiply => Ok(StateFrmKind::DataValExprLeftMult(
            Mcrl2Parser::StateFrmLeftConstantMultiply(Node::new(prefix))?,
            Box::new(expr),
        )
        .spanned(span)),
        Rule::StateFrmDiamond => Ok(StateFrmKind::Modality {
            operator: ModalityOperator::Diamond,
            formula: Mcrl2Parser::StateFrmDiamond(Node::new(prefix))?,
            expr: Box::new(expr),
        }
        .spanned(span)),
        Rule::StateFrmBox => Ok(StateFrmKind::Modality {
            operator: ModalityOperator::Box,
            formula: Mcrl2Parser::StateFrmBox(Node::new(prefix))?,
            expr: Box::new(expr),
        }
        .spanned(span)),
        Rule::StateFrmExists => Ok(StateFrmKind::Quantifier {
            quantifier: Quantifier::Exists,
            variables: Mcrl2Parser::StateFrmExists(Node::new(prefix))?,
            body: Box::new(expr),
        }
        .spanned(span)),
        Rule::StateFrmForall => Ok(StateFrmKind::Quantifier {
            quantifier: Quantifier::Forall,
            variables: Mcrl2Parser::StateFrmForall(Node::new(prefix))?,
            body: Box::new(expr),
        }
        .spanned(span)),
        Rule::StateFrmMu => Ok(StateFrmKind::FixedPoint {
            operator: FixedPointOperator::Least,
            variable: Mcrl2Parser::StateFrmMu(Node::new(prefix))?,
            body: Box::new(expr),
        }
        .spanned(span)),
        Rule::StateFrmNu => Ok(StateFrmKind::FixedPoint {
            operator: FixedPointOperator::Greatest,
            variable: Mcrl2Parser::StateFrmNu(Node::new(prefix))?,
            body: Box::new(expr),
        }
        .spanned(span)),
        Rule::StateFrmNegation => Ok(StateFrmKind::Unary {
            op: StateFrmUnaryOp::Negation,
            expr: Box::new(expr),
        }
        .spanned(span)),
        Rule::StateFrmSup => Ok(StateFrmKind::Bound {
            bound: Bound::Sup,
            variables: Mcrl2Parser::StateFrmSup(Node::new(prefix))?,
            body: Box::new(expr),
        }
        .spanned(span)),
        Rule::StateFrmSum => Ok(StateFrmKind::Bound {
            bound: Bound::Sum,
            variables: Mcrl2Parser::StateFrmSum(Node::new(prefix))?,
            body: Box::new(expr),
        }
        .spanned(span)),
        Rule::StateFrmInf => Ok(StateFrmKind::Bound {
            bound: Bound::Inf,
            variables: Mcrl2Parser::StateFrmInf(Node::new(prefix))?,
            body: Box::new(expr),
        }
        .spanned(span)),
        _ => unimplemented!("Unexpected prefix operator: {:?}", prefix.as_rule()),
    }
}

fn statefrm_infix(lhs: ParseResult<StateFrm>, op: Pair<'_, Rule>, rhs: ParseResult<StateFrm>) -> ParseResult<StateFrm> {
    let lhs = lhs?;
    let rhs = rhs?;
    let span = Span {
        start: lhs.span.start,
        end: rhs.span.end,
    };
    let op = match op.as_rule() {
        Rule::StateFrmAddition => StateFrmOp::Addition,
        Rule::StateFrmImplication => StateFrmOp::Implies,
        Rule::StateFrmDisjunction => StateFrmOp::Disjunction,
        Rule::StateFrmConjunction => StateFrmOp::Conjunction,
        _ => unimplemented!("Unexpected binary operator: {:?}", op.as_rule()),
    };
    Ok(StateFrmKind::Binary {
        op,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    }
    .spanned(span))
}

fn statefrm_postfix(expr: ParseResult<StateFrm>, postfix: Pair<'_, Rule>) -> ParseResult<StateFrm> {
    let expr = expr?;
    let span = Span {
        start: expr.span.start,
        end: postfix.as_span().end(),
    };
    match postfix.as_rule() {
        Rule::StateFrmRightConstantMultiply => Ok(StateFrmKind::DataValExprRightMult(
            Box::new(expr),
            Mcrl2Parser::StateFrmRightConstantMultiply(Node::new(postfix))?,
        )
        .spanned(span)),
        _ => unimplemented!("Unexpected binary operator: {:?}", postfix.as_rule()),
    }
}

impl Operator for StateFrmKind {
    fn fixity(&self) -> Fixity {
        match self {
            StateFrmKind::FixedPoint { .. } => Fixity::Prefix(0),
            StateFrmKind::Quantifier { .. } | StateFrmKind::Bound { .. } => Fixity::Prefix(1),
            StateFrmKind::Binary { op, .. } => match op {
                StateFrmOp::Addition => Fixity::Infix(2, Assoc::Left),
                StateFrmOp::Implies => Fixity::Infix(3, Assoc::Right),
                StateFrmOp::Disjunction => Fixity::Infix(4, Assoc::Right),
                StateFrmOp::Conjunction => Fixity::Infix(5, Assoc::Right),
            },
            StateFrmKind::DataValExprLeftMult(_, _) => Fixity::Prefix(6),
            StateFrmKind::DataValExprRightMult(_, _) => Fixity::Postfix(6),
            StateFrmKind::Modality { .. } => Fixity::Prefix(7),
            StateFrmKind::Unary { .. } => Fixity::Prefix(8),
            StateFrmKind::True
            | StateFrmKind::False
            | StateFrmKind::Delay(_)
            | StateFrmKind::Yaled(_)
            | StateFrmKind::Id(_, _)
            | StateFrmKind::Resolved(_, _, _)
            | StateFrmKind::DataValExpr(_) => Fixity::Primary,
        }
    }

    fn operand(&self) -> Option<&StateFrm> {
        match self {
            StateFrmKind::DataValExprLeftMult(_, expr) => Some(expr),
            StateFrmKind::DataValExprRightMult(expr, _) => Some(expr),
            StateFrmKind::Modality { expr, .. }
            | StateFrmKind::Unary { expr, .. }
            | StateFrmKind::Quantifier { body: expr, .. }
            | StateFrmKind::Bound { body: expr, .. }
            | StateFrmKind::FixedPoint { body: expr, .. } => Some(expr),
            _ => None,
        }
    }
}

#[merc_pest_consume::parser_methods]
impl Mcrl2Parser {
    pub(crate) fn StateFrmId(id: ParseNode) -> ParseResult<StateFrm> {
        let span: Span = id.as_span().into();
        match_nodes!(id.into_children();
            [Id(identifier)] => {
                Ok(StateFrmKind::Id(identifier.into_node(), Vec::new()).spanned(span))
            },
            [Id(identifier), DataExprList(expressions)] => {
                Ok(StateFrmKind::Id(identifier.into_node(), expressions).spanned(span))
            },
        )
    }

    pub(crate) fn StateFrmExists(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn StateFrmForall(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn StateFrmMu(input: ParseNode) -> ParseResult<StateVarDecl> {
        match_nodes!(input.into_children();
            [StateVarDecl(variable)] => {
                Ok(variable)
            },
        )
    }

    pub(crate) fn StateFrmNu(input: ParseNode) -> ParseResult<StateVarDecl> {
        match_nodes!(input.into_children();
            [StateVarDecl(variable)] => {
                Ok(variable)
            },
        )
    }

    pub(crate) fn StateFrmDelay(input: ParseNode) -> ParseResult<StateFrm> {
        let span: Span = input.as_span().into();
        // The `@`-time argument is optional, so there may be zero or one child.
        match input.into_children().next() {
            Some(child) => Ok(StateFrmKind::Delay(Some(Mcrl2Parser::DataExpr(child)?)).spanned(span)),
            None => Ok(StateFrmKind::Delay(None).spanned(span)),
        }
    }

    pub(crate) fn StateFrmYaled(input: ParseNode) -> ParseResult<StateFrm> {
        let span: Span = input.as_span().into();
        // The `@`-time argument is optional, so there may be zero or one child.
        match input.into_children().next() {
            Some(child) => Ok(StateFrmKind::Yaled(Some(Mcrl2Parser::DataExpr(child)?)).spanned(span)),
            None => Ok(StateFrmKind::Yaled(None).spanned(span)),
        }
    }

    pub(crate) fn StateFrmNegation(input: ParseNode) -> ParseResult<StateFrm> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [StateFrm(state)] => {
                Ok(StateFrmKind::Unary { op: crate::StateFrmUnaryOp::Negation, expr: Box::new(state) }.spanned(span))
            },
        )
    }

    pub(crate) fn StateFrmLeftConstantMultiply(input: ParseNode) -> ParseResult<DataExpr> {
        match_nodes!(input.into_children();
            [DataValExpr(expr)] => {
                Ok(expr)
            },
        )
    }

    pub(crate) fn StateFrmRightConstantMultiply(input: ParseNode) -> ParseResult<DataExpr> {
        match_nodes!(input.into_children();
            [DataValExpr(expr)] => {
                Ok(expr)
            },
        )
    }

    pub(crate) fn StateFrmDiamond(input: ParseNode) -> ParseResult<RegFrm> {
        match_nodes!(input.into_children();
            [RegFrm(formula)] => {
                Ok(formula)
            },
        )
    }

    pub(crate) fn StateFrmBox(input: ParseNode) -> ParseResult<RegFrm> {
        match_nodes!(input.into_children();
            [RegFrm(formula)] => {
                Ok(formula)
            },
        )
    }

    pub(crate) fn StateFrmSup(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn StateFrmInf(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn StateFrmSum(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn StateFrm(input: ParseNode) -> ParseResult<StateFrm> {
        parse_statefrm(input.children().as_pairs().clone())
    }

    fn StateVarDecl(input: ParseNode) -> ParseResult<StateVarDecl> {
        let span = input.as_span();
        match_nodes!(input.into_children();
            [Id(identifier), StateVarAssignmentList(arguments)] => {
                Ok(StateVarDecl {
                    identifier: identifier.into_node(),
                    arguments,
                    span: span.into(),
                    id: None,
                })
            },
            [Id(identifier)] => {
                Ok(StateVarDecl {
                    identifier: identifier.into_node(),
                    arguments: Vec::new(),
                    span: span.into(),
                    id: None,
                })
            }
        )
    }

    fn StateVarAssignmentList(input: ParseNode) -> ParseResult<Vec<StateVarAssignment>> {
        match_nodes!(input.into_children();
            [StateVarAssignment(assignments)..] => {
                Ok(assignments.collect())
            }
        )
    }

    fn StateVarAssignment(input: ParseNode) -> ParseResult<StateVarAssignment> {
        match_nodes!(input.into_children();
            [Id(identifier), SortExpr(sort), DataExpr(expr)] => {
                Ok(StateVarAssignment { identifier, sort, expr, id: None })
            }
        )
    }
}
