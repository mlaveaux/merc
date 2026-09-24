use std::cmp::Ordering;

use oxidd::ManagerRef;
use oxidd::ldd::LDDFunction;
use oxidd::ldd::LDDManagerRef;
use oxidd::ldd::Value;
use oxidd::util::AllocResult;
use rustc_hash::FxHashMap;

use crate::height;

/// Computes the interleaved cartesian product `{ a₀b₀a₁b₁… | a ∈ set_a, b ∈ set_b }`.
///
/// # Details
///
/// Used to build a strategy over the doubled, interleaved state vector `[from₀,
/// to₀, from₁, to₁, …]` out of two vector sets over the plain (non-interleaved)
/// vector. The recursion is:
///
/// > `merge(⊤, b) = b`
/// > `merge(a, ⊤) = a`
/// > `merge(∅, _) = merge(_, ∅) = ∅`
/// > `merge(a, b) = node(a.value, merge(b, a.down), merge(a.right, b))`, otherwise
///
/// where `⊤` is the singleton set containing only the empty vector and `∅` is
/// the empty set. Note the down-branch swaps `a` and `b`: each level of the
/// result takes its value from whichever operand is "in front", which is what
/// produces the interleaving.
///
/// [`LDDFunction::make_node`] does not apply the LDD reduction rule (unlike
/// Sylvan's `lddmc_makenode`), so this function must apply it itself: a node
/// whose `down` branch became empty is not a valid node and collapses to its
/// `right` sibling.
pub fn merge(manager: &LDDManagerRef, a: &LDDFunction, b: &LDDFunction) -> AllocResult<LDDFunction> {
    let mut cache = FxHashMap::default();
    merge_rec(manager, a, b, &mut cache)
}

fn merge_rec(
    manager: &LDDManagerRef,
    a: &LDDFunction,
    b: &LDDFunction,
    cache: &mut FxHashMap<(usize, usize), LDDFunction>,
) -> AllocResult<LDDFunction> {
    if a.is_empty_vector() {
        return Ok(b.clone());
    }
    if b.is_empty_vector() {
        return Ok(a.clone());
    }
    if a.is_empty() || b.is_empty() {
        return manager.with_manager_shared(LDDFunction::empty_set);
    }

    let key = (a.id(), b.id());
    if let Some(cached) = cache.get(&key) {
        return Ok(cached.clone());
    }

    let (value, down, right) = a
        .node()
        .expect("checked above: neither the empty set nor the empty vector");

    let down_result = merge_rec(manager, b, &down, cache)?;
    let right_result = merge_rec(manager, &right, b, cache)?;

    let result = if down_result.is_empty() {
        // A node with an empty `down` branch is not canonical; it collapses to `right`.
        right_result
    } else {
        manager.with_manager_shared(|m| LDDFunction::make_node(m, value, &down_result, &right_result))?
    };

    cache.insert(key, result.clone());
    Ok(result)
}

/// Restricts `set` to the vectors whose element at `level` equals `value`.
///
/// `level` must be less than the length of every vector in `set` (checked once, up front, in
/// debug builds); every caller derives it from a valid position in the state vector, so this
/// should never trip in practice.
pub fn fix_element(manager: &LDDManagerRef, set: &LDDFunction, level: usize, value: Value) -> AllocResult<LDDFunction> {
    debug_assert!(
        set.is_empty() || level < height(manager, set),
        "fix_element: level {level} is out of range for a set of height {}",
        height(manager, set)
    );
    fix_element_rec(manager, set, level, value)
}

fn fix_element_rec(manager: &LDDManagerRef, set: &LDDFunction, level: usize, value: Value) -> AllocResult<LDDFunction> {
    if level == 0 {
        let mut current = set.clone();
        loop {
            match current.node() {
                None => return manager.with_manager_shared(LDDFunction::empty_set),
                Some((v, down, right)) => match v.cmp(&value) {
                    Ordering::Equal => {
                        let empty = manager.with_manager_shared(LDDFunction::empty_set)?;
                        return manager.with_manager_shared(|m| LDDFunction::make_node(m, value, &down, &empty));
                    }
                    Ordering::Greater => return manager.with_manager_shared(LDDFunction::empty_set),
                    Ordering::Less => current = right,
                },
            }
        }
    } else {
        // Walk the `right` sibling chain iteratively instead of recursing on it.
        let mut kept = Vec::new();
        let mut current = set.clone();
        loop {
            match current.node() {
                // `set` ran out of levels before reaching `level` (it is ∅, or `level` was past the
                // end of its vectors): there is no element at `level` to match, so the result is ∅.
                None => break,
                Some((v, down, right)) => {
                    let new_down = fix_element_rec(manager, &down, level - 1, value)?;
                    if !new_down.is_empty() {
                        kept.push((v, new_down));
                    }
                    current = right;
                }
            }
        }

        let mut result = manager.with_manager_shared(LDDFunction::empty_set)?;
        for (v, new_down) in kept.into_iter().rev() {
            result = manager.with_manager_shared(|m| LDDFunction::make_node(m, v, &new_down, &result))?;
        }
        Ok(result)
    }
}

/// Returns an LDD containing all elements of the given iterator over vectors.
pub fn from_iter<'a, I>(manager: &LDDManagerRef, iter: I) -> LDDFunction
where
    I: Iterator<Item = &'a Vec<Value>>,
{
    let mut result = manager
        .with_manager_shared(|m| LDDFunction::empty_set(m))
        .expect("Failed to create the empty set");

    for vector in iter {
        let single = manager
            .with_manager_shared(|m| LDDFunction::singleton(m, vector))
            .expect("Failed to create a singleton");
        result = result.union(&single).expect("Failed to compute the union");
    }

    result
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use oxidd::ManagerRef;
    use oxidd::ldd::LDDFunction;

    use merc_utilities::random_test;

    use crate::LDD_CACHE_CAPACITY;
    use crate::LDD_NODE_CAPACITY;
    use crate::LddLenCache;
    use crate::from_iter;
    use crate::ldd_len;
    use crate::random_vector_set;

    use super::fix_element;
    use super::merge;

    /// The number of vectors in `ldd`, which is small enough for every test in this module to be exact.
    fn count(ldd: &LDDFunction) -> usize {
        ldd_len(ldd, &mut LddLenCache::new())
            .exact()
            .expect("the count of a test set is exact") as usize
    }

    /// Cross-checks [`LDDFunction::intersect`] against a `HashSet` reference implementation.
    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_intersect() {
        random_test(100, |rng| {
            let manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);

            let a = random_vector_set(rng, 8, 3, 5);
            let b = random_vector_set(rng, 8, 3, 5);

            let ldd_a = from_iter(&manager, a.iter());
            let ldd_b = from_iter(&manager, b.iter());

            let result = ldd_a.intersect(&ldd_b).unwrap();
            let expected: HashSet<Vec<u32>> = a.intersection(&b).cloned().collect();

            assert_eq!(count(&result), expected.len());
            for vector in &expected {
                assert!(crate::element_of(&manager, vector, &result), "missing {vector:?}");
            }
        })
    }

    /// Cross-checks [merge] against a `HashSet` reference implementation of the
    /// interleaved cartesian product.
    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_merge() {
        random_test(100, |rng| {
            let manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);

            let a = random_vector_set(rng, 8, 3, 5);
            let b = random_vector_set(rng, 8, 3, 5);

            let ldd_a = from_iter(&manager, a.iter());
            let ldd_b = from_iter(&manager, b.iter());

            let result = merge(&manager, &ldd_a, &ldd_b).unwrap();

            let mut expected: HashSet<Vec<u32>> = HashSet::new();
            for va in &a {
                for vb in &b {
                    let mut interleaved = Vec::with_capacity(va.len() + vb.len());
                    for (x, y) in va.iter().zip(vb.iter()) {
                        interleaved.push(*x);
                        interleaved.push(*y);
                    }
                    expected.insert(interleaved);
                }
            }

            assert_eq!(count(&result), expected.len());
            for vector in &expected {
                assert!(crate::element_of(&manager, vector, &result), "missing {vector:?}");
            }
        })
    }

    /// Cross-checks [fix_element] against a `HashSet` reference implementation.
    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_fix_element() {
        random_test(100, |rng| {
            let manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);

            let length = 5;
            let max_value = 5;
            let set = random_vector_set(rng, 32, length, max_value);
            let ldd = from_iter(&manager, set.iter());

            for level in 0..length {
                for value in 0..max_value {
                    let result = fix_element(&manager, &ldd, level, value).unwrap();
                    let expected: HashSet<Vec<u32>> = set.iter().filter(|v| v[level] == value).cloned().collect();

                    assert_eq!(
                        count(&result),
                        expected.len(),
                        "level {level}, value {value}: {:?} vs {:?}",
                        count(&result),
                        expected.len()
                    );
                    for vector in &expected {
                        assert!(crate::element_of(&manager, vector, &result), "missing {vector:?}");
                    }
                }
            }
        })
    }

    /// [`fix_element`] on the empty set must return the empty set regardless of `level`, not the
    /// terminal it happens to bottom out on.
    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_fix_element_empty_set() {
        let manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);
        let empty = manager.with_manager_shared(LDDFunction::empty_set).unwrap();

        for level in 0..5 {
            let result = fix_element(&manager, &empty, level, 0).unwrap();
            assert!(result.is_empty(), "level {level}: expected the empty set");
        }
    }
}
