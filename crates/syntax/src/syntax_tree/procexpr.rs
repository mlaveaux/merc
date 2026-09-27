use itertools::Itertools;
use merc_pest_consume::Node;
use merc_pest_consume::match_nodes;
use merc_utilities::Span;
use pest::iterators::Pair;
use pest::iterators::Pairs;
use pest::pratt_parser::PrattParser;
use std::fmt;
use std::iter;
use std::sync::LazyLock;

use crate::Action;
use crate::ActionName;
use crate::CommExpr;
use crate::DataExpr;
use crate::IdDecl;
use crate::Mcrl2Parser;
use crate::MultiAction;
use crate::MultiActionLabel;
use crate::ParseResult;
use crate::Rename;
use crate::Rule;
use crate::spanned::Spanned;

use super::Assignment;
use super::Assoc;
use super::Fixity;
use super::Operator;
use super::ParseNode;
use super::RuleFixity;
use super::build_pratt_parser;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum ProcExprBinaryOp {
    Sequence,
    Choice,
    Parallel,
    LeftMerge,
    CommMerge,
    Until,
}

/// The kind of a [ProcessExpr] node, without its source span. Every recursive
/// child is a [ProcessExpr] (a [Spanned] wrapper), so each node carries its own
/// location.
#[derive(Clone, Debug, Default, Eq, PartialEq, Hash)]
pub enum ProcessExprKind {
    // Both `Id`'s and `Action`'s own name keep the [Span] they were parsed from.
    Id(ActionName, Vec<Assignment>),
    Action(ActionName, Vec<DataExpr>),
    #[default]
    Delta,
    Tau,
    Sum {
        variables: Vec<IdDecl>,
        operand: Box<ProcessExpr>,
    },
    Dist {
        variables: Vec<IdDecl>,
        expr: DataExpr,
        operand: Box<ProcessExpr>,
    },
    Binary {
        op: ProcExprBinaryOp,
        lhs: Box<ProcessExpr>,
        rhs: Box<ProcessExpr>,
    },
    Hide {
        // Each action name keeps the [Span] it was parsed from, so a later pass can point at the
        // individual name rather than the whole `hide(...)` expression.
        actions: Vec<ActionName>,
        operand: Box<ProcessExpr>,
    },
    Rename {
        renames: Vec<Rename>,
        operand: Box<ProcessExpr>,
    },
    Allow {
        actions: Vec<MultiActionLabel>,
        operand: Box<ProcessExpr>,
    },
    Block {
        // See [ProcessExprKind::Hide] for why each name carries its own [Span].
        actions: Vec<ActionName>,
        operand: Box<ProcessExpr>,
    },
    Comm {
        comm: Vec<CommExpr>,
        operand: Box<ProcessExpr>,
    },
    Condition {
        condition: DataExpr,
        then: Box<ProcessExpr>,
        else_: Option<Box<ProcessExpr>>,
    },
    At {
        expr: Box<ProcessExpr>,
        operand: DataExpr,
    },
}

/// A process expression: a [ProcessExprKind] paired with the source [Span] it
/// was parsed from. Synthetic expressions built by later passes use
/// [Span::default].
pub type ProcessExpr = Spanned<ProcessExprKind>;

impl ProcessExprKind {
    /// Wraps this kind together with a source `span` into a [ProcessExpr].
    pub fn spanned(self, span: Span) -> ProcessExpr {
        Spanned { node: self, span }
    }
}

impl From<ProcessExprKind> for ProcessExpr {
    /// Wraps a kind into a [ProcessExpr] with a default (empty) span, for
    /// synthetic expressions that have no source location.
    fn from(kind: ProcessExprKind) -> Self {
        Spanned {
            node: kind,
            span: Span::default(),
        }
    }
}

impl fmt::Display for ProcExprBinaryOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcExprBinaryOp::Sequence => write!(f, "."),
            ProcExprBinaryOp::Choice => write!(f, "+"),
            ProcExprBinaryOp::Parallel => write!(f, "||"),
            ProcExprBinaryOp::LeftMerge => write!(f, "||_"),
            ProcExprBinaryOp::CommMerge => write!(f, "|"),
            ProcExprBinaryOp::Until => write!(f, "<<"),
        }
    }
}

impl fmt::Display for ProcessExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.node {
            ProcessExprKind::Id(identifier, assignments) => {
                if assignments.is_empty() {
                    write!(f, "{identifier}")
                } else {
                    write!(f, "{}({})", identifier, assignments.iter().format(", "))
                }
            }
            ProcessExprKind::Action(identifier, data_exprs) => {
                if data_exprs.is_empty() {
                    write!(f, "{identifier}")
                } else {
                    write!(f, "{}({})", identifier, data_exprs.iter().format(", "))
                }
            }
            ProcessExprKind::Delta => write!(f, "delta"),
            ProcessExprKind::Tau => write!(f, "tau"),
            ProcessExprKind::Sum { variables, operand } => {
                write!(f, "(sum {} . {})", variables.iter().format(", "), operand)
            }
            ProcessExprKind::Dist {
                variables,
                expr,
                operand,
            } => write!(f, "(dist {} [{}] . {})", variables.iter().format(", "), expr, operand),
            ProcessExprKind::Binary { op, lhs, rhs } => write!(f, "({lhs} {op} {rhs})"),
            ProcessExprKind::Hide { actions, operand } => {
                if !actions.is_empty() {
                    write!(f, "hide({{{}}}, {})", actions.iter().format(", "), operand)
                } else {
                    Ok(())
                }
            }
            ProcessExprKind::Rename { renames, operand } => {
                if !renames.is_empty() {
                    write!(f, "rename({{{}}}, {})", renames.iter().format(", "), operand)
                } else {
                    Ok(())
                }
            }
            ProcessExprKind::Allow { actions, operand } => {
                if !actions.is_empty() {
                    write!(f, "allow({{{}}}, {})", actions.iter().format(", "), operand)
                } else {
                    Ok(())
                }
            }
            ProcessExprKind::Block { actions, operand } => {
                if !actions.is_empty() {
                    write!(f, "block({{{}}}, {})", actions.iter().format(", "), operand)
                } else {
                    Ok(())
                }
            }
            ProcessExprKind::Comm { comm, operand } => {
                if !comm.is_empty() {
                    write!(f, "comm({{{}}}, {})", comm.iter().format(", "), operand)
                } else {
                    Ok(())
                }
            }
            ProcessExprKind::Condition { condition, then, else_ } => {
                // Wrap the whole conditional so it stays a single unit when it is
                // an operand of a higher-precedence operator such as sequence.
                if let Some(else_) = else_ {
                    write!(f, "(({condition}) -> ({then}) <> ({else_}))")
                } else {
                    write!(f, "(({condition}) -> ({then}))")
                }
            }
            ProcessExprKind::At { expr, operand } => write!(f, "({expr})@({operand})"),
        }
    }
}

/// Precedence table for [ProcessExprKind], lowest level first — see [build_pratt_parser] and
/// [Operator].
const PROCEXPR_OPERATORS: &[RuleFixity] = &[
    RuleFixity {
        rule: Rule::ProcExprChoice,
        fixity: Fixity::Infix(0, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::ProcExprSum,
        fixity: Fixity::Prefix(1),
    },
    RuleFixity {
        rule: Rule::ProcExprDist,
        fixity: Fixity::Prefix(1),
    },
    RuleFixity {
        rule: Rule::ProcExprParallel,
        fixity: Fixity::Infix(2, Assoc::Right),
    },
    RuleFixity {
        rule: Rule::ProcExprLeftMerge,
        fixity: Fixity::Infix(3, Assoc::Right),
    },
    RuleFixity {
        rule: Rule::ProcExprIf,
        fixity: Fixity::Prefix(4),
    },
    RuleFixity {
        rule: Rule::ProcExprIfThen,
        fixity: Fixity::Prefix(4),
    },
    RuleFixity {
        rule: Rule::ProcExprUntil,
        fixity: Fixity::Infix(5, Assoc::Left),
    },
    RuleFixity {
        rule: Rule::ProcExprSeq,
        fixity: Fixity::Infix(6, Assoc::Right),
    },
    RuleFixity {
        rule: Rule::ProcExprAt,
        fixity: Fixity::Postfix(7),
    },
    RuleFixity {
        rule: Rule::ProcExprSync,
        fixity: Fixity::Infix(8, Assoc::Left),
    },
];

static PROCEXPR_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| build_pratt_parser(PROCEXPR_OPERATORS));

#[allow(clippy::result_large_err)]
pub fn parse_process_expr(pairs: Pairs<Rule>) -> ParseResult<ProcessExpr> {
    PROCEXPR_PRATT_PARSER
        .map_primary(procexpr_primary)
        .map_infix(procexpr_infix)
        .map_prefix(procexpr_prefix)
        .map_postfix(procexpr_postfix)
        .parse(pairs)
}

fn procexpr_primary(primary: Pair<'_, Rule>) -> ParseResult<ProcessExpr> {
    let span: Span = primary.as_span().into();
    match primary.as_rule() {
        Rule::ProcExprId => Ok(Mcrl2Parser::ProcExprId(Node::new(primary))?),
        Rule::ProcExprDelta => Ok(ProcessExprKind::Delta.spanned(span)),
        Rule::ProcExprTau => Ok(ProcessExprKind::Tau.spanned(span)),
        Rule::ProcExprBlock => Ok(Mcrl2Parser::ProcExprBlock(Node::new(primary))?),
        Rule::ProcExprAllow => Ok(Mcrl2Parser::ProcExprAllow(Node::new(primary))?),
        Rule::ProcExprHide => Ok(Mcrl2Parser::ProcExprHide(Node::new(primary))?),
        Rule::ProcExprRename => Ok(Mcrl2Parser::ProcExprRename(Node::new(primary))?),
        Rule::ProcExprComm => Ok(Mcrl2Parser::ProcExprComm(Node::new(primary))?),
        Rule::Action => {
            let action = Mcrl2Parser::Action(Node::new(primary))?;

            Ok(ProcessExprKind::Action(action.id, action.args).spanned(span))
        }
        Rule::ProcExprBrackets => {
            // Handle parentheses by recursively parsing the inner expression
            let inner = primary
                .into_inner()
                .next()
                .expect("Expected inner expression in brackets");
            parse_process_expr(inner.into_inner())
        }
        _ => unimplemented!("Unexpected rule: {:?}", primary.as_rule()),
    }
}

fn procexpr_infix(
    lhs: ParseResult<ProcessExpr>,
    op: Pair<'_, Rule>,
    rhs: ParseResult<ProcessExpr>,
) -> ParseResult<ProcessExpr> {
    let lhs = lhs?;
    let rhs = rhs?;
    let span = Span {
        start: lhs.span.start,
        end: rhs.span.end,
    };
    let op = match op.as_rule() {
        Rule::ProcExprChoice => ProcExprBinaryOp::Choice,
        Rule::ProcExprParallel => ProcExprBinaryOp::Parallel,
        Rule::ProcExprLeftMerge => ProcExprBinaryOp::LeftMerge,
        Rule::ProcExprSeq => ProcExprBinaryOp::Sequence,
        Rule::ProcExprSync => ProcExprBinaryOp::CommMerge,
        Rule::ProcExprUntil => ProcExprBinaryOp::Until,
        _ => unimplemented!("Unexpected rule: {:?}", op.as_rule()),
    };
    Ok(ProcessExprKind::Binary {
        op,
        lhs: Box::new(lhs),
        rhs: Box::new(rhs),
    }
    .spanned(span))
}

fn procexpr_prefix(prefix: Pair<'_, Rule>, expr: ParseResult<ProcessExpr>) -> ParseResult<ProcessExpr> {
    let start = prefix.as_span().start();
    let expr = expr?;
    let span = Span {
        start,
        end: expr.span.end,
    };
    match prefix.as_rule() {
        Rule::ProcExprSum => Ok(ProcessExprKind::Sum {
            variables: Mcrl2Parser::ProcExprSum(Node::new(prefix))?,
            operand: Box::new(expr),
        }
        .spanned(span)),
        Rule::ProcExprDist => {
            let (variables, data_expr) = Mcrl2Parser::ProcExprDist(Node::new(prefix))?;

            Ok(ProcessExprKind::Dist {
                variables,
                expr: data_expr,
                operand: Box::new(expr),
            }
            .spanned(span))
        }
        Rule::ProcExprIf => {
            let condition = Mcrl2Parser::ProcExprIf(Node::new(prefix))?;

            Ok(ProcessExprKind::Condition {
                condition,
                then: Box::new(expr),
                else_: None,
            }
            .spanned(span))
        }
        Rule::ProcExprIfThen => {
            let (condition, then) = Mcrl2Parser::ProcExprIfThen(Node::new(prefix))?;

            Ok(ProcessExprKind::Condition {
                condition,
                then: Box::new(then),
                else_: Some(Box::new(expr)),
            }
            .spanned(span))
        }
        _ => unimplemented!("Unexpected rule: {:?}", prefix.as_rule()),
    }
}

fn procexpr_postfix(expr: ParseResult<ProcessExpr>, postfix: Pair<'_, Rule>) -> ParseResult<ProcessExpr> {
    let expr = expr?;
    let span = Span {
        start: expr.span.start,
        end: postfix.as_span().end(),
    };
    match postfix.as_rule() {
        Rule::ProcExprAt => Ok(ProcessExprKind::At {
            expr: Box::new(expr),
            operand: Mcrl2Parser::ProcExprAt(Node::new(postfix))?,
        }
        .spanned(span)),
        _ => unimplemented!("Unexpected postfix rule: {:?}", postfix.as_rule()),
    }
}

impl Operator for ProcessExprKind {
    fn fixity(&self) -> Fixity {
        match self {
            ProcessExprKind::Binary { op, .. } => match op {
                ProcExprBinaryOp::Choice => Fixity::Infix(0, Assoc::Left),
                ProcExprBinaryOp::Parallel => Fixity::Infix(2, Assoc::Right),
                ProcExprBinaryOp::LeftMerge => Fixity::Infix(3, Assoc::Right),
                ProcExprBinaryOp::Until => Fixity::Infix(5, Assoc::Left),
                ProcExprBinaryOp::Sequence => Fixity::Infix(6, Assoc::Right),
                ProcExprBinaryOp::CommMerge => Fixity::Infix(8, Assoc::Left),
            },
            ProcessExprKind::Sum { .. } | ProcessExprKind::Dist { .. } => Fixity::Prefix(1),
            ProcessExprKind::Condition { .. } => Fixity::Prefix(4),
            ProcessExprKind::At { .. } => Fixity::Postfix(7),
            ProcessExprKind::Id(_, _)
            | ProcessExprKind::Action(_, _)
            | ProcessExprKind::Delta
            | ProcessExprKind::Tau
            | ProcessExprKind::Hide { .. }
            | ProcessExprKind::Rename { .. }
            | ProcessExprKind::Allow { .. }
            | ProcessExprKind::Block { .. }
            | ProcessExprKind::Comm { .. } => Fixity::Primary,
        }
    }

    fn operand(&self) -> Option<&ProcessExpr> {
        match self {
            ProcessExprKind::Sum { operand, .. } | ProcessExprKind::Dist { operand, .. } => Some(operand),
            ProcessExprKind::Condition { then, else_, .. } => Some(else_.as_deref().unwrap_or(then)),
            ProcessExprKind::At { expr, .. } => Some(expr),
            _ => None,
        }
    }
}

#[merc_pest_consume::parser_methods]
impl Mcrl2Parser {
    pub(crate) fn ProcExprAt(input: ParseNode) -> ParseResult<DataExpr> {
        match_nodes!(input.into_children();
            [DataExprUnit(expr)] => {
                Ok(expr)
            },
        )
    }

    pub(crate) fn ActIdSet(actions: ParseNode) -> ParseResult<Vec<ActionName>> {
        match_nodes!(actions.into_children();
            [IdList(list)] => {
                Ok(list.into_iter().map(|(node, span)| ActionName { node, span }).collect())
            },
        )
    }

    fn MultActId(actions: ParseNode) -> ParseResult<MultiActionLabel> {
        match_nodes!(actions.into_children();
            [Id(actions)..] => {
                Ok(MultiActionLabel { actions: actions.collect() })
            },
        )
    }

    fn MultActIdList(actions: ParseNode) -> ParseResult<Vec<MultiActionLabel>> {
        match_nodes!(actions.into_children();
            [MultActId(action), MultActId(actions)..] => {
                Ok(iter::once(action).chain(actions).collect())
            },
        )
    }

    pub(crate) fn MultActIdSet(actions: ParseNode) -> ParseResult<Vec<MultiActionLabel>> {
        match_nodes!(actions.into_children();
            [MultActIdList(list)] => {
                Ok(list)
            },
        )
    }

    pub(crate) fn ProcExpr(input: ParseNode) -> ParseResult<ProcessExpr> {
        parse_process_expr(input.children().as_pairs().clone())
    }

    fn ProcExprNoIf(input: ParseNode) -> ParseResult<ProcessExpr> {
        parse_process_expr(input.children().as_pairs().clone())
    }

    pub(crate) fn ProcExprId(input: ParseNode) -> ParseResult<ProcessExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [Id(identifier)] => {
                Ok(ProcessExprKind::Id(identifier, Vec::new()).spanned(span))
            },
            [Id(identifier), AssignmentList(assignments)] => {
                Ok(ProcessExprKind::Id(identifier, assignments).spanned(span))
            },
        )
    }

    pub(crate) fn ProcExprBlock(input: ParseNode) -> ParseResult<ProcessExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [ActIdSet(actions), ProcExpr(expr)] => {
                Ok(ProcessExprKind::Block {
                    actions,
                    operand: Box::new(expr),
                }.spanned(span))
            },
        )
    }

    pub(crate) fn ProcExprIf(input: ParseNode) -> ParseResult<DataExpr> {
        match_nodes!(input.into_children();
            [DataExpr(condition)] => {
                Ok(condition)
            },
        )
    }

    pub(crate) fn ProcExprIfThen(input: ParseNode) -> ParseResult<(DataExpr, ProcessExpr)> {
        match_nodes!(input.into_children();
            [DataExpr(condition), ProcExprNoIf(expr)] => {
                Ok((condition, expr))
            },
        )
    }

    pub(crate) fn ProcExprAllow(input: ParseNode) -> ParseResult<ProcessExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [MultActIdSet(actions), ProcExpr(expr)] => {
                Ok(ProcessExprKind::Allow {
                    actions,
                    operand: Box::new(expr),
                }.spanned(span))
            },
        )
    }

    pub(crate) fn ProcExprHide(input: ParseNode) -> ParseResult<ProcessExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [ActIdSet(actions), ProcExpr(expr)] => {
                Ok(ProcessExprKind::Hide {
                    actions,
                    operand: Box::new(expr),
                }.spanned(span))
            },
        )
    }

    fn ActionList(actions: ParseNode) -> ParseResult<Vec<Action>> {
        match_nodes!(actions.into_children();
            [Action(action), Action(actions)..] => {
                Ok(iter::once(action).chain(actions).collect())
            },
        )
    }

    pub(crate) fn MultiActTau(_input: ParseNode) -> ParseResult<()> {
        Ok(())
    }

    pub(crate) fn ProcExprDelta(_input: ParseNode) -> ParseResult<()> {
        Ok(())
    }

    pub(crate) fn MultAct(input: ParseNode) -> ParseResult<MultiAction> {
        match_nodes!(input.into_children();
            [MultiActTau(_)] => {
                Ok(MultiAction { actions: Vec::new() })
            },
            [ActionList(actions)] => {
                Ok(MultiAction { actions })
            },
        )
    }

    fn CommExpr(action: ParseNode) -> ParseResult<CommExpr> {
        match_nodes!(action.into_children();
            [Id(first), MultActId(mut multiact), Id(to)] => {
                multiact.actions.insert(0, first);
                Ok(CommExpr { from: multiact, to })
            },
        )
    }

    fn CommExprList(actions: ParseNode) -> ParseResult<Vec<CommExpr>> {
        match_nodes!(actions.into_children();
            [CommExpr(action), CommExpr(actions)..] => {
                Ok(iter::once(action).chain(actions).collect())
            },
        )
    }

    pub(crate) fn CommExprSet(actions: ParseNode) -> ParseResult<Vec<CommExpr>> {
        match_nodes!(actions.into_children();
            [CommExprList(list)] => {
                Ok(list)
            },
        )
    }

    pub(crate) fn ProcExprRename(input: ParseNode) -> ParseResult<ProcessExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [RenExprSet(renames), ProcExpr(expr)] => {
                Ok(ProcessExprKind::Rename {
                    renames,
                    operand: Box::new(expr),
                }.spanned(span))
            },
        )
    }

    pub(crate) fn ProcExprComm(input: ParseNode) -> ParseResult<ProcessExpr> {
        let span: Span = input.as_span().into();
        match_nodes!(input.into_children();
            [CommExprSet(comm), ProcExpr(expr)] => {
                Ok(ProcessExprKind::Comm {
                    comm,
                    operand: Box::new(expr),
                }.spanned(span))
            },
        )
    }

    pub(crate) fn Action(input: ParseNode) -> ParseResult<Action> {
        match_nodes!(input.into_children();
            [Id(id)] => {
                Ok(Action { id, args: Vec::new() })
            },
            [Id(id), DataExprList(args)] => {
                Ok(Action { id, args })
            },
        )
    }

    fn RenExprSet(renames: ParseNode) -> ParseResult<Vec<Rename>> {
        match_nodes!(renames.into_children();
            [RenExprList(renames)] => {
                Ok(renames)
            },
        )
    }

    fn RenExprList(renames: ParseNode) -> ParseResult<Vec<Rename>> {
        match_nodes!(renames.into_children();
            [RenExpr(renames)..] => {
                Ok(renames.collect())
            },
        )
    }

    fn RenExpr(renames: ParseNode) -> ParseResult<Rename> {
        match_nodes!(renames.into_children();
            [Id(from), Id(to)] => {
                Ok(Rename { from, to })
            },
        )
    }

    pub(crate) fn ProcExprSum(input: ParseNode) -> ParseResult<Vec<IdDecl>> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables)] => {
                Ok(variables)
            },
        )
    }

    pub(crate) fn ProcExprDist(input: ParseNode) -> ParseResult<(Vec<IdDecl>, DataExpr)> {
        match_nodes!(input.into_children();
            [VarsDeclList(variables), DataExpr(expr)] => {
                Ok((variables, expr))
            },
        )
    }
}
