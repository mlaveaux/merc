#![forbid(unsafe_code)]

use ahash::AHashSet;
use merc_aterm::ATerm;
use merc_data::to_untyped_data_expression;
use merc_utilities::MCRL3Error;

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
