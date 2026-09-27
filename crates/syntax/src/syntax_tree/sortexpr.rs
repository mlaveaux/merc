use itertools::Itertools;
use merc_pest_consume::Node;
use merc_pest_consume::match_nodes;
use merc_utilities::Span;
use pest::iterators::Pair;
use pest::iterators::Pairs;
use pest::pratt_parser::PrattParser;
use std::fmt;
use std::sync::LazyLock;

use crate::Mcrl2Parser;
use crate::ParseResult;
use crate::Rule;
use crate::Spanned;

use super::Assoc;
use super::Fixity;
use super::Operator;
use super::ParseNode;
use super::RuleFixity;
use super::SortId;
use super::TypeVarId;
use super::build_pratt_parser;

/// The kind of a [SortExpression] node.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum SortExpressionKind {
    /// Product of two sorts (A # B)
    Product {
        lhs: Box<SortExpression>,
        rhs: Box<SortExpression>,
    },
    /// Function sort (A -> B)
    Function {
        domain: Box<SortExpression>,
        range: Box<SortExpression>,
    },
    Struct {
        inner: Vec<ConstructorDecl>,
    },
    /// Reference to a named sort
    Reference(String),
    /// A bound sort (type) variable, such as the `S` in a container spec.
    TypeVar(String),
    /// A bound sort (type) variable after name resolution has assigned its
    /// [TypeVarId], mirroring how [Self::Reference] becomes [Self::Resolved].
    ResolvedTypeVar(TypeVarId),
    /// Built-in simple sort
    Simple(Sort),
    /// Parameterized complex sort
    Complex(ComplexSort, Box<SortExpression>),
    /// Resolved reference to a sort after name resolution
    Resolved(String, SortId),
    /// Function sort (A_0 # ... # A_n -> B) after flattening (performed during name resolution)
    FlattenedFunction {
        domain: Vec<SortExpression>,
        range: Box<SortExpression>,
    },
}

// `#[derive(Default)]`'s `#[default]` attribute only accepts a unit variant, and `SortExpressionKind`
// has none; a manual impl is the only way to give it the cheap, non-recursive default
// `TakeRecursiveChildren`'s generated impl (see `crates/syntax/src/traverse.rs`) needs.
impl Default for SortExpressionKind {
    fn default() -> Self {
        SortExpressionKind::Reference(String::default())
    }
}

/// A sort expression paired with the source [Span] it was parsed from.
pub type SortExpression = Spanned<SortExpressionKind>;

impl SortExpressionKind {
    /// Wraps this kind together with a source `span` into a [SortExpression].
    pub fn spanned(self, span: Span) -> SortExpression {
        Spanned { node: self, span }
    }
}

impl From<SortExpressionKind> for SortExpression {
    /// For synthetic expressions that have no source location.
    fn from(kind: SortExpressionKind) -> Self {
        Spanned {
            node: kind,
            span: Span::default(),
        }
    }
}

/// Constructor declaration
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct ConstructorDecl {
    /// The constructor's own name (`c1`), with its declaration span.
    pub name: Spanned<String>,
    /// Each argument's optional projection-function name.
    pub args: Vec<(Option<Spanned<String>>, SortExpression)>,
    /// The recogniser function's name.
    pub recogniser: Option<Spanned<String>>,
}

/// Built-in simple sorts.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum Sort {
    Bool,
    Pos,
    Int,
    Nat,
    Real,
}

impl Sort {
    /// This sort's mCRL2 name, matching the literal `SortId` names the binary
    /// aterm format uses; also `Sort`'s own [`Display`](std::fmt::Display) text.
    pub const fn name(self) -> &'static str {
        match self {
            Sort::Bool => "Bool",
            Sort::Pos => "Pos",
            Sort::Int => "Int",
            Sort::Nat => "Nat",
            Sort::Real => "Real",
        }
    }
}

/// Complex (parameterized) sorts.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum ComplexSort {
    List,
    Set,
    FSet,
    FBag,
    Bag,
}

impl fmt::Display for Sort {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl fmt::Display for ComplexSort {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl fmt::Display for SortExpression {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.node {
            SortExpressionKind::Product { lhs, rhs } => write!(f, "({lhs} # {rhs})"),
            SortExpressionKind::Function { domain, range } => write!(f, "({domain} -> {range})"),
            SortExpressionKind::Reference(name) => write!(f, "{name}"),
            SortExpressionKind::TypeVar(name) => write!(f, "'{name}"),
            SortExpressionKind::ResolvedTypeVar(id) => write!(f, "'{id}"),
            SortExpressionKind::Simple(sort) => write!(f, "{sort}"),
            SortExpressionKind::Complex(complex, inner) => write!(f, "{complex}({inner})"),
            SortExpressionKind::Struct { inner } => {
                write!(f, "struct ")?;
                write!(f, "{}", inner.iter().format(" | "))
            }
            SortExpressionKind::Resolved(name, _id) => write!(f, "{name}"),
            SortExpressionKind::FlattenedFunction { domain, range } => {
                let domain = domain.iter().format(" # ");
                write!(f, "({domain} -> {range})")
            }
        }
    }
}

impl fmt::Display for ConstructorDecl {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.args.is_empty() {
            write!(f, "{}", self.name.node)?;

            if let Some(recogniser) = &self.recogniser {
                write!(f, "?{}", recogniser.node)?;
            }

            Ok(())
        } else {
            write!(f, "{}(", self.name.node)?;
            for (i, (name, sort)) in self.args.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                match name {
                    Some(name) => write!(f, "{} : {sort}", name.node)?,
                    None => write!(f, "{sort}")?,
                }
            }
            write!(f, ")")?;

            if let Some(recogniser) = &self.recogniser {
                write!(f, "?{}", recogniser.node)?;
            }

            Ok(())
        }
    }
}

/// Precedence table for [SortExpressionKind], lowest level first — see [build_pratt_parser] and
/// [Operator].
const SORTEXPR_OPERATORS: &[RuleFixity] = &[
    RuleFixity {
        rule: Rule::SortExprFunction,
        fixity: Fixity::Infix(0, Assoc::Right),
    },
    RuleFixity {
        rule: Rule::SortExprProduct,
        fixity: Fixity::Infix(1, Assoc::Left),
    },
];

static SORT_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| build_pratt_parser(SORTEXPR_OPERATORS));

#[allow(clippy::result_large_err)]
pub fn parse_sortexpr_primary(primary: Pair<'_, Rule>) -> ParseResult<SortExpression> {
    let span: Span = primary.as_span().into();
    if let Some(sort) = simple_sort(primary.as_rule()) {
        return Ok(SortExpressionKind::Simple(sort).spanned(span));
    }
    match primary.as_rule() {
        Rule::IdAt => {
            Ok(SortExpressionKind::Reference(Mcrl2Parser::IdAt(Node::new(primary))?.into_node()).spanned(span))
        }
        Rule::SortExpr => Mcrl2Parser::SortExpr(Node::new(primary)),

        Rule::SortExprList => Mcrl2Parser::SortExprList(Node::new(primary)),
        Rule::SortExprSet => Mcrl2Parser::SortExprSet(Node::new(primary)),
        Rule::SortExprBag => Mcrl2Parser::SortExprBag(Node::new(primary)),
        Rule::SortExprFSet => Mcrl2Parser::SortExprFSet(Node::new(primary)),
        Rule::SortExprFBag => Mcrl2Parser::SortExprFBag(Node::new(primary)),

        Rule::SortExprParens => {
            // Handle parentheses by recursively parsing the inner expression
            let inner = primary
                .into_inner()
                .next()
                .expect("Expected inner expression in brackets");
            parse_sortexpr(inner.into_inner())
        }

        Rule::SortExprStruct => Mcrl2Parser::SortExprStruct(Node::new(primary)),
        _ => unimplemented!("Unexpected rule: {:?}", primary.as_rule()),
    }
}

/// The atomic sorts (`Bool`, `Int`, `Pos`, `Nat`, `Real`) that need no further parsing.
fn simple_sort(rule: Rule) -> Option<Sort> {
    match rule {
        Rule::SortExprBool => Some(Sort::Bool),
        Rule::SortExprInt => Some(Sort::Int),
        Rule::SortExprPos => Some(Sort::Pos),
        Rule::SortExprNat => Some(Sort::Nat),
        Rule::SortExprReal => Some(Sort::Real),
        _ => None,
    }
}

/// Parses a sequence of `Rule` pairs into a `SortExpression` using a Pratt parser for operator precedence.
///
/// # Panics
///
/// Panics if `pairs` were not produced by the `SortExpr` grammar rule.
#[allow(clippy::result_large_err)]
pub fn parse_sortexpr(pairs: Pairs<Rule>) -> ParseResult<SortExpression> {
    SORT_PRATT_PARSER
        .map_primary(|primary| parse_sortexpr_primary(primary))
        .map_infix(|lhs, op, rhs| {
            let lhs = lhs?;
            let rhs = rhs?;
            let span = Span {
                start: lhs.span.start,
                end: rhs.span.end,
            };
            match op.as_rule() {
                Rule::SortExprFunction => Ok(SortExpressionKind::Function {
                    domain: Box::new(lhs),
                    range: Box::new(rhs),
                }
                .spanned(span)),
                Rule::SortExprProduct => Ok(SortExpressionKind::Product {
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                }
                .spanned(span)),
                _ => unimplemented!("Unexpected binary operator: {:?}", op.as_rule()),
            }
        })
        .parse(pairs)
}

impl Operator for SortExpressionKind {
    fn fixity(&self) -> Fixity {
        match self {
            SortExpressionKind::Function { .. } => Fixity::Infix(0, Assoc::Right),
            SortExpressionKind::Product { .. } => Fixity::Infix(1, Assoc::Left),
            SortExpressionKind::Struct { .. }
            | SortExpressionKind::Reference(_)
            | SortExpressionKind::TypeVar(_)
            | SortExpressionKind::ResolvedTypeVar(_)
            | SortExpressionKind::Simple(_)
            | SortExpressionKind::Complex(_, _)
            | SortExpressionKind::Resolved(_, _)
            | SortExpressionKind::FlattenedFunction { .. } => Fixity::Primary,
        }
    }

    fn operand(&self) -> Option<&SortExpression> {
        None
    }
}

#[merc_pest_consume::parser_methods]
impl Mcrl2Parser {
    fn SortExprPrimary(sort: ParseNode) -> ParseResult<SortExpression> {
        parse_sortexpr(sort.children().as_pairs().clone())
    }

    pub(crate) fn SortExpr(expr: ParseNode) -> ParseResult<SortExpression> {
        parse_sortexpr(expr.children().as_pairs().clone())
    }

    // Complex sorts
    pub(crate) fn SortExprList(inner: ParseNode) -> ParseResult<SortExpression> {
        let span: Span = inner.as_span().into();
        Ok(SortExpressionKind::Complex(
            ComplexSort::List,
            Box::new(parse_sortexpr(inner.children().as_pairs().clone())?),
        )
        .spanned(span))
    }

    pub(crate) fn SortExprSet(inner: ParseNode) -> ParseResult<SortExpression> {
        let span: Span = inner.as_span().into();
        Ok(SortExpressionKind::Complex(
            ComplexSort::Set,
            Box::new(parse_sortexpr(inner.children().as_pairs().clone())?),
        )
        .spanned(span))
    }

    pub(crate) fn SortExprBag(inner: ParseNode) -> ParseResult<SortExpression> {
        let span: Span = inner.as_span().into();
        Ok(SortExpressionKind::Complex(
            ComplexSort::Bag,
            Box::new(parse_sortexpr(inner.children().as_pairs().clone())?),
        )
        .spanned(span))
    }

    pub(crate) fn SortExprFSet(inner: ParseNode) -> ParseResult<SortExpression> {
        let span: Span = inner.as_span().into();
        Ok(SortExpressionKind::Complex(
            ComplexSort::FSet,
            Box::new(parse_sortexpr(inner.children().as_pairs().clone())?),
        )
        .spanned(span))
    }

    pub(crate) fn SortExprFBag(inner: ParseNode) -> ParseResult<SortExpression> {
        let span: Span = inner.as_span().into();
        Ok(SortExpressionKind::Complex(
            ComplexSort::FBag,
            Box::new(parse_sortexpr(inner.children().as_pairs().clone())?),
        )
        .spanned(span))
    }

    pub(crate) fn SortExprStruct(inner: ParseNode) -> ParseResult<SortExpression> {
        let span: Span = inner.as_span().into();
        match_nodes!(inner.into_children();
            [ConstrDeclList(inner)] => {
                Ok(SortExpressionKind::Struct { inner }.spanned(span))
            },
        )
    }

    pub(crate) fn ConstrDeclList(input: ParseNode) -> ParseResult<Vec<ConstructorDecl>> {
        match_nodes!(input.into_children();
            [ConstrDecl(decl)..] => {
                Ok(decl.collect())
            },
        )
    }

    // `ConstrDecl = { IdAt ~ ( "(" ~ ProjDeclList ~ ")" )? ~ ( "?" ~ IdAt )? }`: one arm per
    // combination of the two optional groups. The leading name and the trailing recogniser each
    // keep their own span.
    pub(crate) fn ConstrDecl(input: ParseNode) -> ParseResult<ConstructorDecl> {
        match_nodes!(input.into_children();
            [IdAt(name)] => {
                Ok(ConstructorDecl { name, args: Vec::new(), recogniser: None })
            },
            [IdAt(name), ProjDeclList(args)] => {
                Ok(ConstructorDecl { name, args, recogniser: None })
            },
            [IdAt(name), IdAt(recogniser)] => {
                Ok(ConstructorDecl { name, args: Vec::new(), recogniser: Some(recogniser) })
            },
            [IdAt(name), ProjDeclList(args), IdAt(recogniser)] => {
                Ok(ConstructorDecl { name, args, recogniser: Some(recogniser) })
            },
        )
    }

    pub(crate) fn ProjDeclList(input: ParseNode) -> ParseResult<Vec<(Option<Spanned<String>>, SortExpression)>> {
        match_nodes!(input.into_children();
            [ProjDecl(decl)..] => {
                Ok(decl.collect())
            },
        )
    }

    pub(crate) fn ProjDecl(input: ParseNode) -> ParseResult<(Option<Spanned<String>>, SortExpression)> {
        match_nodes!(input.into_children();
            [SortExpr(sort)] => {
                Ok((None, sort))
            },
            [Id(name), SortExpr(sort)] => {
                Ok((Some(name), sort))
            },
        )
    }

    pub(crate) fn SortProduct(sort: ParseNode) -> ParseResult<Vec<SortExpression>> {
        let mut iter = sort.into_children();

        // An expression of the shape SortExprPrimary ~ (SortExprProduct ~ SortExprPrimary)*
        let mut result = vec![parse_sortexpr_primary(iter.next().unwrap().as_pair().clone())?];

        for mut chunk in &iter.chunks(2) {
            if chunk.next().unwrap().as_rule() == Rule::SortExprProduct {
                let sort = parse_sortexpr_primary(chunk.next().unwrap().as_pair().clone())?;
                result.push(sort);
            }
        }

        Ok(result)
    }
}
