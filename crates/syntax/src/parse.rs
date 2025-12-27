use pest::Parser;
use pest_derive::Parser;

use merc_pest_consume::Error;
use merc_utilities::MercError;

use crate::DataExpr;
use crate::DataExprBinaryOp;
use crate::MultiAction;
use crate::ParseNode;
use crate::StateFrmOp;
use crate::UntypedActionRenameSpec;
use crate::UntypedDataSpecification;
use crate::UntypedPbes;
use crate::UntypedProcessSpecification;
use crate::UntypedStateFrmSpec;

#[derive(Parser)]
#[grammar = "mcrl2_grammar.pest"]
pub struct Mcrl2Parser;

/// Parses the given mCRL2 specification into an AST.
impl UntypedProcessSpecification {
    pub fn parse(spec: &str) -> Result<UntypedProcessSpecification, MercError> {
        let mut result = Mcrl2Parser::parse(Rule::MCRL2Spec, spec).map_err(extend_parser_error)?;
        let root = result.next().expect("Could not parse mCRL2 specification");
        Ok(Mcrl2Parser::MCRL2Spec(ParseNode::new(root))?)
    }
}

/// Parses the given mCRL2 specification into an AST.
impl UntypedDataSpecification {
    pub fn parse(spec: &str) -> Result<UntypedDataSpecification, MercError> {
        let mut result = Mcrl2Parser::parse(Rule::DataSpec, spec).map_err(extend_parser_error)?;
        let root = result.next().expect("Could not parse mCRL2 data specification");

        Ok(Mcrl2Parser::DataSpec(ParseNode::new(root))?)
    }
}

impl DataExpr {
    pub fn parse(spec: &str) -> Result<DataExpr, MercError> {
        let mut result = Mcrl2Parser::parse(Rule::DataExpr, spec).map_err(extend_parser_error)?;
        let root = result.next().expect("Could not parse mCRL2 data expression");

        Ok(Mcrl2Parser::DataExpr(ParseNode::new(root))?)
    }
}

impl MultiAction {
    pub fn parse(spec: &str) -> Result<MultiAction, MercError> {
        let mut result = Mcrl2Parser::parse(Rule::MultAct, spec).map_err(extend_parser_error)?;
        let root = result.next().expect("Could not parse mCRL2 multi-action");

        Ok(Mcrl2Parser::MultAct(ParseNode::new(root))?)
    }
}

impl UntypedStateFrmSpec {
    pub fn parse(spec: &str) -> Result<UntypedStateFrmSpec, MercError> {
        let mut result = Mcrl2Parser::parse(Rule::StateFrmSpec, spec).map_err(extend_parser_error)?;
        let root = result
            .next()
            .expect("Could not parse mCRL2 state formula specification");

        Ok(Mcrl2Parser::StateFrmSpec(ParseNode::new(root))?)
    }
}

impl UntypedActionRenameSpec {
    pub fn parse(spec: &str) -> Result<UntypedActionRenameSpec, MercError> {
        let mut result = Mcrl2Parser::parse(Rule::ActionRenameSpec, spec).map_err(extend_parser_error)?;
        let root = result
            .next()
            .expect("Could not parse mCRL2 action rename specification");

        Ok(Mcrl2Parser::ActionRenameSpec(ParseNode::new(root))?)
    }
}

impl UntypedPbes {
    pub fn parse(spec: &str) -> Result<UntypedPbes, MercError> {
        let mut result = Mcrl2Parser::parse(Rule::PbesSpec, spec).map_err(extend_parser_error)?;
        let root = result
            .next()
            .expect("Could not parse parameterised boolean equation system");

        Ok(Mcrl2Parser::PbesSpec(ParseNode::new(root))?)
    }
}

fn extend_parser_error(error: Error<Rule>) -> Error<Rule> {
    error.renamed_rules(|rule| match rule {
        Rule::DataExprWhr => "DataExpr whr AssignmentList end".to_string(),
        Rule::DataExprUpdate => "DataExpr[(DataExpr -> DataExpr)*]".to_string(),
        Rule::DataExprApplication => "DataExpr(DataExpr*)".to_string(),

        // DataExpr Binary Operators
        Rule::DataExprConj => format!("{}", DataExprBinaryOp::Conj),
        Rule::DataExprDisj => format!("{}", DataExprBinaryOp::Disj),
        Rule::DataExprImpl => format!("{}", DataExprBinaryOp::Implies),
        Rule::DataExprEq => format!("{}", DataExprBinaryOp::Equal),
        Rule::DataExprNeq => format!("{}", DataExprBinaryOp::NotEqual),
        Rule::DataExprLess => format!("{}", DataExprBinaryOp::LessThan),
        Rule::DataExprLeq => format!("{}", DataExprBinaryOp::LessEqual),
        Rule::DataExprGreater => format!("{}", DataExprBinaryOp::GreaterThan),
        Rule::DataExprGeq => format!("{}", DataExprBinaryOp::GreaterEqual),
        Rule::DataExprIn => format!("{}", DataExprBinaryOp::In),
        Rule::DataExprDiv => format!("{}", DataExprBinaryOp::Div),
        Rule::DataExprIntDiv => format!("{}", DataExprBinaryOp::IntDiv),
        Rule::DataExprMod => format!("{}", DataExprBinaryOp::Mod),
        Rule::DataExprMult => format!("{}", DataExprBinaryOp::Multiply),
        Rule::DataExprAdd => format!("{}", DataExprBinaryOp::Add),
        Rule::DataExprSubtract => format!("{}", DataExprBinaryOp::Subtract),
        Rule::DataExprAt => format!("{}", DataExprBinaryOp::At),
        Rule::DataExprCons => format!("{}", DataExprBinaryOp::Cons),
        Rule::DataExprSnoc => format!("{}", DataExprBinaryOp::Snoc),
        Rule::DataExprConcat => format!("{}", DataExprBinaryOp::Concat),

        // Regular Formulas
        Rule::RegFrmAlternative => "RegFrm + RegFrm".to_string(),
        Rule::RegFrmComposition => "RegFrm . RegFrm".to_string(),
        Rule::RegFrmIteration => "RegFrm*".to_string(),
        Rule::RegFrmPlus => "RegFrm+".to_string(),

        // State formulas
        Rule::StateFrmAddition => format!("{}", StateFrmOp::Addition),
        Rule::StateFrmLeftConstantMultiply => "Number * StateFrm".to_string(),
        Rule::StateFrmImplication => format!("{}", StateFrmOp::Implies),
        Rule::StateFrmDisjunction => format!("{}", StateFrmOp::Disjunction),
        Rule::StateFrmConjunction => format!("{}", StateFrmOp::Conjunction),
        Rule::StateFrmRightConstantMultiply => "StateFrm * Number".to_string(),
        _ => format!("{rule:?}"),
    })
}
