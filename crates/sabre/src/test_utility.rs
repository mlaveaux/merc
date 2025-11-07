#![forbid(unsafe_code)]

use ahash::AHashSet;
use mcrl3_aterm::ATerm;
use mcrl3_data::to_untyped_data_expression;
use mcrl3_utilities::MCRL3Error;

use crate::Rule;

/// Create a rewrite rule lhs -> rhs with the given names being variables.
pub fn create_rewrite_rule(lhs: &str, rhs: &str, variables: &[&str]) -> Result<Rule, MCRL3Error> {
    let lhs = ATerm::from_string(lhs)?;
    let rhs = ATerm::from_string(rhs)?;
    let mut vars = AHashSet::new();
    for var in variables {
        vars.insert(var.to_string());
    }

    Ok(Rule {
        conditions: vec![],
        lhs: to_untyped_data_expression(&lhs, Some(&vars)).into(),
        rhs: to_untyped_data_expression(&rhs, Some(&vars)).into(),
    })
}
