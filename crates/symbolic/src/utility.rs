use merc_ldd::Ldd;
use merc_ldd::Storage;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;

fn ldd_to_bdd(storage: &mut Storage, _manager: BDDManagerRef, input: &Ldd, _bits: &Ldd, _first_variable: usize) -> BDDFunction {
    if storage.empty_set() == input {
        // return manager.::false_function();
    }

    unimplemented!();
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
mod tests {}
