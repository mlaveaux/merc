//! Functions in this module are only relevant for testing purposes.

use rand::Rng;
use std::collections::HashSet;

use crate::Ldd;
use crate::Storage;
use crate::Value;
use crate::iterators::*;
use crate::operations::*;

/// Returns a vector of the given length with random u64 values (from 0..max_value).
pub fn random_vector(rng: &mut impl Rng, length: usize, max_value: Value) -> Vec<Value> {
    let mut vector: Vec<Value> = Vec::new();
    for _ in 0..length {
        vector.push(rng.random_range(0..max_value));
    }

    vector
}

/// Returns a sorted vector of the given length with unique u64 values (from 0..max_value).
pub fn random_sorted_vector(rng: &mut impl Rng, length: usize, max_value: Value) -> Vec<Value> {
    use rand::prelude::IteratorRandom;

    let mut result = (0..max_value).choose_multiple(rng, length);
    result.sort();
    result
}

/// Returns a set of 'amount' vectors where every vector has the given length.
pub fn random_vector_set(rng: &mut impl Rng, amount: usize, length: usize, max_value: Value) -> HashSet<Vec<Value>> {
    let mut result: HashSet<Vec<Value>> = HashSet::new();

    // Insert 'amount' number of vectors into the result.
    for _ in 0..amount {
        result.insert(random_vector(rng, length, max_value));
    }

    result
}

/// Returns an LDD containing all elements of the given iterator over vectors.
pub fn from_iter<'a, I>(storage: &mut Storage, iter: I) -> Ldd
where
    I: Iterator<Item = &'a Vec<Value>>,
{
    let mut result = storage.empty_set().clone();

    for vector in iter {
        let single = singleton(storage, vector);
        result = union(storage, &result, &single);
    }

    result
}

/// Prints vectors included in left, but not in right. Returns true iff the difference is non-empty.
pub fn print_left(storage: &Storage, left: &Ldd, right: &Ldd) -> bool {
    let mut result = true;

    for element in iter(storage, left) {
        if !element_of(storage, &element, right) {
            result = false;
            eprintln!("{:?}", element);
        }
    }

    result
}

/// Prints the differences in contained vectors between two LDDs.
pub fn print_differences(storage: &Storage, left: &Ldd, right: &Ldd) {
    // eprintln!("Vectors contained in {:?}, but not in {:?}:", left, right);
    print_left(storage, left, right);

    // eprintln!("Vectors contained in {}, but not in {}:", right, left);
    print_left(storage, right, left);
}

/// Returns project(vector, proj), see [project]. Requires proj to be sorted.
pub fn project_vector(vector: &[Value], proj: &[Value]) -> Vec<Value> {
    let mut result = Vec::<Value>::new();
    for i in proj {
        result.push(vector[*i as usize]);
    }
    result
}
