/// Returns true iff the given permutation is a bijective mapping within the 0..max range.
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

#[cfg(test)]
mod tests {

    use super::*;

    use crate::random_test;
    use rand::seq::SliceRandom;

    #[test]
    fn test_random_is_valid_permutation() {
        random_test(100, |rng| {
            // Generate a valid permutation.
            let valid_permutation: Vec<usize> = {
                let mut order: Vec<usize> = (0..100).collect();
                order.shuffle(rng);
                order
            };

            assert!(is_valid_permutation(|i| valid_permutation[i], valid_permutation.len()));

            // Generate an invalid permutation (duplicate entries).
            let invalid_permutation = [0, 1, 2, 3, 4, 5, 6, 7, 8, 8];
            assert!(!is_valid_permutation(
                |i| invalid_permutation[i],
                invalid_permutation.len()
            ));

            // Generate an invalid permutation (missing entries).
            let invalid_permutation = [0, 1, 3, 4, 5, 6, 7, 8];
            assert!(!is_valid_permutation(
                |i| invalid_permutation[i],
                invalid_permutation.len()
            ));
        });
    }
}
