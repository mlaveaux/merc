use std::sync::LazyLock;

use pest::iterators::Pair;
use pest::iterators::Pairs;
use pest::pratt_parser::Assoc;
use pest::pratt_parser::Assoc::Left;
use pest::pratt_parser::Assoc::Right;
use pest::pratt_parser::Op;
use pest::pratt_parser::PrattParser;

use merc_pest_consume::Node;

use crate::ActFrm;
use crate::ActFrmBinaryOp;
use crate::Bound;
use crate::DataExpr;
use crate::DataExprBinaryOp;
use crate::DataExprUnaryOp;
use crate::FixedPointOperator;
use crate::Mcrl2Parser;
use crate::ModalityOperator;
use crate::ParseResult;
use crate::PbesExpr;
use crate::PbesExprBinaryOp;
use crate::ProcExprBinaryOp;
use crate::ProcessExpr;
use crate::Quantifier;
use crate::RegFrm;
use crate::Rule;
use crate::Sort;
use crate::StateFrm;
use crate::StateFrmOp;
use crate::StateFrmUnaryOp;
use crate::syntax_tree::SortExpression;

pub static SORT_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| {
    // Precedence is defined lowest to highest
    PrattParser::new()
        // Sort operators
        .op(Op::infix(Rule::SortExprFunction, Left)) // $right 0
        .op(Op::infix(Rule::SortExprProduct, Right)) // $left 1
});

#[allow(clippy::result_large_err)]
pub fn parse_sortexpr_primary(primary: Pair<'_, Rule>) -> ParseResult<SortExpression> {
    match primary.as_rule() {
        Rule::Id => Ok(SortExpression::Reference(Mcrl2Parser::Id(Node::new(primary))?)),
        Rule::SortExpr => Mcrl2Parser::SortExpr(Node::new(primary)),

        Rule::SortExprBool => Ok(SortExpression::Simple(Sort::Bool)),
        Rule::SortExprInt => Ok(SortExpression::Simple(Sort::Int)),
        Rule::SortExprPos => Ok(SortExpression::Simple(Sort::Pos)),
        Rule::SortExprNat => Ok(SortExpression::Simple(Sort::Nat)),
        Rule::SortExprReal => Ok(SortExpression::Simple(Sort::Real)),

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
#[allow(clippy::result_large_err)]
pub fn parse_sortexpr(pairs: Pairs<Rule>) -> ParseResult<SortExpression> {
    SORT_PRATT_PARSER
        .map_primary(|primary| parse_sortexpr_primary(primary))
        .map_infix(|lhs, op, rhs| match op.as_rule() {
            Rule::SortExprFunction => Ok(SortExpression::Function {
                domain: Box::new(lhs?),
                range: Box::new(rhs?),
            }),
            Rule::SortExprProduct => Ok(SortExpression::Product {
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            _ => unimplemented!("Unexpected binary operator: {:?}", op.as_rule()),
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
        .map_primary(|primary| match primary.as_rule() {
            Rule::DataExprTrue => Ok(DataExpr::Bool(true)),
            Rule::DataExprFalse => Ok(DataExpr::Bool(false)),
            Rule::DataExprEmptyList => Ok(DataExpr::EmptyList),
            Rule::DataExprEmptySet => Ok(DataExpr::EmptySet),
            Rule::DataExprEmptyBag => Ok(DataExpr::EmptyBag),
            Rule::DataExprListEnum => Mcrl2Parser::DataExprListEnum(Node::new(primary)),
            Rule::DataExprBagEnum => Mcrl2Parser::DataExprBagEnum(Node::new(primary)),
            Rule::DataExprSetBagComp => Mcrl2Parser::DataExprSetBagComp(Node::new(primary)),
            Rule::DataExprSetEnum => Mcrl2Parser::DataExprSetEnum(Node::new(primary)),
            Rule::Number => Mcrl2Parser::Number(Node::new(primary)),
            Rule::Id => Ok(DataExpr::Id(Mcrl2Parser::Id(Node::new(primary))?)),

            Rule::DataExprBrackets => {
                // Handle parentheses by recursively parsing the inner expression
                let inner = primary
                    .into_inner()
                    .next()
                    .expect("Expected inner expression in brackets");
                parse_dataexpr(inner.into_inner())
            }

            _ => unimplemented!("Unexpected rule: {:?}", primary.as_rule()),
        })
        .map_infix(|lhs, op, rhs| {
            let op = match op.as_rule() {
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

            Ok(DataExpr::Binary {
                op,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            })
        })
        .map_postfix(|expr, postfix| match postfix.as_rule() {
            Rule::DataExprUpdate => Ok(DataExpr::FunctionUpdate {
                expr: Box::new(expr?),
                update: Box::new(Mcrl2Parser::DataExprUpdate(Node::new(postfix))?),
            }),
            Rule::DataExprApplication => Ok(DataExpr::Application {
                function: Box::new(expr?),
                arguments: Mcrl2Parser::DataExprApplication(Node::new(postfix))?,
            }),
            Rule::DataExprWhr => Ok(DataExpr::Whr {
                expr: Box::new(expr?),
                assignments: Mcrl2Parser::DataExprWhr(Node::new(postfix))?,
            }),
            _ => unimplemented!("Unexpected postfix operator: {:?}", postfix.as_rule()),
        })
        .map_prefix(|prefix, expr| match prefix.as_rule() {
            Rule::DataExprForall => Ok(DataExpr::Quantifier {
                op: Quantifier::Forall,
                variables: Mcrl2Parser::DataExprForall(Node::new(prefix))?,
                body: Box::new(expr?),
            }),
            Rule::DataExprExists => Ok(DataExpr::Quantifier {
                op: Quantifier::Exists,
                variables: Mcrl2Parser::DataExprExists(Node::new(prefix))?,
                body: Box::new(expr?),
            }),
            Rule::DataExprLambda => Ok(DataExpr::Lambda {
                variables: Mcrl2Parser::DataExprLambda(Node::new(prefix))?,
                body: Box::new(expr?),
            }),
            Rule::DataExprNegation => Ok(DataExpr::Unary {
                op: DataExprUnaryOp::Negation,
                expr: Box::new(expr?),
            }),
            Rule::DataExprMinus => Ok(DataExpr::Unary {
                op: DataExprUnaryOp::Minus,
                expr: Box::new(expr?),
            }),
            Rule::DataExprSize => Ok(DataExpr::Unary {
                op: DataExprUnaryOp::Size,
                expr: Box::new(expr?),
            }),
            _ => unimplemented!("Unexpected prefix operator: {:?}", prefix.as_rule()),
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
        .op(Op::prefix(Rule::ProcExprIf)) // $right 5
        .op(Op::prefix(Rule::ProcExprIfThen)) // $right 5
        .op(Op::infix(Rule::ProcExprUntil, Assoc::Left)) // $left 6
        .op(Op::infix(Rule::ProcExprSeq, Assoc::Right)) // $right 7
        .op(Op::postfix(Rule::ProcExprAt)) // $left 8
        .op(Op::infix(Rule::ProcExprSync, Assoc::Left)) // $left 9
});

#[allow(clippy::result_large_err)]
pub fn parse_process_expr(pairs: Pairs<Rule>) -> ParseResult<ProcessExpr> {
    PROCEXPR_PRATT_PARSER
        .map_primary(|primary| match primary.as_rule() {
            Rule::ProcExprId => Ok(Mcrl2Parser::ProcExprId(Node::new(primary))?),
            Rule::ProcExprDelta => Ok(ProcessExpr::Delta),
            Rule::ProcExprTau => Ok(ProcessExpr::Tau),
            Rule::ProcExprBlock => Ok(Mcrl2Parser::ProcExprBlock(Node::new(primary))?),
            Rule::ProcExprAllow => Ok(Mcrl2Parser::ProcExprAllow(Node::new(primary))?),
            Rule::ProcExprHide => Ok(Mcrl2Parser::ProcExprHide(Node::new(primary))?),
            Rule::ProcExprRename => Ok(Mcrl2Parser::ProcExprRename(Node::new(primary))?),
            Rule::ProcExprComm => Ok(Mcrl2Parser::ProcExprComm(Node::new(primary))?),
            Rule::Action => {
                let action = Mcrl2Parser::Action(Node::new(primary))?;

                Ok(ProcessExpr::Action(action.id, action.args))
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
        })
        .map_infix(|lhs, op, rhs| match op.as_rule() {
            Rule::ProcExprChoice => Ok(ProcessExpr::Binary {
                op: ProcExprBinaryOp::Choice,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            Rule::ProcExprParallel => Ok(ProcessExpr::Binary {
                op: ProcExprBinaryOp::Parallel,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            Rule::ProcExprLeftMerge => Ok(ProcessExpr::Binary {
                op: ProcExprBinaryOp::LeftMerge,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            Rule::ProcExprSeq => Ok(ProcessExpr::Binary {
                op: ProcExprBinaryOp::Sequence,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            Rule::ProcExprSync => Ok(ProcessExpr::Binary {
                op: ProcExprBinaryOp::CommMerge,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            _ => unimplemented!("Unexpected rule: {:?}", op.as_rule()),
        })
        .map_prefix(|prefix, expr| match prefix.as_rule() {
            Rule::ProcExprSum => Ok(ProcessExpr::Sum {
                variables: Mcrl2Parser::ProcExprSum(Node::new(prefix))?,
                operand: Box::new(expr?),
            }),
            Rule::ProcExprDist => {
                let (variables, data_expr) = Mcrl2Parser::ProcExprDist(Node::new(prefix))?;

                Ok(ProcessExpr::Dist {
                    variables,
                    expr: data_expr,
                    operand: Box::new(expr?),
                })
            }
            Rule::ProcExprIf => {
                let condition = Mcrl2Parser::ProcExprIf(Node::new(prefix))?;

                Ok(ProcessExpr::Condition {
                    condition,
                    then: Box::new(expr?),
                    else_: None,
                })
            }
            Rule::ProcExprIfThen => {
                let (condition, then) = Mcrl2Parser::ProcExprIfThen(Node::new(prefix))?;

                Ok(ProcessExpr::Condition {
                    condition,
                    then: Box::new(then),
                    else_: Some(Box::new(expr?)),
                })
            }
            _ => unimplemented!("Unexpected rule: {:?}", prefix.as_rule()),
        })
        .map_postfix(|expr, postfix| match postfix.as_rule() {
            Rule::ProcExprAt => Ok(ProcessExpr::At {
                expr: Box::new(expr?),
                operand: Mcrl2Parser::ProcExprAt(Node::new(postfix))?,
            }),
            _ => unimplemented!("Unexpected postfix rule: {:?}", postfix.as_rule()),
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
#[allow(clippy::result_large_err)]
pub fn parse_actfrm(pairs: Pairs<Rule>) -> ParseResult<ActFrm> {
    ACTFRM_PRATT_PARSER
        .map_primary(|primary| {
            match primary.as_rule() {
                Rule::ActFrmTrue => Ok(ActFrm::True),
                Rule::ActFrmFalse => Ok(ActFrm::False),
                Rule::MultAct => Ok(ActFrm::MultAct(Mcrl2Parser::MultAct(Node::new(primary))?)),
                Rule::DataValExpr => Ok(ActFrm::DataExprVal(Mcrl2Parser::DataValExpr(Node::new(primary))?)),
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
        .map_prefix(|prefix, expr| match prefix.as_rule() {
            Rule::ActFrmExists => Ok(ActFrm::Quantifier {
                quantifier: Quantifier::Exists,
                variables: Mcrl2Parser::ActFrmExists(Node::new(prefix))?,
                body: Box::new(expr?),
            }),
            Rule::ActFrmForall => Ok(ActFrm::Quantifier {
                quantifier: Quantifier::Forall,
                variables: Mcrl2Parser::ActFrmForall(Node::new(prefix))?,
                body: Box::new(expr?),
            }),
            Rule::ActFrmNegation => Ok(ActFrm::Negation(Box::new(expr?))),
            _ => unimplemented!("Unexpected prefix operator: {:?}", prefix.as_rule()),
        })
        .map_infix(|lhs, op, rhs| match op.as_rule() {
            Rule::ActFrmUnion => Ok(ActFrm::Binary {
                op: ActFrmBinaryOp::Union,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            Rule::ActFrmIntersect => Ok(ActFrm::Binary {
                op: ActFrmBinaryOp::Intersect,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            Rule::ActFrmImplies => Ok(ActFrm::Binary {
                op: ActFrmBinaryOp::Implies,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            _ => unimplemented!("Unexpected binary operator: {:?}", op.as_rule()),
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
#[allow(clippy::result_large_err)]
pub fn parse_regfrm(pairs: Pairs<Rule>) -> ParseResult<RegFrm> {
    REGFRM_PRATT_PARSER
        .map_primary(|primary| match primary.as_rule() {
            Rule::ActFrm => Ok(RegFrm::Action(Mcrl2Parser::ActFrm(Node::new(primary))?)),
            Rule::RegFrmBackets => {
                // Handle parentheses by recursively parsing the inner expression
                let inner = primary
                    .into_inner()
                    .next()
                    .expect("Expected inner expression in brackets");
                parse_regfrm(inner.into_inner())
            }
            _ => unimplemented!("Unexpected rule: {:?}", primary.as_rule()),
        })
        .map_infix(|lhs, op, rhs| match op.as_rule() {
            Rule::RegFrmAlternative => Ok(RegFrm::Choice {
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            Rule::RegFrmComposition => Ok(RegFrm::Sequence {
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            _ => unimplemented!("Unexpected binary operator: {:?}", op.as_rule()),
        })
        .map_postfix(|expr, postfix| match postfix.as_rule() {
            Rule::RegFrmIteration => Ok(RegFrm::Iteration(Box::new(expr?))),
            Rule::RegFrmPlus => Ok(RegFrm::Plus(Box::new(expr?))),
            _ => unimplemented!("Unexpected rule: {:?}", postfix.as_rule()),
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
            match primary.as_rule() {
                Rule::StateFrmId => Mcrl2Parser::StateFrmId(Node::new(primary)),
                Rule::StateFrmTrue => Ok(StateFrm::True),
                Rule::StateFrmFalse => Ok(StateFrm::False),
                Rule::StateFrmDelay => Mcrl2Parser::StateFrmDelay(Node::new(primary)),
                Rule::StateFrmYaled => Mcrl2Parser::StateFrmYaled(Node::new(primary)),
                Rule::StateFrmNegation => Mcrl2Parser::StateFrmNegation(Node::new(primary)),
                Rule::StateFrmDataValExpr => Ok(StateFrm::DataValExpr(Mcrl2Parser::DataValExpr(Node::new(primary))?)),
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
        .map_prefix(|prefix, expr| match prefix.as_rule() {
            Rule::StateFrmLeftConstantMultiply => Ok(StateFrm::DataValExprLeftMult(
                Mcrl2Parser::StateFrmLeftConstantMultiply(Node::new(prefix))?,
                Box::new(expr?),
            )),
            Rule::StateFrmDiamond => Ok(StateFrm::Modality {
                operator: ModalityOperator::Diamond,
                formula: Mcrl2Parser::StateFrmDiamond(Node::new(prefix))?,
                expr: Box::new(expr?),
            }),
            Rule::StateFrmBox => Ok(StateFrm::Modality {
                operator: ModalityOperator::Box,
                formula: Mcrl2Parser::StateFrmBox(Node::new(prefix))?,
                expr: Box::new(expr?),
            }),
            Rule::StateFrmExists => Ok(StateFrm::Quantifier {
                quantifier: Quantifier::Exists,
                variables: Mcrl2Parser::StateFrmExists(Node::new(prefix))?,
                body: Box::new(expr?),
            }),
            Rule::StateFrmForall => Ok(StateFrm::Quantifier {
                quantifier: Quantifier::Forall,
                variables: Mcrl2Parser::StateFrmForall(Node::new(prefix))?,
                body: Box::new(expr?),
            }),
            Rule::StateFrmMu => Ok(StateFrm::FixedPoint {
                operator: FixedPointOperator::Least,
                variable: Mcrl2Parser::StateFrmMu(Node::new(prefix))?,
                body: Box::new(expr?),
            }),
            Rule::StateFrmNu => Ok(StateFrm::FixedPoint {
                operator: FixedPointOperator::Greatest,
                variable: Mcrl2Parser::StateFrmNu(Node::new(prefix))?,
                body: Box::new(expr?),
            }),
            Rule::StateFrmNegation => Ok(StateFrm::Unary {
                op: StateFrmUnaryOp::Negation,
                expr: Box::new(expr?),
            }),
            Rule::StateFrmSup => Ok(StateFrm::Bound {
                bound: Bound::Sup,
                variables: Mcrl2Parser::StateFrmSup(Node::new(prefix))?,
                body: Box::new(expr?),
            }),
            Rule::StateFrmSum => Ok(StateFrm::Bound {
                bound: Bound::Sup,
                variables: Mcrl2Parser::StateFrmSum(Node::new(prefix))?,
                body: Box::new(expr?),
            }),
            Rule::StateFrmInf => Ok(StateFrm::Bound {
                bound: Bound::Sup,
                variables: Mcrl2Parser::StateFrmInf(Node::new(prefix))?,
                body: Box::new(expr?),
            }),
            _ => unimplemented!("Unexpected prefix operator: {:?}", prefix.as_rule()),
        })
        .map_infix(|lhs, op, rhs| match op.as_rule() {
            Rule::StateFrmAddition => Ok(StateFrm::Binary {
                op: StateFrmOp::Addition,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            Rule::StateFrmImplication => Ok(StateFrm::Binary {
                op: StateFrmOp::Implies,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            Rule::StateFrmDisjunction => Ok(StateFrm::Binary {
                op: StateFrmOp::Disjunction,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            Rule::StateFrmConjunction => Ok(StateFrm::Binary {
                op: StateFrmOp::Conjunction,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            _ => unimplemented!("Unexpected binary operator: {:?}", op.as_rule()),
        })
        .map_postfix(|expr, postfix| match postfix.as_rule() {
            Rule::StateFrmRightConstantMultiply => Ok(StateFrm::DataValExprRightMult(
                Box::new(expr?),
                Mcrl2Parser::StateFrmRightConstantMultiply(Node::new(postfix))?,
            )),
            _ => unimplemented!("Unexpected binary operator: {:?}", postfix.as_rule()),
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
            match primary.as_rule() {
                Rule::DataValExpr => Ok(PbesExpr::DataValExpr(Mcrl2Parser::DataValExpr(Node::new(primary))?)),
                Rule::PbesExprParens => {
                    // Handle parentheses by recursively parsing the inner expression
                    let inner = primary
                        .into_inner()
                        .next()
                        .expect("Expected inner expression in brackets");
                    parse_pbesexpr(inner.into_inner())
                }
                Rule::PbesExprTrue => Ok(PbesExpr::True),
                Rule::PbesExprFalse => Ok(PbesExpr::False),
                Rule::PropVarInst => Ok(PbesExpr::PropVarInst(Mcrl2Parser::PropVarInst(Node::new(primary))?)),
                _ => unimplemented!("Unexpected rule: {:?}", primary.as_rule()),
            }
        })
        .map_prefix(|op, expr| match op.as_rule() {
            Rule::PbesExprNegation => Ok(PbesExpr::Negation(Box::new(expr?))),
            _ => unimplemented!("Unexpected prefix operator: {:?}", op.as_rule()),
        })
        .map_infix(|lhs, op, rhs| match op.as_rule() {
            Rule::PbesExprConj => Ok(PbesExpr::Binary {
                op: PbesExprBinaryOp::Conjunction,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            Rule::PbesExprDisj => Ok(PbesExpr::Binary {
                op: PbesExprBinaryOp::Disjunction,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            Rule::PbesExprImplies => Ok(PbesExpr::Binary {
                op: PbesExprBinaryOp::Implies,
                lhs: Box::new(lhs?),
                rhs: Box::new(rhs?),
            }),
            _ => unimplemented!("Unexpected binary operator: {:?}", op.as_rule()),
        })
        .map_postfix(|expr, postfix| match postfix.as_rule() {
            Rule::PbesExprExists => Ok(PbesExpr::Quantifier {
                quantifier: Quantifier::Exists,
                variables: Mcrl2Parser::PbesExprExists(Node::new(postfix))?,
                body: Box::new(expr?),
            }),
            Rule::PbesExprForall => Ok(PbesExpr::Quantifier {
                quantifier: Quantifier::Forall,
                variables: Mcrl2Parser::PbesExprForall(Node::new(postfix))?,
                body: Box::new(expr?),
            }),
            _ => unimplemented!("Unexpected postfix operator: {:?}", postfix.as_rule()),
        })
        .parse(pairs)
}

static _PRESEXPR_PRATT_PARSER: LazyLock<PrattParser<Rule>> = LazyLock::new(|| {
    // Precedence is defined lowest to highest
    PrattParser::new()
        .op(Op::prefix(Rule::PresExprInf) | Op::prefix(Rule::PresExprSup) | Op::prefix(Rule::PresExprSum)) // $right 0
        .op(Op::infix(Rule::PresExprAdd, Assoc::Right)) // $right 2
        .op(Op::infix(Rule::PbesExprImplies, Assoc::Right)) // $right 3
        .op(Op::infix(Rule::PbesExprDisj, Assoc::Right)) // $right 4
        .op(Op::infix(Rule::PbesExprConj, Assoc::Right)) // $right 5
        .op(Op::prefix(Rule::PresExprLeftConstantMultiply) | Op::postfix(Rule::PresExprRightConstMultiply)) // $right 6
        .op(Op::prefix(Rule::PbesExprNegation)) // $right 7
});
