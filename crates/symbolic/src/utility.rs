use merc_ldd::DataRef;
use merc_ldd::LddRef;
use merc_ldd::height;
use merc_utilities::MercError;
use oxidd::BooleanFunction;
use oxidd::ManagerRef;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;

use merc_ldd::Ldd;
use merc_ldd::Storage;

/// Converts an LDD representing a set of vectors into a BDD representing the same set by bitblasting the vector elements.
fn ldd_to_bdd(
    storage: &mut Storage,
    manager_ref: &BDDManagerRef,
    ldd: &LddRef<'_>,
    bits: &LddRef<'_>,
    first_variable: u32,
) -> Result<BDDFunction, MercError> {
    // Base cases
    if **storage.empty_set() == *ldd {
        return Ok(manager_ref.with_manager_shared(|manager| BDDFunction::t(manager)));
    }
    if **storage.empty_vector() == *ldd {
        return Ok(manager_ref.with_manager_shared(|manager| BDDFunction::f(manager)));
    }

    // TODO: Implement caching    
    let DataRef(value, right, down) = storage.get_ref(ldd);
    let DataRef(bits_value, bits_right, bits_down) = storage.get_ref(bits);

    let mut right = ldd_to_bdd(storage, manager_ref, &right, &bits, first_variable)?;
    let mut down = ldd_to_bdd(storage, manager_ref, &down, &bits_down, first_variable + 2 * bits_value)?;

    // Encode current value
    for i in 0..bits_value {
        // encode with high bit first
        let bit = bits_value - i - 1;
        if value & (1 << i) != 0 {
            // bit is 1
            // down = manager_ref.with_manager_shared(|manager| {
            //     manager.var(first_variable + 2*bit)., storage.empty_set(), &down);
            // });
        } else {
            // bit is 0
            // down = storage.insert(first_variable + 2*bit, &down, storage.empty_set());
        }
    }

    Ok(down.or(&right)?)
}

/// Computes the highest value for every layer in the LDD
fn compute_highest(storage: &mut Storage, ldd: &Ldd) -> Vec<usize> {
    let mut result = vec![0; height(storage, ldd)];
    compute_highest_rec(storage, &mut result, ldd, 0);
    result
}

/// Helper function for compute_highest
fn compute_highest_rec(storage: &mut Storage, result: &mut Vec<usize>, set: &LddRef<'_>, index: usize) {
    if set == storage.empty_set() || set == storage.empty_vector() {
        return;
    }

    let DataRef(value, right, down) = storage.get_ref(set);
    compute_highest_rec(storage, result, &right, index);
    compute_highest_rec(storage, result, &down, index + 1);

    result[index] = result[index].max(value as usize + 1);
}

/// Computes the number of bits required to represent each element in the vector
fn compute_bits(highest: &Vec<usize>) -> Vec<usize> {
    highest
        .iter()
        .map(|&h| (usize::BITS - h.leading_zeros()) as usize)
        .collect()
}

#[cfg(test)]
mod tests {
    use merc_ldd::{from_iter, random_vector_set};
    use merc_utilities::random_test;

    use super::*;

    #[test]
    fn test_random_ldd_to_bdd() {

        random_test(100, |rng| {
            let set = random_vector_set(rng, 4, 3, 5);

            let mut storage = Storage::new();
            let ldd = from_iter(&mut storage, set.iter());

            let highest = compute_highest(&mut storage, &ldd);
            for (i, h) in highest.iter().enumerate() {

                // Determine the heighest value for every vector
                for value in set.iter() {
                    assert!(*h > value[i] as usize, "The highest value for layer {} is {}, but vector has value {}", i, h, value[i]);
                }
            }
        })        
    }
}
