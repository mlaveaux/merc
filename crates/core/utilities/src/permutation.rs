/// Returns true iff the given permutation is a bijective mapping within the 0..max range.
///
/// This check requires O(max) time and space, so it should only be used for testing purposes.
pub fn is_valid_permutation<P>(permutation: P, max: usize) -> bool
where
    P: Fn(usize) -> usize,
{
    let mut visited = vec![false; max];

    for i in 0..max {
        // Out of bounds
        if permutation(i) >= max {
            return false;
        }

        if visited[permutation(i)] {
            return false;
        }
        visited[permutation(i)] = true;
    }

    true
}
