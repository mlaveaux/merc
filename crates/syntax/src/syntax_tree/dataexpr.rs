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
use super::Fixity;
use super::Operator;
use super::ParseNode;
use super::RuleFixity;
use super::VarId;
use super::build_pratt_parser;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum DataExprUnaryOp {
    Negation,
    Minus,
    Size,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum DataExprBinaryOp {
    Conj,
    Disj,
    Implies,
    Equal,
    NotEqual,
    LessThan,
    LessEqual,
    GreaterThan,
    GreaterEqual,
    Cons,
    Snoc,
    In,
    Concat,
    Add,
    Subtract,
    Div,
    IntDiv,
    Mod,
    Multiply,
    At,
}

/// The kind of a [DataExpr] node.
#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum DataExprKind {
    Id(String),
    /// A variable reference paired with its declaring binder's own [VarId]: not this
    /// occurrence's identity.
    Resolved(String, VarId),
    Number(String), // Is string because the number can be any size.
    Bool(bool),
    Application {
        function: Box<DataExpr>,
        arguments: Vec<DataExpr>,
    },
    #[default]
    EmptyList,
    List(Vec<DataExpr>),
    EmptySet,
    Set(Vec<DataExpr>),
    EmptyBag,
    Bag(Vec<BagElement>),
    SetBagComp {
        variable: IdDecl,
        predicate: Box<DataExpr>,
    },
    Lambda {
        variables: Vec<IdDecl>,
        body: Box<DataExpr>,
    },
    Quantifier {
        op: Quantifier,
        variables: Vec<IdDecl>,
        body: Box<DataExpr>,
    },
    Unary {
        op: DataExprUnaryOp,
        expr: Box<DataExpr>,
    },
    Binary {
        op: DataExprBinaryOp,
        lhs: Box<DataExpr>,
        rhs: Box<DataExpr>,
    },
    FunctionUpdate {
        expr: Box<DataExpr>,
        update: Box<DataExprUpdate>,
    },
    Whr {
        expr: Box<DataExpr>,
        assignments: Vec<Assignment>,
    },
}

/// A data expression paired with the source [Span] it was
/// parsed from.
pub type DataExpr = Spanned<DataExprKind>;

impl DataExprKind {
    /// Wraps this kind together with a source `span`.
    pub fn spanned(self, span: Span) -> DataExpr {
        Spanned { node: self, span }
    }
}

impl From<DataExprKind> for DataExpr {
    /// For synthetic expressions that have no source location.
    fn from(kind: DataExprKind) -> Self {
        Spanned {
            node: kind,
            span: Span::default(),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct BagElement {
    pub expr: DataExpr,
    pub multiplicity: DataExpr,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct DataExprUpdate {
    pub expr: DataExpr,
    pub update: DataExpr,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct AssignmentData {
    pub identifier: String,
    pub expr: DataExpr,
    /// Assigned during variable resolution when this assignment is a `whr` binding (a new
    /// variable, in scope for the body).
    pub id: Option<VarId>,
}

// Not itself self-recursive (its `expr: DataExpr` field is a different type, whose own `Spanned`
// wrapper already detaches its children on drop), so the default no-op is correct.
impl crate::TakeRecursiveChildren for AssignmentData {}

/// A process-instantiation assignment (`x = e`, as in `P(x = 1)`), paired with the source [Span]
/// it was parsed from. Equality/ordering/hashing ignore the span, per [Spanned]'s documented
/// convention.
pub type Assignment = Spanned<AssignmentData>;

impl AssignmentData {
    /// Wraps this data together with a source `span`.
    pub fn spanned(self, span: Span) -> Assignment {
        Spanned { node: self, span }
    }
}

impl Assignment {
    /// Creates a new assignment with the given identifier and expression, with a default (empty)
    /// span.
    pub fn new(identifier: String, expr: DataExpr) -> Self {
        AssignmentData {
            identifier,
            expr,
            id: None,
        }
        .spanned(Span::default())
    }
}

impl fmt::Display for Assignment {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} = {}", self.identifier, self.expr)
    }
}

impl fmt::Display for DataExprUnaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataExprUnaryOp::Negation => write!(f, "!"),
            DataExprUnaryOp::Minus => write!(f, "-"),
            DataExprUnaryOp::Size => write!(f, "#"),
        }
    }
}

impl fmt::Display for DataExpr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.node {
            DataExprKind::EmptyList => write!(f, "[]"),
            DataExprKind::EmptyBag => write!(f, "{{:}}"),
            DataExprKind::EmptySet => write!(f, "{{}}"),
            DataExprKind::List(expressions) => write!(f, "[{}]", expressions.iter().format(", ")),
            DataExprKind::Bag(expressions) => write!(
                f,
                "{{ {} }}",
                expressions
                    .iter()
                    .format_with(", ", |e, f| f(&format_args!("{}: {}", e.expr, e.multiplicity)))
            ),
            DataExprKind::Set(expressions) => write!(f, "{{ {} }}", expressions.iter().format(", ")),
            DataExprKind::Id(identifier) | DataExprKind::Resolved(identifier, _) => write!(f, "{identifier}"),
            DataExprKind::Binary { op, lhs, rhs } => write!(f, "({lhs} {op} {rhs})"),
            DataExprKind::Unary { op, expr } => write!(f, "({op} {expr})"),
            DataExprKind::Bool(value) => write!(f, "{value}"),
            DataExprKind::Quantifier { op, variables, body } => {
                write!(f, "({} {} . {})", op, variables.iter().format(", "), body)
            }
            DataExprKind::Lambda { variables, body } => {
                write!(f, "(lambda {} . {})", variables.iter().format(", "), body)
            }
            DataExprKind::Application { function, arguments } => {
                if arguments.is_empty() {
                    write!(f, "{function}")
                } else {
                    write!(f, "{}({})", function, arguments.iter().format(", "))
                }
            }
            DataExprKind::Number(value) => write!(f, "{value}"),
            DataExprKind::FunctionUpdate { expr, update } => write!(f, "{expr}[{update}]"),
            DataExprKind::SetBagComp { variable, predicate } => write!(f, "{{ {variable} | {predicate} }}"),
            DataExprKind::Whr { expr, assignments } => {
                write!(f, "{} whr {} end", expr, assignments.iter().format(", "))
            }
        }
    }
}

impl fmt::Display for DataExprUpdate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} -> {}", self.expr, self.update)
    }
}

impl fmt::Display for DataExprBinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DataExprBinaryOp::At => write!(f, "."),
            DataExprBinaryOp::Concat => write!(f, "++"),
            DataExprBinaryOp::Cons => write!(f, "|>"),
            DataExprBinaryOp::Equal => write!(f, "=="),
            DataExprBinaryOp::NotEqual => write!(f, "!="),
            DataExprBinaryOp::LessThan => write!(f, "<"),
            DataExprBinaryOp::LessEqual => write!(f, "<="),
            DataExprBinaryOp::GreaterThan => write!(f, ">"),
            DataExprBinaryOp::GreaterEqual => write!(f, ">="),
            DataExprBinaryOp::Conj => write!(f, "&&"),
            DataExprBinaryOp::Disj => write!(f, "||"),
            DataExprBinaryOp::Add => write!(f, "+"),
            DataExprBinaryOp::Subtract => write!(f, "-"),
            DataExprBinaryOp::Div => write!(f, "/"),
            DataExprBinaryOp::Implies => write!(f, "=>"),
            DataExprBinaryOp::In => write!(f, "in"),
            DataExprBinaryOp::IntDiv => write!(f, "div"),
            DataExprBinaryOp::Mod => write!(f, "mod"),
            DataExprBinaryOp::Multiply => write!(f, "*"),
            DataExprBinaryOp::Snoc => write!(f, "<|"),
        }
    }
}

/// Precedence table for [DataExprKind], lowest level first — see [build_pratt_parser] and
/// [Operator].
const DATAEXPR_OPERATORS: &[RuleFixity] = &[
    RuleFixity {
        rule: Rule::DataExprWhr,
        fixity: Fixity::Postfix(0),
    },
    RuleFixity {
        rule: Rule::DataExprForall,
        fixity: Fixity::Prefix(1),
    },
    RuleFixity {
        rule: Rule::DataExprExists,
        fixity: Fixity::Prefix(1),
    },
    RuleFixity {
        rule: Rule::DataExprLambda,
        fixity: Fixity::Prefix(1),
    },
    RuleFixity {
        rule: Rule::DataExprImpl,
        fixity: Fixity::Infix(2, Assoc::Right),
    },
    RuleFixity {
        rule: Rule::DataExprDisj,
        fixity: Fixity::Infix(3, Assoc::Right),
    },
    RuleFixity {
        rule: Rule::DataExprConj,
        fixity: Fixity::Infix(4, Assoc::Right),
    },
    RuleFixity {
        rule: Rule::DataExprEq,
        fixity: Fixity::Infix(5, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprNeq,
        fixity: Fixity::Infix(5, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprLess,
        fixity: Fixity::Infix(6, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprLeq,
        fixity: Fixity::Infix(6, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprGeq,
        fixity: Fixity::Infix(6, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprGreater,
        fixity: Fixity::Infix(6, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprIn,
        fixity: Fixity::Infix(6, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprCons,
        fixity: Fixity::Infix(7, Assoc::Right),
    },
    RuleFixity {
        rule: Rule::DataExprSnoc,
        fixity: Fixity::Infix(8, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprConcat,
        fixity: Fixity::Infix(9, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprAdd,
        fixity: Fixity::Infix(10, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprSubtract,
        fixity: Fixity::Infix(10, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprDiv,
        fixity: Fixity::Infix(11, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprIntDiv,
        fixity: Fixity::Infix(11, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprMod,
        fixity: Fixity::Infix(11, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprMult,
        fixity: Fixity::Infix(12, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprAt,
        fixity: Fixity::Infix(12, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::DataExprMinus,
        fixity: Fixity::Prefix(12),
    },
    RuleFixity {
        rule: Rule::DataExprNegation,
        fixity: Fixity::Prefix(12),
    },
    RuleFixity {
        rule: Rule::DataExprSize,
        fixity: Fixity::Prefix(12),
    },
    RuleFixity {
        rule: Rule::DataExprUpdate,
        fixity: Fixity::Postfix(13),
    },
    RuleFixity {
        rule: Rule::DataExprApplication,
        fixity: Fixity::Postfix(13),
    },
];

static DATAEXPR_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| build_pratt_parser(DATAEXPR_OPERATORS));

#[allow(clippy::result_large_err)]
pub fn parse_dataexpr(pairs: Pairs<Rule>) -> ParseResult<DataExpr> {
    DATAEXPR_PRATT_PARSER
        .map_primary(dataexpr_primary)
        .map_infix(dataexpr_infix)
        .map_postfix(dataexpr_postfix)
        .map_prefix(dataexpr_prefix)
        .parse(pairs)
}

fn dataexpr_primary(primary: Pair<'_, Rule>) -> ParseResult<DataExpr> {
    let span: Span = primary.as_span().into();
    match primary.as_rule() {
        Rule::DataExprTrue => Ok(DataExprKind::Bool(true).spanned(span)),
        Rule::DataExprFalse => Ok(DataExprKind::Bool(false).spanned(span)),
        Rule::DataExprEmptyList => Ok(DataExprKind::EmptyList.spanned(span)),
        Rule::DataExprEmptySet => Ok(DataExprKind::EmptySet.spanned(span)),
        Rule::DataExprEmptyBag => Ok(DataExprKind::EmptyBag.spanned(span)),
        Rule::DataExprListEnum => Mcrl2Parser::DataExprListEnum(Node::new(primary)),
        Rule::DataExprBagEnum => Mcrl2Parser::DataExprBagEnum(Node::new(primary)),
        Rule::DataExprSetBagComp => Mcrl2Parser::DataExprSetBagComp(Node::new(primary)),
        Rule::DataExprSetEnum => Mcrl2Parser::DataExprSetEnum(Node::new(primary)),
        Rule::Number => Mcrl2Parser::Number(Node::new(primary)),
        Rule::IdAt => Ok(DataExprKind::Id(Mcrl2Parser::IdAt(Node::new(primary))?.into_node()).spanned(span)),

        Rule::DataExprBrackets => {
            // Handle parentheses by recursively parsing the inner expression
            let inner = primary
                .into_inner()
                .next()
                .expect("Expected inner expression in brackets");
            parse_dataexpr(inner.into_inner())
        }

        _ => unimplemented!("Unexpected rule: {:?}", primary.as_rule()),
    }
}

fn dataexpr_infix(lhs: ParseResult<DataExpr>, op: Pair<'_, Rule>, rhs: ParseResult<DataExpr>) -> ParseResult<DataExpr> {
    let op_kind = match op.as_rule() {
        Rule::DataExprConj => DataExprBinaryOp::Conj,
        Rule::DataExprDisj => DataExprBinaryOp::Disj,
        Rule::DataExprEq => DataExprBinaryOp::Equal,
        Rule::DataExprNeq => DataExprBinaryOp::NotEqual,
        Rule::DataExprLess => DataExprBinaryOp::LessThan,
        Rule::DataExprLeq => DataExprBinaryOp::LessEqual,
        Rule::DataExprGreater => DataExprBinaryOp::GreaterThan,
        Rule::DataExprGeq => DataExprBinaryOp::GreaterEqual,
        Rule::DataExprIn => DataExprBinaryOp::In,
        Rule::DataExprCons => DataExprBinaryOp::Cons,
        Rule::DataExprSnoc => DataExprBinaryOp::Snoc,
        Rule::DataExprConcat => DataExprBinaryOp::Concat,
        Rule::DataExprAdd => DataExprBinaryOp::Add,
        Rule::DataExprSubtract => DataExprBinaryOp::Subtract,
        Rule::DataExprDiv => DataExprBinaryOp::Div,
        Rule::DataExprIntDiv => DataExprBinaryOp::IntDiv,
        Rule::DataExprMod => DataExprBinaryOp::Mod,
        Rule::DataExprMult => DataExprBinaryOp::Multiply,
        Rule::DataExprAt => DataExprBinaryOp::At,
        Rule::DataExprImpl => DataExprBinaryOp::Implies,
        _ => unimplemented!("Unexpected binary operator rule: {:?}", op.as_rule()),
    };

    let lhs = lhs?;
    let rhs = rhs?;
    let span = Span {
        start: lhs.span.start,
        end: rhs.span.end,
    };
    Ok(DataExprKind::Binary {
        op: op_kind,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    }
    .spanned(span))
}

fn dataexpr_postfix(expr: ParseResult<DataExpr>, postfix: Pair<'_, Rule>) -> ParseResult<DataExpr> {
    let expr = expr?;
    let end = postfix.as_span().end();
    let span = Span {
        start: expr.span.start,
        end,
    };
    match postfix.as_rule() {
        Rule::DataExprUpdate => Ok(DataExprKind::FunctionUpdate {
            expr: Box::new(expr),
            update: Box::new(Mcrl2Parser::DataExprUpdate(Node::new(postfix))?),
        }
        .spanned(span)),
        Rule::DataExprApplication => Ok(DataExprKind::Application {
            function: Box::new(expr),
            arguments: Mcrl2Parser::DataExprApplication(Node::new(postfix))?,
        }
        .spanned(span)),
        Rule::DataExprWhr => Ok(DataExprKind::Whr {
            expr: Box::new(expr),
            assignments: Mcrl2Parser::DataExprWhr(Node::new(postfix))?,
        }
        .spanned(span)),
        _ => unimplemented!("Unexpected postfix operator: {:?}", postfix.as_rule()),
    }
}

fn dataexpr_prefix(prefix: Pair<'_, Rule>, expr: ParseResult<DataExpr>) -> ParseResult<DataExpr> {
    let start = prefix.as_span().start();
    let expr = expr?;
    let span = Span {
        start,
        end: expr.span.end,
    };
    match prefix.as_rule() {
        Rule::DataExprForall => Ok(DataExprKind::Quantifier {
            op: Quantifier::Forall,
            variables: Mcrl2Parser::DataExprForall(Node::new(prefix))?,
            body: Box::new(expr),
        }
        .spanned(span)),
        Rule::DataExprExists => Ok(DataExprKind::Quantifier {
            op: Quantifier::Exists,
            variables: Mcrl2Parser::DataExprExists(Node::new(prefix))?,
            body: Box::new(expr),
        }
        .spanned(span)),
        Rule::DataExprLambda => Ok(DataExprKind::Lambda {
            variables: Mcrl2Parser::DataExprLambda(Node::new(prefix))?,
            body: Box::new(expr),
        }
        .spanned(span)),
        Rule::DataExprNegation => Ok(DataExprKind::Unary {
            op: DataExprUnaryOp::Negation,
            expr: Box::new(expr),
        }
        .spanned(span)),
        Rule::DataExprMinus => Ok(DataExprKind::Unary {
            op: DataExprUnaryOp::Minus,
            expr: Box::new(expr),
        }
        .spanned(span)),
        Rule::DataExprSize => Ok(DataExprKind::Unary {
            op: DataExprUnaryOp::Size,
            expr: Box::new(expr),
        }
        .spanned(span)),
        _ => unimplemented!("Unexpected prefix operator: {:?}", prefix.as_rule()),
    }
}

impl Operator for DataExprKind {
    fn fixity(&self) -> Fixity {
        match self {
            DataExprKind::Whr { .. } => Fixity::Postfix(0),
            DataExprKind::Lambda { .. } => Fixity::Prefix(1),
            DataExprKind::Quantifier { .. } => Fixity::Prefix(1),
            DataExprKind::Binary { op, .. } => match op {
                DataExprBinaryOp::Implies => Fixity::Infix(2, Assoc::Right),
                DataExprBinaryOp::Disj => Fixity::Infix(3, Assoc::Right),
                DataExprBinaryOp::Conj => Fixity::Infix(4, Assoc::Right),
                DataExprBinaryOp::Equal | DataExprBinaryOp::NotEqual => Fixity::Infix(5, Assoc::Left),
                DataExprBinaryOp::LessThan
                | DataExprBinaryOp::LessEqual
                | DataExprBinaryOp::GreaterThan
                | DataExprBinaryOp::GreaterEqual
                | DataExprBinaryOp::In => Fixity::Infix(6, Assoc::Left),
                DataExprBinaryOp::Cons => Fixity::Infix(7, Assoc::Right),
                DataExprBinaryOp::Snoc => Fixity::Infix(8, Assoc::Left),
                DataExprBinaryOp::Concat => Fixity::Infix(9, Assoc::Left),
                DataExprBinaryOp::Add | DataExprBinaryOp::Subtract => Fixity::Infix(10, Assoc::Left),
                DataExprBinaryOp::Div | DataExprBinaryOp::IntDiv | DataExprBinaryOp::Mod => {
                    Fixity::Infix(11, Assoc::Left)
                }
                DataExprBinaryOp::Multiply | DataExprBinaryOp::At => Fixity::Infix(12, Assoc::Left),
            },
            DataExprKind::Unary { .. } => Fixity::Prefix(12),
            DataExprKind::FunctionUpdate { .. } | DataExprKind::Application { .. } => Fixity::Postfix(13),
            DataExprKind::Id(_)
            | DataExprKind::Resolved(_, _)
            | DataExprKind::Number(_)
            | DataExprKind::Bool(_)
            | DataExprKind::EmptyList
            | DataExprKind::List(_)
            | DataExprKind::EmptySet
            | DataExprKind::Set(_)
            | DataExprKind::EmptyBag
            | DataExprKind::Bag(_)
            | DataExprKind::SetBagComp { .. } => Fixity::Primary,
        }
    }

    fn operand(&self) -> Option<&DataExpr> {
        match self {
            DataExprKind::Whr { expr, .. }
            | DataExprKind::FunctionUpdate { expr, .. }
            | DataExprKind::Application { function: expr, .. }
            | DataExprKind::Lambda { body: expr, .. }
            | DataExprKind::Quantifier { body: expr, .. }
            | DataExprKind::Unary { expr, .. } => Some(expr),
            _ => None,
        }
    }
}

#[merc_pest_consume::parser_methods]
impl Mcrl2Parser {
    pub(crate) fn DataExpr(expr: ParseNode) -> ParseResult<DataExpr> {
        parse_dataexpr(expr.children().as_pairs().clone())
    }

    pub(crate) fn DataExprUnit(expr: ParseNode) -> ParseResult<DataExpr> {
        parse_dataexpr(expr.children().as_pairs().clone())
    }

    pub(crate) fn DataValExpr(expr: ParseNode) -> ParseResult<DataExpr> {
        match_nodes!(expr.into_children();
            [DataExpr(expr)] => {
                Ok(expr)
            },
        )
    }

    pub(crate) fn DataExprUpdate(expr: ParseNode) -> ParseResult<DataExprUpdate> {
        match_nodes!(expr.into_children();
            [DataExpr(expr), DataExpr(update)] => {
                Ok(DataExprUpdate { expr, update })
            },
        )
    }

    pub(crate) fn DataExprApplication(expr: ParseNode) -> ParseResult<Vec<DataExpr>> {
        match_nodes!(expr.into_children();
            [DataExprList(expressions)] => {
                Ok(expressions)
            },
        )
    }

    pub(crate) fn DataExprWhr(expr: ParseNode) -> ParseResult<Vec<Assignment>> {
        match_nodes!(expr.into_children();
            [AssignmentList(assignments)] => {
                Ok(assignments)
            },
        )
    }

    pub(crate) fn AssignmentList(assignments: ParseNode) -> ParseResult<Vec<Assignment>> {
        match_nodes!(assignments.into_children();
            [Assignment(assignment)] => {
                Ok(vec![assignment])
            },
            [Assignment(assignment)..] => {
                Ok(assignment.collect())
            },
        )
    }

    pub(crate) fn Assignment(assignment: ParseNode) -> ParseResult<Assignment> {
        match_nodes!(assignment.into_children();
            [IdAt(identifier), DataExpr(expr)] => {
                let (identifier, span) = identifier.into_parts();
                Ok(AssignmentData { identifier, expr, id: None }.spanned(span))
            },
        )
    }

    pub(crate) fn DataExprSize(expr: ParseNode) -> ParseResult<DataExpr> {
        let span: Span = expr.as_span().into();
        match_nodes!(expr.into_children();
            [DataExpr(expr)] => {
                Ok(DataExprKind::Unary { op: DataExprUnaryOp::Size, expr: Box::new(expr) }.spanned(span))
            },
        )
    }

    pub(crate) fn DataExprListEnum(input: ParseNode) -> ParseResult<DataExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [DataExprList(expressions)] => {
                Ok(DataExprKind::List(expressions).spanned(span))
            },
        )
    }

    pub(crate) fn DataExprBagEnum(input: ParseNode) -> ParseResult<DataExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [BagEnumEltList(elements)] => {
                Ok(DataExprKind::Bag(elements).spanned(span))
            },
        )
    }

    fn BagEnumEltList(input: ParseNode) -> ParseResult<Vec<BagElement>> {
        match_nodes!(input.into_children();
            [BagEnumElt(elements)..] => {
                Ok(elements.collect())
            },
        )
    }

    fn BagEnumElt(input: ParseNode) -> ParseResult<BagElement> {
        match_nodes!(input.into_children();
            [DataExpr(expr), DataExpr(multiplicity)] => {
                Ok(BagElement { expr, multiplicity })
            },
        )
    }

    pub(crate) fn DataExprSetEnum(input: ParseNode) -> ParseResult<DataExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [DataExprList(expressions)] => {
                Ok(DataExprKind::Set(expressions).spanned(span))
            },
        )
    }

    pub(crate) fn DataExprSetBagComp(input: ParseNode) -> ParseResult<DataExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [VarDecl(variable), DataExpr(predicate)] => {
                Ok(DataExprKind::SetBagComp { variable, predicate: Box::new(predicate) }.spanned(span))
            },
        )
    }

    pub(crate) fn Number(input: ParseNode) -> ParseResult<DataExpr> {
        let span: Span = input.as_span().into();
        Ok(DataExprKind::Number(input.as_str().into()).spanned(span))
    }

    fn VarDecl(decl: ParseNode) -> ParseResult<IdDecl> {
        match_nodes!(decl.into_children();
            [IdAt(identifier), SortExpr(sort)] => {
                let (identifier, span) = identifier.into_parts();
                Ok(IdDecl::new(identifier, sort, span))
            },
        )
    }

    pub(crate) fn DataExprLambda(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(vars)] => {
                Ok(vars)
            },
        )
    }

    pub(crate) fn DataExprForall(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(vars)] => {
                Ok(vars)
            },
        )
    }

    pub(crate) fn DataExprExists(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(vars)] => {
                Ok(vars)
            },
        )
    }
}
