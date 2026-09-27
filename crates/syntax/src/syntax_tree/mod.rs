use std::fmt;

use merc_pest_consume::Error;
use merc_pest_consume::match_nodes;
use merc_utilities::IdAllocator;
use merc_utilities::Span;
use merc_utilities::TagIndex;
use pest::pratt_parser::Op;
use pest::pratt_parser::PrattParser;

use crate::Mcrl2Parser;
use crate::Rule;
use crate::spanned::Spanned;

mod actfrm;
mod dataexpr;
mod pbesexpr;
mod presexpr;
mod procexpr;
mod regfrm;
mod sortexpr;
mod specs;
mod statefrm;

pub use actfrm::*;
pub use dataexpr::*;
pub use pbesexpr::*;
pub use presexpr::*;
pub use procexpr::*;
pub use regfrm::*;
pub use sortexpr::*;
pub use specs::*;
pub use statefrm::*;

/// A unique type for sort declarations.
pub struct SortTag;

/// The index type for a sort declaration, assigned during name resolution.
pub type SortId = TagIndex<usize, SortTag>;

/// A unique type for constructor declarations.
pub struct ConstructorTag;

/// The index type for a constructor declaration, local to
/// `UntypedDataSpecification::constructor_declarations`.
pub type ConstructorId = TagIndex<usize, ConstructorTag>;

/// A unique type for map declarations.
pub struct MapTag;

/// The index type for a map declaration, local to
/// `UntypedDataSpecification::map_declarations`.
pub type MapId = TagIndex<usize, MapTag>;

/// A unique type for equation specification blocks (`var ... eqn ...`).
pub struct EqnSpecTag;

/// The index type for an equation specification block, local to
/// `UntypedDataSpecification::equation_declarations`.
pub type EqnSpecId = TagIndex<usize, EqnSpecTag>;

/// A unique type for equation declarations.
pub struct EquationTag;

/// The index type for a single equation, local to its enclosing `EqnSpec`.
pub type EquationId = TagIndex<usize, EquationTag>;

/// A unique type for variable-binder occurrences.
pub struct VarTag;

/// The index type assigned to every variable binder during variable resolution, spec-wide.
pub type VarId = TagIndex<usize, VarTag>;

/// Hands out fresh, spec-wide [VarId]s during variable resolution.
pub type VarIdAllocator = IdAllocator<VarTag>;

/// A unique type for a state-formula fixpoint-variable (`mu X`/`nu X`) binder.
pub struct StateVarTag;

/// The index type assigned to every fixpoint-variable binder during variable resolution,
/// spec-wide, mirroring [VarId] for the propositional namespace.
pub type StateVarId = TagIndex<usize, StateVarTag>;

/// Hands out fresh, spec-wide [StateVarId]s during variable resolution.
pub type StateVarIdAllocator = IdAllocator<StateVarTag>;

/// A unique type for a bound sort (type) variable.
pub struct TypeVarTag;

/// The index type for a bound sort variable.
pub type TypeVarId = TagIndex<usize, TypeVarTag>;

/// An identifier occurrence naming an action or process, paired with the [merc_utilities::Span] it
/// was parsed from, so a later pass can point at the individual name rather than the whole
/// enclosing expression. Used both for a name inside a `hide`/`block`/`allow`/`comm`/`rename` set
/// and for `ProcessExprKind::Action`/`Id`'s own name. Equality, ordering and hashing ignore the
/// span (see [Spanned]).
pub type ActionName = Spanned<String>;

/// A process-declaration identifier occurrence.
pub type ProcessName = Spanned<String>;

/// A propositional-variable identifier occurrence.
pub type PropVarName = Spanned<String>;

/// A declaration of an identifier with its sort.
///
/// Reused for every "name: sort" binding in the grammar. It defaults to [SortId]
/// for the binder-like uses that never assign one, and is instantiated with
/// [ConstructorId] or [MapId] where appropriate.
#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct IdDecl<Id = SortId> {
    /// Identifier being declared.
    pub identifier: Spanned<String>,
    /// Sort expression for this identifier
    pub sort: SortExpression,
    /// Unique ID assigned to this declaration during name/id resolution.
    pub id: Option<Id>,
    /// Assigned during variable resolution when this declaration is a variable binder (every
    /// site except a constructor/map declaration, which isn't a variable); `None` otherwise. See
    /// [VarId].
    pub var_id: Option<VarId>,
}

impl<Id> IdDecl<Id> {
    /// Creates a new identifier declaration with the given identifier, sort, and the identifier's
    /// own span.
    pub fn new(identifier: String, sort: SortExpression, span: merc_utilities::Span) -> Self {
        IdDecl {
            identifier: Spanned { node: identifier, span },
            sort,
            id: None,
            var_id: None,
        }
    }

    /// Reinterprets this declaration under a different id type.
    pub fn retag<NewId>(self) -> IdDecl<NewId> {
        IdDecl {
            identifier: self.identifier,
            sort: self.sort,
            id: None,
            var_id: self.var_id,
        }
    }
}

impl<Id> fmt::Display for IdDecl<Id> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}: {}", self.identifier.node, self.sort)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum Quantifier {
    Exists,
    Forall,
}

impl fmt::Display for Quantifier {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Quantifier::Exists => write!(f, "exists"),
            Quantifier::Forall => write!(f, "forall"),
        }
    }
}

// TODO: What should this be called?
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Bound {
    Inf,
    Sup,
    Sum,
}

impl fmt::Display for Bound {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Bound::Inf => write!(f, "inf"),
            Bound::Sum => write!(f, "sum"),
            Bound::Sup => write!(f, "sup"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Eq {
    EqInf,
    EqnInf,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum Condition {
    Condsm,
    Condeq,
}

/// An operator's associativity, independent of `pest`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Assoc {
    Left,
    Right,
}

impl From<Assoc> for pest::pratt_parser::Assoc {
    fn from(assoc: Assoc) -> Self {
        match assoc {
            Assoc::Left => pest::pratt_parser::Assoc::Left,
            Assoc::Right => pest::pratt_parser::Assoc::Right,
        }
    }
}

/// How an AST node's own operator participates in precedence: a prefix, infix or postfix
/// operator at the given level. Higher levels bind tighter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Fixity {
    Prefix(u8),
    Infix(u8, Assoc),
    Postfix(u8),
    Primary,
}

/// Implemented by every `*Kind` enum whose values are Pratt-parsed.
pub trait Operator: Sized + crate::TakeRecursiveChildren {
    /// Returns the fixity and precedence level of this operator.
    fn fixity(&self) -> Fixity;

    /// Returns the operand of this operator if it has one (prefix or postfix), or `None` otherwise.
    fn operand(&self) -> Option<&Spanned<Self>>;
}

/// One entry in a `*_OPERATORS` table: which grammar rule an operator parses from, alongside its
/// [Fixity].
#[derive(Clone, Copy)]
struct RuleFixity {
    rule: Rule,
    fixity: Fixity,
}

/// Builds a [PrattParser] whose precedence levels are exactly `table`'s own [Fixity] levels:
/// entries sharing a level become one Pratt-parser precedence step (combined with `|`, exactly as
/// a hand-written `.op(Op::infix(...) | Op::prefix(...))` would), lowest level first.
///
/// # Panics
///
/// Panics if `table` contains a `Fixity::Primary` entry — a primary rule is handled by
/// `map_primary` alone and never belongs in this table.
fn build_pratt_parser(table: &[RuleFixity]) -> PrattParser<Rule> {
    let max_level = table
        .iter()
        .map(|entry| match entry.fixity {
            Fixity::Prefix(level) | Fixity::Postfix(level) | Fixity::Infix(level, _) => level,
            Fixity::Primary => unreachable!("a primary rule never belongs in a *_OPERATORS table"),
        })
        .max()
        .unwrap_or(0);

    let mut parser = PrattParser::new();
    for level in 0..=max_level {
        let level_ops = table
            .iter()
            .filter(|entry| match entry.fixity {
                Fixity::Prefix(l) | Fixity::Postfix(l) | Fixity::Infix(l, _) => l == level,
                Fixity::Primary => false,
            })
            .map(|entry| match entry.fixity {
                Fixity::Prefix(_) => Op::prefix(entry.rule),
                Fixity::Postfix(_) => Op::postfix(entry.rule),
                Fixity::Infix(_, assoc) => Op::infix(entry.rule, assoc.into()),
                Fixity::Primary => unreachable!("a primary rule never belongs in a *_OPERATORS table"),
            })
            .reduce(|a, b| a | b);
        if let Some(level_ops) = level_ops {
            parser = parser.op(level_ops);
        }
    }
    parser
}

/// The error type produced while consuming the parse tree.
pub(crate) type ParseResult<T> = std::result::Result<T, Error<Rule>>;
pub(crate) type ParseNode<'i> = merc_pest_consume::Node<'i, Rule, ()>;

merc_pest_consume::declare_parser!(parser = Mcrl2Parser, rule = Rule);

/// Consumes the pest parse tree into syntax tree nodes, split by grammar area into
/// `sortexpr`/`dataexpr`/`procexpr`/`actfrm`/`regfrm`/`statefrm`/`pbesexpr`/`presexpr`/`specs`,
/// each its own `#[merc_pest_consume::parser_methods]` impl block for `Mcrl2Parser`.
///
/// Private consume methods are only called from `match_nodes!` arms within their own file.
/// `pub(crate)` methods are called across files in this module.
#[merc_pest_consume::parser_methods]
impl Mcrl2Parser {
    pub(crate) fn Id(identifier: ParseNode) -> ParseResult<Spanned<String>> {
        Ok(Spanned {
            node: identifier.as_str().to_string(),
            span: identifier.as_span().into(),
        })
    }

    pub(crate) fn IdAt(identifier: ParseNode) -> ParseResult<Spanned<String>> {
        Ok(Spanned {
            node: identifier.as_str().to_string(),
            span: identifier.as_span().into(),
        })
    }

    pub(crate) fn IdList(identifiers: ParseNode) -> ParseResult<Vec<(String, Span)>> {
        Ok(identifiers
            .into_children()
            .map(|node| (node.as_str().to_string(), node.as_span().into()))
            .collect())
    }

    pub(crate) fn VarsDeclList(vars: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(vars.into_children();
            [VarsDecl(decl)..] => {
                Ok(decl.flatten().collect())
            },
        )
    }

    fn VarsDecl(decl: ParseNode) -> ParseResult<Vec<IdDecl>> {
        let mut vars = Vec::new();

        match_nodes!(decl.into_children();
            [IdList(identifiers), SortExpr(sort)] => {
                for (id, span) in identifiers {
                    vars.push(IdDecl::new(id, sort.clone(), span));
                }
            },
        );

        Ok(vars)
    }

    pub(crate) fn DataExprList(expr: ParseNode) -> ParseResult<Vec<DataExpr>> {
        match_nodes!(expr.into_children();
            [DataExpr(expr)] => {
                Ok(vec![expr])
            },
            [DataExpr(expr)..] => {
                Ok(expr.collect())
            },
        )
    }
}
