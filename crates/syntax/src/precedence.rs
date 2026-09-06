use std::sync::LazyLock;

use pest::iterators::Pair;
use pest::iterators::Pairs;
use pest::pratt_parser::Assoc;
use pest::pratt_parser::Op;
use pest::pratt_parser::PrattParser;

use merc_pest_consume::Node;
use merc_utilities::Span;

use crate::ActFrm;
use crate::ActFrmBinaryOp;
use crate::ActFrmKind;
use crate::Bound;
use crate::DataExpr;
use crate::DataExprBinaryOp;
use crate::DataExprKind;
use crate::DataExprUnaryOp;
use crate::FixedPointOperator;
use crate::Mcrl2Parser;
use crate::ModalityOperator;
use crate::ParseResult;
use crate::PbesExpr;
use crate::PbesExprBinaryOp;
use crate::PbesExprKind;
use crate::PresExpr;
use crate::PresExprBinaryOp;
use crate::PresExprKind;
use crate::ProcExprBinaryOp;
use crate::ProcessExpr;
use crate::ProcessExprKind;
use crate::Quantifier;
use crate::RegFrm;
use crate::RegFrmKind;
use crate::Rule;
use crate::Sort;
use crate::StateFrm;
use crate::StateFrmKind;
use crate::StateFrmOp;
use crate::StateFrmUnaryOp;
use crate::syntax_tree::SortExpression;
use crate::syntax_tree::SortExpressionKind;

pub static SORT_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| {
    // Precedence is defined lowest to highest
    PrattParser::new()
        // Sort operators
        .op(Op::infix(Rule::SortExprFunction, Assoc::Right)) // $right 0
        .op(Op::infix(Rule::SortExprProduct, Assoc::Left)) // $left 1
});

#[allow(clippy::result_large_err)]
pub fn parse_sortexpr_primary(primary: Pair<'_, Rule>) -> ParseResult<SortExpression> {
    let span: Span = primary.as_span().into();
    match primary.as_rule() {
        Rule::IdAt => Ok(SortExpressionKind::Reference(Mcrl2Parser::IdAt(Node::new(primary))?.node).spanned(span)),
        Rule::SortExpr => Mcrl2Parser::SortExpr(Node::new(primary)),

        Rule::SortExprBool => Ok(SortExpressionKind::Simple(Sort::Bool).spanned(span)),
        Rule::SortExprInt => Ok(SortExpressionKind::Simple(Sort::Int).spanned(span)),
        Rule::SortExprPos => Ok(SortExpressionKind::Simple(Sort::Pos).spanned(span)),
        Rule::SortExprNat => Ok(SortExpressionKind::Simple(Sort::Nat).spanned(span)),
        Rule::SortExprReal => Ok(SortExpressionKind::Simple(Sort::Real).spanned(span)),

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

pub static DATAEXPR_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| {
    // Precedence is defined lowest to highest
    PrattParser::new()
        .op(Op::postfix(Rule::DataExprWhr)) // $left 0
        .op(Op::prefix(Rule::DataExprForall) | Op::prefix(Rule::DataExprExists) | Op::prefix(Rule::DataExprLambda)) // $right 1
        .op(Op::infix(Rule::DataExprImpl, Assoc::Right)) // $right 2
        .op(Op::infix(Rule::DataExprDisj, Assoc::Right)) // $right 3
        .op(Op::infix(Rule::DataExprConj, Assoc::Right)) // $right 4
        .op(Op::infix(Rule::DataExprEq, Assoc::Left) | Op::infix(Rule::DataExprNeq, Assoc::Left)) // $left 5
        .op(Op::infix(Rule::DataExprLess, Assoc::Left)
            | Op::infix(Rule::DataExprLeq, Assoc::Left)
            | Op::infix(Rule::DataExprGeq, Assoc::Left)
            | Op::infix(Rule::DataExprGreater, Assoc::Left)
            | Op::infix(Rule::DataExprIn, Assoc::Left)) // $left 6
        .op(Op::infix(Rule::DataExprCons, Assoc::Right)) // $right 7
        .op(Op::infix(Rule::DataExprSnoc, Assoc::Left)) // $left 8
        .op(Op::infix(Rule::DataExprConcat, Assoc::Left)) // $left 9
        .op(Op::infix(Rule::DataExprAdd, Assoc::Left) | Op::infix(Rule::DataExprSubtract, Assoc::Left)) // $left 10
        .op(Op::infix(Rule::DataExprDiv, Assoc::Left)
            | Op::infix(Rule::DataExprIntDiv, Assoc::Left)
            | Op::infix(Rule::DataExprMod, Assoc::Left)) // $left 11
        .op(Op::infix(Rule::DataExprMult, Assoc::Left)
            | Op::infix(Rule::DataExprAt, Assoc::Left) // $left 12
            | Op::prefix(Rule::DataExprMinus)
            | Op::prefix(Rule::DataExprNegation)
            | Op::prefix(Rule::DataExprSize)) // $right 12
        .op(Op::postfix(Rule::DataExprUpdate) | Op::postfix(Rule::DataExprApplication)) // ) // $left 13
});

#[allow(clippy::result_large_err)]
pub fn parse_dataexpr(pairs: Pairs<Rule>) -> ParseResult<DataExpr> {
    DATAEXPR_PRATT_PARSER
        .map_primary(|primary| {
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
                Rule::IdAt => Ok(DataExprKind::Id(Mcrl2Parser::IdAt(Node::new(primary))?.node).spanned(span)),

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
        })
        .map_infix(|lhs, op, rhs| {
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
        })
        .map_postfix(|expr, postfix| {
            let expr = expr?;
            let end = Span::from(postfix.as_span()).end;
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
        })
        .map_prefix(|prefix, expr| {
            let start = Span::from(prefix.as_span()).start;
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
        })
        .parse(pairs)
}

pub static PROCEXPR_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| {
    // Precedence is defined lowest to highest
    PrattParser::new()
        .op(Op::infix(Rule::ProcExprChoice, Assoc::Left)) // $left 1
        .op(Op::prefix(Rule::ProcExprSum) | Op::prefix(Rule::ProcExprDist)) // $right 2
        .op(Op::infix(Rule::ProcExprParallel, Assoc::Right)) // $right 3
        .op(Op::infix(Rule::ProcExprLeftMerge, Assoc::Right)) // $right 4
        .op(Op::prefix(Rule::ProcExprIfPrefix)) // $right 5
        .op(Op::prefix(Rule::ProcExprIfThen)) // $right 5
        .op(Op::infix(Rule::ProcExprUntil, Assoc::Left)) // $left 6
        .op(Op::infix(Rule::ProcExprSeq, Assoc::Right)) // $right 7
        .op(Op::postfix(Rule::ProcExprAt)) // $left 8
        .op(Op::infix(Rule::ProcExprSync, Assoc::Left)) // $left 9
});

#[allow(clippy::result_large_err)]
pub fn parse_process_expr(pairs: Pairs<Rule>) -> ParseResult<ProcessExpr> {
    PROCEXPR_PRATT_PARSER
        .map_primary(|primary| {
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
        })
        .map_infix(|lhs, op, rhs| {
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
        })
        .map_prefix(|prefix, expr| {
            let start = Span::from(prefix.as_span()).start;
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
                Rule::ProcExprIfPrefix => {
                    let (condition, then) = Mcrl2Parser::ProcExprIfPrefix(Node::new(prefix))?;

                    match then {
                        // No "<> else" tail: what follows this prefix operator in the Pratt
                        // chain is the "then" branch, and there is no "else" branch.
                        None => Ok(ProcessExprKind::Condition {
                            condition,
                            then: Box::new(expr),
                            else_: None,
                        }
                        .spanned(span)),
                        // A "<> else" tail was present: it supplied the "then" branch, and what
                        // follows this prefix operator in the Pratt chain is the "else" branch.
                        Some(then) => Ok(ProcessExprKind::Condition {
                            condition,
                            then: Box::new(then),
                            else_: Some(Box::new(expr)),
                        }
                        .spanned(span)),
                    }
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
        })
        .map_postfix(|expr, postfix| {
            let expr = expr?;
            let span = Span {
                start: expr.span.start,
                end: Span::from(postfix.as_span()).end,
            };
            match postfix.as_rule() {
                Rule::ProcExprAt => Ok(ProcessExprKind::At {
                    expr: Box::new(expr),
                    operand: Mcrl2Parser::ProcExprAt(Node::new(postfix))?,
                }
                .spanned(span)),
                _ => unimplemented!("Unexpected postfix rule: {:?}", postfix.as_rule()),
            }
        })
        .parse(pairs)
}

/// Defines the operator precedence for action formulas using a Pratt parser.
pub static ACTFRM_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| {
    // Precedence is defined lowest to highest
    PrattParser::new()
        .op(Op::prefix(Rule::ActFrmExists) | Op::prefix(Rule::ActFrmForall)) // $right  0
        .op(Op::infix(Rule::ActFrmImplies, Assoc::Right)) //  $right 2
        .op(Op::infix(Rule::ActFrmUnion, Assoc::Right)) // $right 3
        .op(Op::infix(Rule::ActFrmIntersect, Assoc::Right)) // $right 4
        .op(Op::postfix(Rule::ActFrmAt)) //  $left 5
        .op(Op::prefix(Rule::ActFrmNegation)) // $right 6
});

/// Parses a sequence of `Rule` pairs into an `ActFrm` using a Pratt parser defined in [ACTFRM_PRATT_PARSER] for operator precedence.
///
/// # Panics
///
/// Panics if `pairs` were not produced by the `ActFrm` grammar rule.
#[allow(clippy::result_large_err)]
pub fn parse_actfrm(pairs: Pairs<Rule>) -> ParseResult<ActFrm> {
    ACTFRM_PRATT_PARSER
        .map_primary(|primary| {
            let span: Span = primary.as_span().into();
            match primary.as_rule() {
                Rule::ActFrmTrue => Ok(ActFrmKind::True.spanned(span)),
                Rule::ActFrmFalse => Ok(ActFrmKind::False.spanned(span)),
                Rule::MultAct => Ok(ActFrmKind::MultAct(Mcrl2Parser::MultAct(Node::new(primary))?).spanned(span)),
                Rule::DataValExpr => {
                    Ok(ActFrmKind::DataExprVal(Mcrl2Parser::DataValExpr(Node::new(primary))?).spanned(span))
                }
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
        })
        .map_prefix(|prefix, expr| {
            let start = Span::from(prefix.as_span()).start;
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
        })
        .map_infix(|lhs, op, rhs| {
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
        })
        .parse(pairs)
}

/// Defines the operator precedence for regular expressions using a Pratt parser.
pub static REGFRM_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| {
    // Precedence is defined lowest to highest
    PrattParser::new()
        .op(Op::infix(Rule::RegFrmAlternative, Assoc::Left)) // $left 1
        .op(Op::infix(Rule::RegFrmComposition, Assoc::Right)) // $right 2
        .op(Op::postfix(Rule::RegFrmIteration) | Op::postfix(Rule::RegFrmPlus)) // $left 3
});

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
                end: Span::from(postfix.as_span()).end,
            };
            match postfix.as_rule() {
                Rule::RegFrmIteration => Ok(RegFrmKind::Iteration(Box::new(expr)).spanned(span)),
                Rule::RegFrmPlus => Ok(RegFrmKind::Plus(Box::new(expr)).spanned(span)),
                _ => unimplemented!("Unexpected rule: {:?}", postfix.as_rule()),
            }
        })
        .parse(pairs)
}

/// Defines the operator precedence for state formulas using a Pratt parser.
static STATEFRM_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| {
    // Precedence is defined lowest to highest
    PrattParser::new()
        .op(Op::prefix(Rule::StateFrmMu) | Op::prefix(Rule::StateFrmNu)) // $right 1
        .op(Op::prefix(Rule::StateFrmForall)
            | Op::prefix(Rule::StateFrmExists)
            | Op::prefix(Rule::StateFrmInf)
            | Op::prefix(Rule::StateFrmSup)
            | Op::prefix(Rule::StateFrmSum)) // $right 2
        .op(Op::infix(Rule::StateFrmAddition, Assoc::Left)) // $left 3
        .op(Op::infix(Rule::StateFrmImplication, Assoc::Right)) // $right 4
        .op(Op::infix(Rule::StateFrmDisjunction, Assoc::Right)) // $right 5
        .op(Op::infix(Rule::StateFrmConjunction, Assoc::Right)) // $right 6
        .op(Op::prefix(Rule::StateFrmLeftConstantMultiply) | Op::postfix(Rule::StateFrmRightConstantMultiply)) // $right 7
        .op(Op::prefix(Rule::StateFrmBox) | Op::prefix(Rule::StateFrmDiamond)) // $right 8
        .op(Op::prefix(Rule::StateFrmNegation) | Op::prefix(Rule::StateFrmUnaryMinus)) // $right 9
});

#[allow(clippy::result_large_err)]
pub fn parse_statefrm(pairs: Pairs<Rule>) -> ParseResult<StateFrm> {
    STATEFRM_PRATT_PARSER
        .map_primary(|primary| {
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
        })
        .map_prefix(|prefix, expr| {
            let start = Span::from(prefix.as_span()).start;
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
        })
        .map_infix(|lhs, op, rhs| {
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
        })
        .map_postfix(|expr, postfix| {
            let expr = expr?;
            let span = Span {
                start: expr.span.start,
                end: Span::from(postfix.as_span()).end,
            };
            match postfix.as_rule() {
                Rule::StateFrmRightConstantMultiply => Ok(StateFrmKind::DataValExprRightMult(
                    Box::new(expr),
                    Mcrl2Parser::StateFrmRightConstantMultiply(Node::new(postfix))?,
                )
                .spanned(span)),
                _ => unimplemented!("Unexpected binary operator: {:?}", postfix.as_rule()),
            }
        })
        .parse(pairs)
}

static PBESEXPR_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| {
    // Precedence is defined lowest to highest
    PrattParser::new()
        .op(Op::prefix(Rule::PbesExprForall) | Op::prefix(Rule::PbesExprExists)) // $right 0
        .op(Op::infix(Rule::PbesExprImplies, Assoc::Right)) // $right 2
        .op(Op::infix(Rule::PbesExprDisj, Assoc::Right)) // $right 3
        .op(Op::infix(Rule::PbesExprConj, Assoc::Right)) // $right 4
        .op(Op::prefix(Rule::PbesExprNegation)) // $right 5
});

#[allow(clippy::result_large_err)]
pub fn parse_pbesexpr(pairs: Pairs<Rule>) -> ParseResult<PbesExpr> {
    PBESEXPR_PRATT_PARSER
        .map_primary(|primary| {
            let span: Span = primary.as_span().into();
            match primary.as_rule() {
                Rule::DataValExpr => {
                    Ok(PbesExprKind::DataValExpr(Mcrl2Parser::DataValExpr(Node::new(primary))?).spanned(span))
                }
                Rule::PbesExprParens => {
                    // Handle parentheses by recursively parsing the inner expression
                    let inner = primary
                        .into_inner()
                        .next()
                        .expect("Expected inner expression in brackets");
                    parse_pbesexpr(inner.into_inner())
                }
                Rule::PbesExprTrue => Ok(PbesExprKind::True.spanned(span)),
                Rule::PbesExprFalse => Ok(PbesExprKind::False.spanned(span)),
                Rule::PropVarInst => {
                    Ok(PbesExprKind::PropVarInst(Mcrl2Parser::PropVarInst(Node::new(primary))?).spanned(span))
                }
                _ => unimplemented!("Unexpected rule: {:?}", primary.as_rule()),
            }
        })
        .map_prefix(|op, expr| {
            let start = Span::from(op.as_span()).start;
            let expr = expr?;
            let span = Span {
                start,
                end: expr.span.end,
            };
            match op.as_rule() {
                Rule::PbesExprNegation => Ok(PbesExprKind::Negation(Box::new(expr)).spanned(span)),
                Rule::PbesExprExists => Ok(PbesExprKind::Quantifier {
                    quantifier: Quantifier::Exists,
                    variables: Mcrl2Parser::PbesExprExists(Node::new(op))?,
                    body: Box::new(expr),
                }
                .spanned(span)),
                Rule::PbesExprForall => Ok(PbesExprKind::Quantifier {
                    quantifier: Quantifier::Forall,
                    variables: Mcrl2Parser::PbesExprForall(Node::new(op))?,
                    body: Box::new(expr),
                }
                .spanned(span)),
                _ => unimplemented!("Unexpected prefix operator: {:?}", op.as_rule()),
            }
        })
        .map_infix(|lhs, op, rhs| {
            let lhs = lhs?;
            let rhs = rhs?;
            let span = Span {
                start: lhs.span.start,
                end: rhs.span.end,
            };
            let op = match op.as_rule() {
                Rule::PbesExprConj => PbesExprBinaryOp::Conjunction,
                Rule::PbesExprDisj => PbesExprBinaryOp::Disjunction,
                Rule::PbesExprImplies => PbesExprBinaryOp::Implies,
                _ => unimplemented!("Unexpected binary operator: {:?}", op.as_rule()),
            };
            Ok(PbesExprKind::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            }
            .spanned(span))
        })
        .parse(pairs)
}

static PRESEXPR_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| {
    // Precedence is defined lowest to highest
    PrattParser::new()
        .op(Op::prefix(Rule::PresExprInf) | Op::prefix(Rule::PresExprSup) | Op::prefix(Rule::PresExprSum)) // $right 0
        .op(Op::infix(Rule::PresExprAdd, Assoc::Right)) // $right 2
        .op(Op::infix(Rule::PbesExprImplies, Assoc::Right)) // $right 3
        .op(Op::infix(Rule::PbesExprDisj, Assoc::Right)) // $right 4
        .op(Op::infix(Rule::PbesExprConj, Assoc::Right)) // $right 5
        .op(Op::prefix(Rule::PresExprLeftConstantMultiply) | Op::postfix(Rule::PresExprRightConstMultiply)) // $right 6
        .op(Op::prefix(Rule::PresExprNegation)) // $right 7
});

#[allow(clippy::result_large_err)]
pub fn parse_presexpr(pairs: Pairs<Rule>) -> ParseResult<PresExpr> {
    PRESEXPR_PRATT_PARSER
        .map_primary(|primary| {
            let span: Span = primary.as_span().into();
            match primary.as_rule() {
                Rule::DataValExpr => {
                    Ok(PresExprKind::DataValExpr(Mcrl2Parser::DataValExpr(Node::new(primary))?).spanned(span))
                }
                Rule::PresExprParens => {
                    // Handle parentheses by recursively parsing the inner expression
                    let inner = primary
                        .into_inner()
                        .next()
                        .expect("Expected inner expression in brackets");
                    parse_presexpr(inner.into_inner())
                }
                Rule::PbesExprTrue => Ok(PresExprKind::True.spanned(span)),
                Rule::PbesExprFalse => Ok(PresExprKind::False.spanned(span)),
                Rule::PropVarInst => {
                    Ok(PresExprKind::PropVarInst(Mcrl2Parser::PropVarInst(Node::new(primary))?).spanned(span))
                }
                Rule::PresExprEqinf => Ok(Mcrl2Parser::PresExprEqinf(Node::new(primary))?),
                Rule::PresExprEqninf => Ok(Mcrl2Parser::PresExprEqninf(Node::new(primary))?),
                Rule::PresExprCondsm => Ok(Mcrl2Parser::PresExprCondsm(Node::new(primary))?),
                Rule::PresExprCondeq => Ok(Mcrl2Parser::PresExprCondeq(Node::new(primary))?),
                _ => unimplemented!("Unexpected rule: {:?}", primary.as_rule()),
            }
        })
        .map_prefix(|op, expr| {
            let start = Span::from(op.as_span()).start;
            let expr = expr?;
            let span = Span {
                start,
                end: expr.span.end,
            };
            match op.as_rule() {
                Rule::PresExprNegation => Ok(PresExprKind::Negation(Box::new(expr)).spanned(span)),
                Rule::PresExprInf => Ok(PresExprKind::Bound {
                    op: Bound::Inf,
                    expr: Box::new(expr),
                    variables: Mcrl2Parser::PresExprInf(Node::new(op))?,
                }
                .spanned(span)),
                Rule::PresExprSup => Ok(PresExprKind::Bound {
                    op: Bound::Sup,
                    expr: Box::new(expr),
                    variables: Mcrl2Parser::PresExprSup(Node::new(op))?,
                }
                .spanned(span)),
                Rule::PresExprSum => Ok(PresExprKind::Bound {
                    op: Bound::Sum,
                    expr: Box::new(expr),
                    variables: Mcrl2Parser::PresExprSum(Node::new(op))?,
                }
                .spanned(span)),
                Rule::PresExprLeftConstantMultiply => Ok(PresExprKind::LeftConstantMultiply {
                    constant: Mcrl2Parser::PresExprLeftConstantMultiply(Node::new(op))?,
                    expr: Box::new(expr),
                }
                .spanned(span)),
                _ => unimplemented!("Unexpected prefix operator: {:?}", op.as_rule()),
            }
        })
        .map_infix(|lhs, op, rhs| {
            let lhs = lhs?;
            let rhs = rhs?;
            let span = Span {
                start: lhs.span.start,
                end: rhs.span.end,
            };
            let op = match op.as_rule() {
                Rule::PbesExprImplies => PresExprBinaryOp::Implies,
                Rule::PbesExprDisj => PresExprBinaryOp::Disjunction,
                Rule::PbesExprConj => PresExprBinaryOp::Conjunction,
                Rule::PresExprAdd => PresExprBinaryOp::Add,
                _ => unimplemented!("Unexpected binary operator: {:?}", op.as_rule()),
            };
            Ok(PresExprKind::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            }
            .spanned(span))
        })
        .map_postfix(|expr, postfix| {
            let expr = expr?;
            let span = Span {
                start: expr.span.start,
                end: Span::from(postfix.as_span()).end,
            };
            match postfix.as_rule() {
                Rule::PresExprRightConstMultiply => Ok(PresExprKind::RightConstantMultiply {
                    expr: Box::new(expr),
                    constant: Mcrl2Parser::PresExprRightConstMultiply(Node::new(postfix))?,
                }
                .spanned(span)),
                _ => unimplemented!("Unexpected postfix operator: {:?}", postfix.as_rule()),
            }
        })
        .parse(pairs)
}
