use std::fmt::Debug;
use std::time::Instant;

use clap::ValueEnum;

use merc_data::to_untyped_data_expression;
use merc_rec_tests::load_rec_from_file;
use merc_sabre::InnermostRewriter;
use merc_sabre::NaiveRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre::SabreRewriter;
use merc_utilities::MercError;

/// Selects the rewriter to use.
#[derive(ValueEnum, Debug, Clone)]
pub enum Rewriter {
    Naive,
    Innermost,
    Sabre,
}

/// Rewrites the given REC specification.
pub fn rewrite_rec(rewriter: Rewriter, filename_specification: &str, output: bool) -> Result<(), MercError> {
    let (syntax_spec, syntax_terms) = load_rec_from_file(filename_specification.into())?;

    let spec = syntax_spec.to_rewrite_spec();

    match rewriter {
        Rewriter::Naive => {
            let mut inner = NaiveRewriter::new(&spec);

            let now = Instant::now();
            for term in &syntax_terms {
                let term = to_untyped_data_expression(term.clone(), None);
                let result = inner.rewrite(&term);
                if output {
                    println!("{}", result)
                }
            }
            println!("Naive rewrite took {} ms", now.elapsed().as_millis());
        }
        Rewriter::Innermost => {
            let mut inner = InnermostRewriter::new(&spec);

            let now = Instant::now();
            for term in &syntax_terms {
                let term = to_untyped_data_expression(term.clone(), None);
                let result = inner.rewrite(&term);
                if output {
                    println!("{}", result)
                }
            }
            println!("Innermost rewrite took {} ms", now.elapsed().as_millis());
        }
        Rewriter::Sabre => {
            let mut sa = SabreRewriter::new(&spec);

            let now = Instant::now();
            for term in &syntax_terms {
                let term = to_untyped_data_expression(term.clone(), None);
                let result = sa.rewrite(&term);
                if output {
                    println!("{}", result)
                }
            }
            println!("Sabre rewrite took {} ms", now.elapsed().as_millis());
        }
    }

    Ok(())
}
