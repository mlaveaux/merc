use crate::BinaryOperator;
use crate::Data;
use crate::DataRef;
use crate::Ldd;
use crate::LddRef;
use crate::Storage;
use crate::TernaryOperator;
use crate::UnaryFunction;
use crate::Value;
use crate::cache_binary_op;
use crate::cache_comm_binary_op;
use crate::cache_terniary_op;
use crate::cache_unary_function;
use crate::iterators::*;

use std::cmp::Ordering;
use std::cmp::{self};

/// Returns an LDD containing only the given vector, i.e., { vector }.
pub fn singleton(storage: &mut Storage, vector: &[Value]) -> Ldd {
    let mut root = storage.empty_vector().clone();
    let empty_set = storage.empty_set().clone();
    for val in vector.iter().rev() {
        root = storage.insert(*val, &root, &empty_set);
    }

    root
}

/// Computes a meta LDD that is suitable for the [project] function from the
/// given projection indices.
///
/// This function is useful to be able to cache the projection LDD instead of
/// computing it from the projection array every time.
pub fn compute_proj(storage: &mut Storage, proj: &[Value]) -> Ldd {
    // Compute length of proj.
    let length = match proj.iter().max() {
        Some(x) => *x + 1,
        None => 0,
    };

    // Convert projection vectors to meta information.
    let mut result: Vec<Value> = Vec::new();
    for i in 0..length {
        let included = proj.contains(&i);

        if included {
            result.push(1);
        } else {
            result.push(0);
        }
    }

    singleton(storage, &result)
}

/// Computes the set of vectors projected onto the given indices, where proj is equal to compute_proj([i_0, ..., i_k]).
///
/// Formally, for a single vector <x_0, ..., x_n> we have that:
///     - project(<x_0, ..., x_n>, i_0 < ... < i_k) = <x_(i_0), ..., x_(i_k)>
///     - project(X, i_0 < ... < i_k) = { project(x, i_0 < ... < i_k) | x in X }.
///
/// Note that the indices are sorted in the definition, but compute_proj
/// can take any array and ignores both duplicates and order. Also, it
/// follows that i_k must be smaller than or equal to n as x_(i_k) is not
/// defined otherwise.
pub fn project(storage: &mut Storage, set: &LddRef, proj: &LddRef) -> Ldd {
    debug_assert_ne!(proj, storage.empty_set(), "proj must be a singleton");

    if proj == storage.empty_vector() {
        // If meta is not defined then the rest is not in the projection (proj is always zero)
        storage.empty_vector().clone()
    } else if set == storage.empty_set() {
        storage.empty_set().clone()
    } else {
        debug_assert_ne!(set, storage.empty_vector(), "proj can be at most as high as set");

        let DataRef(proj_value, proj_down, _) = storage.get_ref(proj);
        let DataRef(value, down, right) = storage.get_ref(set);

        match proj_value {
            0 => {
                let right_result = project(storage, &right, proj);
                let down_result = project(storage, &down, &proj_down);
                union(storage, &right_result, &down_result)
            }
            1 => {
                let right_result = project(storage, &right, proj);
                let down_result = project(storage, &down, &proj_down);
                if down_result == *storage.empty_set() {
                    right_result
                } else {
                    storage.insert(value, &down_result, &right_result)
                }
            }
            x => {
                panic!("proj has unexpected value {x}");
            }
        }
    }
}

/// Computes a meta LDD from the given read and write projections that is
/// suitable for [relational_product].
///
/// The read and write projections are arrays of indices that are read,
/// respectively written, by the corresponding sparse relation.
pub fn compute_meta(storage: &mut Storage, read_proj: &[Value], write_proj: &[Value]) -> Ldd {
    // Compute length of meta.
    let length = cmp::max(
        match read_proj.iter().max() {
            Some(x) => *x + 1,
            None => 0,
        },
        match write_proj.iter().max() {
            Some(x) => *x + 1,
            None => 0,
        },
    );

    // Convert projection vectors to meta.
    let mut meta: Vec<Value> = Vec::new();
    for i in 0..length {
        let read = read_proj.contains(&i);
        let write = write_proj.contains(&i);

        if read && write {
            meta.push(3);
            meta.push(4);
        } else if read {
            meta.push(1);
        } else if write {
            meta.push(2);
        } else {
            meta.push(0);
        }
    }

    singleton(storage, &meta)
}

/// Computes the set of vectors reachable in one step from the given set as defined by the sparse relation rel. Requires that meta = compute_meta(read_proj, write_proj).
///
/// # Details
///
/// Formal definition of the function. relational_product(R, S, read_proj, write_proj) = { x[write_proj := y'] | project(x, read_proj) = x' and (x', y') in R and x in S }
/// where R is the relation and S the set.
///  
/// meta is a singleton vector where the value indicates the following:
///   - 0 = not part the relation.
///   - 1 = only in read_proj.
///   - 2 = only in write_proj.
///   - 3 = in both read_proj and write_proj (read phase).
///   - 4 = in both read_proj and write_proj (write phase).
pub fn relational_product(storage: &mut Storage, set: &LddRef, rel: &LddRef, meta: &LddRef) -> Ldd {
    debug_assert_ne!(meta, storage.empty_set(), "proj must be a singleton");

    if meta == storage.empty_vector() {
        // If meta is not defined then the rest is not in the relation (meta is always zero)
        storage.protect(set)
    } else if set == storage.empty_set() || rel == storage.empty_set() {
        storage.empty_set().clone()
    } else {
        cache_terniary_op(
            storage,
            TernaryOperator::RelationalProduct,
            set,
            rel,
            meta,
            |storage, set, rel, meta| {
                let DataRef(meta_value, meta_down, _) = storage.get_ref(meta);

                match meta_value {
                    0 => {
                        // Consider all values on this level part of the output and continue with rest.
                        let DataRef(value, down, right) = storage.get_ref(set);

                        let right_result = relational_product(storage, &right, rel, meta);
                        let down_result = relational_product(storage, &down, rel, &meta_down);
                        if down_result == *storage.empty_set() {
                            right_result
                        } else {
                            storage.insert(value, &down_result, &right_result)
                        }
                    }
                    1 => {
                        // Read the values present in the relation and continue with these values in the set.
                        let DataRef(set_value, set_down, set_right) = storage.get_ref(set);
                        let DataRef(rel_value, rel_down, rel_right) = storage.get_ref(rel);

                        match set_value.cmp(&rel_value) {
                            Ordering::Less => relational_product(storage, &set_right, rel, meta),
                            Ordering::Equal => {
                                let down_result = relational_product(storage, &set_down, &rel_down, &meta_down);
                                let right_result = relational_product(storage, &set_right, &rel_right, meta);
                                if down_result == *storage.empty_set() {
                                    right_result
                                } else {
                                    storage.insert(set_value, &down_result, &right_result)
                                }
                            }
                            Ordering::Greater => relational_product(storage, set, &rel_right, meta),
                        }
                    }
                    2 => {
                        // All values in set should be considered.
                        let mut combined = storage.empty_set().clone();
                        let mut current = storage.protect(set);
                        loop {
                            let DataRef(_, set_down, set_right) = storage.get_ref(&current);
                            combined = union(storage, &combined, &set_down);

                            if set_right == *storage.empty_set() {
                                break;
                            }
                            current = storage.protect(&set_right);
                        }

                        // Write the values present in the relation.
                        let DataRef(rel_value, rel_down, rel_right) = storage.get_ref(rel);

                        let down_result = relational_product(storage, &combined, &rel_down, &meta_down);
                        let right_result = relational_product(storage, set, &rel_right, meta);
                        if down_result == *storage.empty_set() {
                            right_result
                        } else {
                            storage.insert(rel_value, &down_result, &right_result)
                        }
                    }
                    3 => {
                        let DataRef(set_value, set_down, set_right) = storage.get_ref(set);
                        let DataRef(rel_value, rel_down, rel_right) = storage.get_ref(rel);

                        match set_value.cmp(&rel_value) {
                            Ordering::Less => relational_product(storage, &set_right, rel, meta),
                            Ordering::Equal => {
                                let down_result = relational_product(storage, &set_down, &rel_down, &meta_down);
                                let right_result = relational_product(storage, &set_right, &rel_right, meta);
                                union(storage, &down_result, &right_result)
                            }
                            Ordering::Greater => relational_product(storage, set, &rel_right, meta),
                        }
                    }
                    4 => {
                        // Write the values present in the relation.
                        let DataRef(rel_value, rel_down, rel_right) = storage.get_ref(rel);

                        let down_result = relational_product(storage, set, &rel_down, &meta_down);
                        let right_result = relational_product(storage, set, &rel_right, meta);
                        if down_result == *storage.empty_set() {
                            right_result
                        } else {
                            storage.insert(rel_value, &down_result, &right_result)
                        }
                    }
                    x => {
                        panic!("meta has unexpected value: {x}");
                    }
                }
            },
        )
    }
}

/// Returns the largest subset of 'a' that does not contains elements of 'b', i.e., set difference.
pub fn minus(storage: &mut Storage, a: &LddRef, b: &LddRef) -> Ldd {
    if a == b || a == storage.empty_set() {
        storage.empty_set().clone()
    } else if b == storage.empty_set() {
        storage.protect(a)
    } else {
        cache_binary_op(storage, BinaryOperator::Minus, a, b, |storage, a, b| {
            let DataRef(a_value, a_down, a_right) = storage.get_ref(a);
            let DataRef(b_value, b_down, b_right) = storage.get_ref(b);

            match a_value.cmp(&b_value) {
                Ordering::Less => {
                    let right_result = minus(storage, &a_right, b);
                    storage.insert(a_value, &a_down, &right_result)
                }
                Ordering::Equal => {
                    let down_result = minus(storage, &a_down, &b_down);
                    let right_result = minus(storage, &a_right, &b_right);
                    if down_result == *storage.empty_set() {
                        right_result
                    } else {
                        storage.insert(a_value, &down_result, &right_result)
                    }
                }
                Ordering::Greater => minus(storage, a, &b_right),
            }
        })
    }
}

/// Returns the union of the given LDDs, i.e., a ∪ b.
pub fn union(storage: &mut Storage, a: &LddRef, b: &LddRef) -> Ldd {
    if a == b {
        storage.protect(a)
    } else if a == storage.empty_set() {
        storage.protect(b)
    } else if b == storage.empty_set() {
        storage.protect(a)
    } else {
        cache_comm_binary_op(storage, BinaryOperator::Union, a, b, |storage, a, b| {
            let DataRef(a_value, a_down, a_right) = storage.get_ref(a);
            let DataRef(b_value, b_down, b_right) = storage.get_ref(b);

            match a_value.cmp(&b_value) {
                Ordering::Less => {
                    let result = union(storage, &a_right, b);
                    storage.insert(a_value, &a_down, &result)
                }
                Ordering::Equal => {
                    let down_result = union(storage, &a_down, &b_down);
                    let right_result = union(storage, &a_right, &b_right);
                    storage.insert(a_value, &down_result, &right_result)
                }
                Ordering::Greater => {
                    let result = union(storage, a, &b_right);
                    storage.insert(b_value, &b_down, &result)
                }
            }
        })
    }
}

/// Interleave the vectors of two equal height ldds.
pub fn merge(storage: &mut Storage, a: &LddRef, b: &LddRef) -> Ldd {
    if a == storage.empty_vector() {
        storage.protect(b)
    } else if b == storage.empty_vector() {
        storage.protect(a)
    } else if a == storage.empty_set() || b == storage.empty_set() {
        storage.empty_set().clone()
    } else {
        cache_binary_op(storage, BinaryOperator::Merge, a, b, |storage, a, b| {
            let DataRef(value, down, right) = storage.get_ref(a);

            let down_result = merge(storage, b, &down);
            let right_result = merge(storage, &right, b);

            storage.insert(value, &down_result, &right_result)
        })
    }
}

/// Appends the given value to every vector in the set represented by the given ldd.
pub fn append(storage: &mut Storage, ldd: &LddRef, value: Value) -> Ldd {
    if ldd == storage.empty_set() {
        storage.empty_set().clone()
    } else if ldd == storage.empty_vector() {
        singleton(storage, &[value])
    } else {
        // Traverse the ldd.
        let DataRef(val, down, right) = storage.get_ref(ldd);

        let down_result = append(storage, &down, value);
        let right_result = append(storage, &right, value);

        storage.insert(val, &down_result, &right_result)
    }
}

/// Returns true iff the set contains the vector.
pub fn element_of(storage: &Storage, vector: &[Value], ldd: &Ldd) -> bool {
    if vector.is_empty() {
        ldd == storage.empty_vector()
    } else if ldd == storage.empty_vector() {
        false
    } else {
        for Data(value, down, _) in iter_right(storage, ldd) {
            match value.cmp(&vector[0]) {
                Ordering::Less => {
                    continue;
                }
                Ordering::Equal => {
                    return element_of(storage, &vector[1..], &down);
                }
                Ordering::Greater => {
                    return false;
                }
            }
        }

        false
    }
}

/// Returns the number of elements in the set.
pub fn len(storage: &mut Storage, set: &LddRef) -> usize {
    if set == storage.empty_set() {
        0
    } else if set == storage.empty_vector() {
        1
    } else {
        cache_unary_function(storage, UnaryFunction::Len, set, |storage, a| {
            let mut result: usize = 0;

            let mut current = storage.protect(a);
            while current != *storage.empty_set() {
                // Progress to the right LDD.
                let DataRef(_, down, right) = storage.get_ref(&current);
                result += len(storage, &down);
                current = storage.protect(&right);
            }

            result
        })
    }
}

/// Returns the height of the LDD tree.
pub fn height(storage: &Storage, ldd: &LddRef) -> usize {
    if ldd == storage.empty_set() || ldd == storage.empty_vector() {
        0
    } else {
        // Since all children have the same height we only have to look at the down node.
        let DataRef(_, down, _) = storage.get_ref(ldd);

        height(storage, &down) + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fmt_node;
    use crate::test_utility::*;

    use merc_utilities::random_test;
    use rand::Rng;
    use std::collections::HashSet;
    use std::ops::Sub;

    // Compare the LDD element_of implementation for random inputs.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_element_of() {
        random_test(100, |rng| {
            let mut storage = Storage::new();

            let length = 10;
            let set = random_vector_set(rng, 32, length, 10);
            let ldd = from_iter(&mut storage, set.iter());

            // All elements in the set should be contained in the ldd.
            for expected in &set {
                assert!(
                    element_of(&storage, expected, &ldd),
                    "Did not find expected vector in ldd"
                );
            }

            // No shorter vectors should be contained in the ldd (try several times).
            for _ in 0..10 {
                let len = rng.random_range(0..length);
                let short_vector = random_vector(rng, len, 10);
                assert!(
                    !element_of(&storage, &short_vector, &ldd),
                    "Found shorter vector in ldd."
                );
            }

            // No longer vectors should be contained in the ldd.
            for _ in 0..10 {
                let len = rng.random_range(length + 1..20);
                let short_vector = random_vector(rng, len, 10);
                assert!(!element_of(&storage, &short_vector, &ldd), "Found longer vector in ldd");
            }

            // Try vectors of correct size with both the set and ldd.
            for _ in 0..10 {
                let vector = random_vector(rng, length, 10);
                assert_eq!(
                    set.contains(&vector),
                    element_of(&storage, &vector, &ldd),
                    "Set contains did not match ldd element_of"
                );
            }
        });
    }

    // Compare the HashSet implementation of union with the LDD union implementation for random inputs.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_union() {
        random_test(100, |rng| {
            let mut storage = Storage::new();

            let set_a = random_vector_set(rng, 32, 10, 10);
            let set_b = random_vector_set(rng, 32, 10, 10);
            let expected = from_iter(&mut storage, set_a.union(&set_b));

            let a = from_iter(&mut storage, set_a.iter());
            let b = from_iter(&mut storage, set_b.iter());
            let result = union(&mut storage, &a, &b);

            assert_eq!(result, expected);
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_merge() {
        random_test(100, |rng| {
            let mut storage = Storage::new();

            let set_a = random_vector_set(rng, 32, 10, 10);
            let set_b = random_vector_set(rng, 32, 10, 10);

            // Compute the interleave explicitly.
            fn interleave(a: &[u32], b: &[u32]) -> Vec<u32> {
                let mut result = vec![];

                let mut iter = b.iter();
                for value in a {
                    result.push(*value);
                    result.push(*iter.next().unwrap());
                }

                result
            }

            let mut set_result = HashSet::<Vec<u32>>::new();
            for a in &set_a {
                for b in &set_b {
                    set_result.insert(interleave(a, b));
                }
            }

            let expected = from_iter(&mut storage, set_result.iter());

            let a = from_iter(&mut storage, set_a.iter());
            let b = from_iter(&mut storage, set_b.iter());
            let result: Ldd = merge(&mut storage, &a, &b);

            assert_eq!(result, expected);
        });
    }

    // Compare the singleton implementation with a random vector used as input.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_singleton() {
        random_test(100, |rng| {
            let mut storage = Storage::new();
            let vector = random_vector(rng, 10, 10);

            let ldd = singleton(&mut storage, &vector[..]);

            // Check that ldd contains exactly one vector that is equal to the initial vector.
            let mut it = iter(&storage, &ldd);
            let result = it.next().unwrap();
            assert_eq!(vector, result, "Contained vector did not match expected");
            assert_eq!(it.next(), None, "The ldd should not contain any other vector");
        });
    }

    // Test the len function with random inputs.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_len() {
        random_test(100, |rng| {
            let mut storage = Storage::new();

            let set = random_vector_set(rng, 32, 10, 10);
            let ldd = from_iter(&mut storage, set.iter());

            assert_eq!(set.len(), len(&mut storage, &ldd), "Length did not match expected set");
        });
    }

    // Test the minus function with random inputs.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_minus() {
        random_test(100, |rng| {
            let mut storage = Storage::new();

            let set_a = random_vector_set(rng, 32, 10, 10);
            let set_b = {
                let mut result = random_vector_set(rng, 32, 10, 10);

                // To ensure some overlap (which is unlikely) we insert some elements of a into b.
                let mut it = set_a.iter();
                for _ in 0..16 {
                    result.insert(it.next().unwrap().clone());
                }

                result
            };

            let expected_result = set_a.sub(&set_b);

            let a = from_iter(&mut storage, set_a.iter());
            let b = from_iter(&mut storage, set_b.iter());
            let result = minus(&mut storage, &a, &b);

            let expected = from_iter(&mut storage, expected_result.iter());

            println!("{}", fmt_node(&storage, &result));
            println!("{}", fmt_node(&storage, &expected));

            assert_eq!(result, expected);
        });
    }

    // Test the relational product function with read-only inputs.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_readonly_relational_product() {
        random_test(100, |rng| {
            let mut storage = Storage::new();
            let set = random_vector_set(rng, 32, 10, 10);

            let ldd = from_iter(&mut storage, set.iter());

            let read_proj = random_sorted_vector(rng, 4, 9);
            let meta = compute_meta(&mut storage, &read_proj, &[]);

            let proj_ldd = compute_proj(&mut storage, &read_proj);
            let relation = project(&mut storage, &ldd, &proj_ldd);

            let result = relational_product(&mut storage, &ldd, &relation, &meta);
            let read_project = project(&mut storage, &result, &proj_ldd);

            // relational_product(R, S, read_proj, []) = { x | project(x, read_proj) = x' and (x', <>) in R and x in S }
            assert_eq!(read_project, relation);
        });
    }

    // Test the relational product function with write-only inputs.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_writeonly_relational_product() {
        random_test(100, |rng| {
            let mut storage = Storage::new();
            let set = random_vector_set(rng, 32, 10, 10);

            let ldd = from_iter(&mut storage, set.iter());

            let write_proj = random_sorted_vector(rng, 4, 9);
            let meta = compute_meta(&mut storage, &[], &write_proj);

            let proj_ldd = compute_proj(&mut storage, &write_proj);
            let relation = project(&mut storage, &ldd, &proj_ldd);

            let result = relational_product(&mut storage, &ldd, &relation, &meta);
            let write_project = project(&mut storage, &result, &proj_ldd);

            // relational_product(R, S, [], write_proj) = { x[write_proj := y'] | (<>, y') in R and x in S }
            assert_eq!(write_project, relation);
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_relational_product() {
        random_test(100, |rng| {
            let mut storage = Storage::new();

            let set = random_vector_set(rng, 32, 10, 10);
            let relation = random_vector_set(rng, 32, 4, 10);

            // Pick arbitrary read and write parameters in order.
            let read_proj = random_sorted_vector(rng, 2, 9);
            let write_proj = random_sorted_vector(rng, 2, 9);

            // The indices of the input vectors do not match the indices in the relation. The input vector is defined for all values, but the relation only
            // for relevant positions.
            let (read_rel_proj, write_rel_proj) = {
                let mut read_rel_proj: Vec<Value> = Vec::new();
                let mut write_rel_proj: Vec<Value> = Vec::new();

                let mut current = 0;
                for i in 0..10 {
                    if read_proj.contains(&i) {
                        read_rel_proj.push(current);
                        current += 1;
                    }

                    if write_proj.contains(&i) {
                        write_rel_proj.push(current);
                        current += 1;
                    }
                }

                (read_rel_proj, write_rel_proj)
            };

            // Compute LDD result.
            let ldd = from_iter(&mut storage, set.iter());
            let rel = from_iter(&mut storage, relation.iter());

            let meta = compute_meta(&mut storage, &read_proj, &write_proj);
            let result = relational_product(&mut storage, &ldd, &rel, &meta);

            eprintln!("set = {}", fmt_node(&storage, &ldd));
            eprintln!("relation = {}", fmt_node(&storage, &rel));
            eprintln!("result = {}", fmt_node(&storage, &result));
            eprintln!("========");

            eprintln!("meta = {}", fmt_node(&storage, &meta));
            eprintln!(
                "read {:?}, write {:?}, read_rel {:?} and write_rel {:?}",
                read_proj, write_proj, read_rel_proj, write_rel_proj
            );

            let expected = {
                let mut expected: HashSet<Vec<Value>> = HashSet::new();

                // Compute relational_product(R, S, read_proj, write_proj) = { x[write_proj := y'] | project(x, read_proj) = x' and (x', y') in R and x in S }
                for x in set.iter() {
                    'next: for rel in relation.iter() {
                        let mut value = x.clone(); // The resulting vector.
                        let x_prime = project_vector(rel, &read_rel_proj);
                        let y_prime = project_vector(rel, &write_rel_proj);

                        // Ensure that project(x, read_proj) = x'
                        for (i, r) in read_proj.iter().enumerate() {
                            if value[*r as usize] != x_prime[i] {
                                continue 'next;
                            }
                        }

                        // Compute x[write_proj := y']
                        for (i, w) in write_proj.iter().enumerate() {
                            value[*w as usize] = y_prime[i];
                        }

                        // Print information about the value that we are testing.
                        eprintln!("value = {:?}, rel = {:?}", &value, &rel);
                        eprintln!("x_prime = {:?}, y_prime = {:?}", &x_prime, &y_prime);

                        assert!(
                            element_of(&storage, &value, &result),
                            "Result does not contain vector {:?}.",
                            &value
                        );
                        expected.insert(value);
                    }
                }

                expected
            };

            // Check the other way around
            for res in iter(&storage, &result) {
                assert!(
                    expected.contains(&res),
                    "Result unexpectedly contains vector {:?}.",
                    res
                );
            }
        });
    }

    // Test the project function with random inputs.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_project() {
        random_test(100, |rng| {
            let mut storage = Storage::new();

            let set = random_vector_set(rng, 32, 10, 10);
            let proj = random_sorted_vector(rng, 4, 9);

            let ldd = from_iter(&mut storage, set.iter());
            let proj_ldd = compute_proj(&mut storage, &proj);
            let result = project(&mut storage, &ldd, &proj_ldd);

            // Compute a naive projection on the vector set.
            let mut expected_result: HashSet<Vec<Value>> = HashSet::new();
            for element in &set {
                expected_result.insert(project_vector(element, &proj));
            }
            let expected = from_iter(&mut storage, expected_result.iter());
            assert_eq!(result, expected, "projected result does not match vector projection.");
        });
    }

    // Test the append function with random inputs.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_append() {
        random_test(100, |rng| {
            let mut storage = Storage::new();

            let set = random_vector_set(rng, 32, 10, 10);
            let ldd = from_iter(&mut storage, set.iter());
            let result = append(&mut storage, &ldd, 0);

            let mut expected_result: HashSet<Vec<Value>> = HashSet::new();
            for element in &set {
                let mut appended = element.to_vec();
                appended.push(0 as Value);
                expected_result.insert(appended);
            }
            let expected = from_iter(&mut storage, expected_result.iter());

            print_differences(&storage, &result, &expected);
            assert_eq!(result, expected, "appended result does not match vector append");
        });
    }
}
