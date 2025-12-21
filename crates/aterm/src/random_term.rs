#![forbid(unsafe_code)]

use rand::Rng;
use rustc_hash::FxHashSet;

use crate::ATerm;
use crate::Symbol;
use crate::Term;
use crate::storage::THREAD_TERM_POOL;

/// Create a random term consisting of the given symbol and constants. Performs
/// iterations number of constructions, and uses chance_duplicates to choose the
/// amount of subterms that are duplicated.
pub fn random_term(rng: &mut impl Rng, symbols: &[(String, usize)], constants: &[String], iterations: usize) -> ATerm {
    use rand::prelude::IteratorRandom;

    debug_assert!(!constants.is_empty(), "We need constants to be able to create a term");

    let mut subterms = THREAD_TERM_POOL.with_borrow(|tp| {
        FxHashSet::<ATerm>::from_iter(constants.iter().map(|name| {
            let symbol = tp.create_symbol(name, 0);
            let a: &[ATerm] = &[];
            tp.create_term(&symbol, a).protect()
        }))
    });

    let mut result = None;
    for _ in 0..iterations {
        let (symbol, arity) = symbols.iter().choose(rng).unwrap();

        let mut arguments = vec![];
        for _ in 0..*arity {
            arguments.push(subterms.iter().choose(rng).unwrap().clone());
        }

        let symbol = Symbol::new(symbol, *arity);
        let term = ATerm::with_args(&symbol, &arguments).protect();

        // Make this term available as another subterm that can be used.
        subterms.insert(term.clone());

        result = Some(term);
    }

    result.expect("At least one iteration was performed")
}
