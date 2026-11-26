#![forbid(unsafe_code)]

#[cfg(test)]
use rand::Rng;
#[cfg(test)]
use rustc_hash::FxHashSet;

#[cfg(test)]
use crate::ATerm;
#[cfg(test)]
use crate::Symbol;
#[cfg(test)]
use crate::Term;
#[cfg(test)]
use crate::storage::THREAD_TERM_POOL;

/// Builds a random term over the given function symbols and constants by applying `iterations`
/// construction steps, each of which extends the pool of available subterms that later steps can
/// reuse as arguments.
///
/// # Panics
///
/// Panics if `constants` is empty, since at least one constant is needed to seed the pool of
/// subterms.
#[cfg(test)]
pub(crate) fn random_term<R: Rng>(
    rng: &mut R,
    symbols: &[(String, usize)],
    constants: &[String],
    iterations: usize,
) -> ATerm {
    use rand::prelude::IteratorRandom;

    debug_assert!(!constants.is_empty(), "We need constants to be able to create a term");

    let mut subterms = THREAD_TERM_POOL.with(|tp| {
        FxHashSet::<ATerm>::from_iter(constants.iter().map(|name| {
            let symbol = tp.create_symbol(name, 0);
            let a: &[ATerm] = &[];
            tp.create_term(&symbol, a).protect()
        }))
    });

    let mut result = None;
    for _ in 0..iterations {
        let (symbol, arity) = symbols.iter().choose(rng).expect("At least one symbol is available");

        let mut arguments = vec![];
        for _ in 0..*arity {
            arguments.push(
                subterms
                    .iter()
                    .choose(rng)
                    .expect("At least one subterm is available")
                    .clone(),
            );
        }

        let symbol = Symbol::new(symbol, *arity);
        let term = ATerm::with_args(&symbol, &arguments).protect();

        // Make this term available as another subterm that can be used.
        subterms.insert(term.clone());

        result = Some(term);
    }

    result.expect("At least one iteration was performed")
}
