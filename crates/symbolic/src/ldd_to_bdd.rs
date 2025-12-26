use oxidd::BooleanFunction;
use oxidd::ManagerRef;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;

use merc_ldd::DataRef;
use merc_ldd::LddRef;
use merc_ldd::Storage;
use merc_ldd::height;
use merc_utilities::MercError;

pub fn ldd_to_bdd_simple(
    storage: &mut Storage,
    manager_ref: &BDDManagerRef,
    ldd: &LddRef<'_>,
) -> Result<BDDFunction, MercError> {
    let highest = compute_highest(storage, ldd);
    let bits = compute_bits(&highest);
    let bits_dd = merc_ldd::singleton(storage, &bits);

    ldd_to_bdd(storage, manager_ref, ldd, &bits_dd, 0)
}

/// Converts an LDD representing a set of vectors into a BDD representing the
/// same set by bitblasting the vector elements.
///
/// # Details
///
/// The bits should be a singleton LDD containing the result of
/// [`compute_bits`]. The conversion works recursively by processing the LDD
/// node by node, and introducing bits number of BDD variables for each layer in
/// the LDD. Note that `first_variable` indicates the level of the first
/// variable, and the next bits are placed at consecutive BDD layers. These
/// variables *must* already exist in the given BDD manager.
pub fn ldd_to_bdd(
    storage: &mut Storage,
    manager_ref: &BDDManagerRef,
    ldd: &LddRef<'_>,
    bits: &LddRef<'_>,
    first_variable: u32,
) -> Result<BDDFunction, MercError> {
    // Base cases
    if **storage.empty_set() == *ldd {
        return Ok(manager_ref.with_manager_shared(|manager| BDDFunction::f(manager)));
    }
    if **storage.empty_vector() == *ldd {
        return Ok(manager_ref.with_manager_shared(|manager| BDDFunction::t(manager)));
    }

    // TODO: Implement caching
    let DataRef(value, down, right) = storage.get_ref(ldd);
    let DataRef(bits_value, bits_down, _bits_right) = storage.get_ref(bits); // Is singleton so right is ignored.

    let right = ldd_to_bdd(storage, manager_ref, &right, bits, first_variable)?;
    let mut down = ldd_to_bdd(storage, manager_ref, &down, &bits_down, first_variable + bits_value)?;

    // Encode current value
    for i in 0..bits_value {
        // encode with high bit first
        let bit = bits_value - i - 1;
        if value & (1 << i) != 0 {
            // bit is 1
            down = manager_ref.with_manager_shared(|manager| {
                BDDFunction::var(manager, first_variable + bit)?.ite(&BDDFunction::f(manager), &down)
            })?;
        } else {
            // bit is 0
            down = manager_ref.with_manager_shared(|manager| {
                BDDFunction::var(manager, first_variable + bit)?.ite(&down, &BDDFunction::f(manager))
            })?;
        }
    }

    Ok(down.or(&right)?)
}

/// Computes the highest value for every layer in the LDD
fn compute_highest(storage: &mut Storage, ldd: &LddRef<'_>) -> Vec<u32> {
    let mut result = vec![0; height(storage, ldd)];
    compute_highest_rec(storage, &mut result, ldd, 0);
    result
}

/// Helper function for compute_highest
fn compute_highest_rec(storage: &mut Storage, result: &mut [u32], set: &LddRef<'_>, depth: usize) {
    if set == storage.empty_set() || set == storage.empty_vector() {
        return;
    }

    let DataRef(value, down, right) = storage.get_ref(set);
    compute_highest_rec(storage, result, &right, depth);
    compute_highest_rec(storage, result, &down, depth + 1);

    result[depth] = result[depth].max(value);
}

/// Computes the number of bits required to represent the highest value at each layer.
fn compute_bits(highest: &[u32]) -> Vec<u32> {
    highest.iter().map(|&h| u32::BITS - h.leading_zeros()).collect()
}

#[cfg(test)]
mod tests {
    use merc_ldd::fmt_node;
    use merc_ldd::from_iter;
    use merc_ldd::random_vector_set;
    use merc_ldd::singleton;
    use merc_utilities::random_test;

    use crate::create_variables;

    use super::*;

    #[test]
    fn test_random_compute_highest() {
        random_test(100, |rng| {
            let set = random_vector_set(rng, 4, 3, 5);
            let mut storage = Storage::new();
            let ldd = from_iter(&mut storage, set.iter());
            println!("LDD: {}", fmt_node(&storage, &ldd));

            let highest = compute_highest(&mut storage, &ldd);
            println!("Highest: {:?}", highest);
            for (i, h) in highest.iter().enumerate() {
                // Determine the highest value for every vector
                for value in set.iter() {
                    assert!(
                        *h >= value[i],
                        "The highest value for depth {} is {}, but vector has value {}",
                        i,
                        h,
                        value[i]
                    );
                }
            }

            let bits = compute_bits(&highest);
            println!("Bits: {:?}", bits);

            for (i, b) in bits.iter().enumerate() {
                let expected_bits = if highest[i] == 0 {
                    0
                } else {
                    u32::BITS - highest[i].leading_zeros()
                };
                assert_eq!(
                    *b, expected_bits,
                    "The number of bits for depth {} is {}, but expected {}",
                    i, b, expected_bits
                );
            }
        })
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_ldd_to_bdd() {
        random_test(100, |rng| {
            let set = random_vector_set(rng, 4, 3, 5);

            let mut storage = Storage::new();
            let ldd = from_iter(&mut storage, set.iter());
            println!("LDD: {}", fmt_node(&storage, &ldd));

            let highest = compute_highest(&mut storage, &ldd);
            let bits = compute_bits(&highest);
            let bits_dd = singleton(&mut storage, &bits);

            let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);

            let total_bits: u32 = bits.iter().sum();
            println!("Total bits: {}", total_bits);
            let _variables = create_variables(&manager_ref, total_bits).unwrap();

            let _bdd = ldd_to_bdd(&mut storage, &manager_ref, &ldd, &bits_dd, 0).unwrap();
        });
    }
}
