use std::collections::HashSet;

use crate::{Symbol, atermpp::aterm::ATerm};

/// Create a random term consisting of the given symbol and constants. Performs
/// iterations number of constructions, and uses chance_duplicates to choose the
/// amount of subterms that are duplicated.
pub fn random_term(
    rng: &mut impl rand::Rng,
    symbols: &[(String, usize)],
    constants: &[String],
    iterations: usize,
) -> ATerm {
    use rand::prelude::IteratorRandom;

    debug_assert!(!constants.is_empty(), "We need constants to be able to create a term");

    let mut subterms = HashSet::<ATerm>::from_iter(constants.iter().map(|name| {
        ATerm::new(&Symbol::new(name, 0), &[] as &[ATerm])
    }));

    let mut result = ATerm::default();
    for _ in 0..iterations {
        let (symbol, arity) = symbols.iter().choose(rng).unwrap();

        let mut arguments = vec![];
        for _ in 0..*arity {
            arguments.push(subterms.iter().choose(rng).unwrap().clone());
        }

        result = ATerm::new(&Symbol::new(symbol, *arity), &arguments);

        // Make this term available as another subterm that can be used.
        subterms.insert(result.clone());
    }

    result
}
