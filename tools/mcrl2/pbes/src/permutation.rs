/// Authors: Menno Bartels and Maurice Laveaux
use itertools::Itertools;
use std::collections::HashSet;
use std::fmt;

use merc_utilities::MercError;

#[derive(Clone, PartialEq, Eq)]
pub struct Permutation {
    /// We represent a permutation as an explicit list of (domain -> image) pairs,
    /// sorted by domain.
    mapping: Vec<(usize, usize)>,
}

impl Permutation {
    /// Create a permutation from a given mapping of (domain -> image) pairs.
    /// The function validates that:
    /// - domain entries are unique,
    /// - images are exactly a permutation of the domain entries,
    /// - internal representation is sorted by domain.
    pub fn from_mapping(mut mapping: Vec<(usize, usize)>) -> Self {
        // Validate lengths and uniqueness in debug builds.
        if cfg!(debug_assertions) {
            let mut seen_domain = HashSet::new();
            for (d, _) in &mapping {
                debug_assert!(
                    seen_domain.insert(*d),
                    "Invalid permutation mapping: multiple mappings for {}",
                    d
                );
            }
        }

        // Sort by domain for deterministic representation.
        mapping.sort_unstable_by_key(|(d, _)| *d);

        Permutation { mapping }
    }

    /// Parse a permutation from a string input of the form "[0->2, 1->0, 2->1]".
    pub fn from_input(line: &str) -> Result<Self, MercError> {
        // Remove the surrounding brackets if present.
        let trimmed_input = line.trim();
        let input_no_brackets =
            if !trimmed_input.is_empty() && trimmed_input.starts_with('[') && trimmed_input.ends_with(']') {
                &trimmed_input[1..trimmed_input.len() - 1]
            } else {
                return Err("Permutation must be enclosed in brackets []".into());
            };

        // Parse all the comma-separated tokens into (from, to) pairs.
        let mut pairs: Vec<(usize, usize)> = Vec::new();
        for token in input_no_brackets.split(',') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }

            let arrow_pos = token
                .find("->")
                .ok_or_else(|| MercError::from(format!("Invalid permutation format: {}", token)))?;

            let from_str = token[..arrow_pos].trim();
            let to_str = token[arrow_pos + 2..].trim();

            let from: usize = from_str
                .parse()
                .map_err(|_| MercError::from(format!("Invalid number: {}", from_str)))?;
            let to: usize = to_str
                .parse()
                .map_err(|_| MercError::from(format!("Invalid number: {}", to_str)))?;

            if pairs.iter().any(|(f, _)| *f == from) {
                return Err(MercError::from(format!(
                    "Invalid permutation: multiple mappings for {}",
                    from
                )));
            }

            pairs.push((from, to));
        }

        Ok(Permutation::from_mapping(pairs))
    }

    /// Construct a new permutation by concatenating two (disjoint) permutations.
    pub fn concat(self, other: &Permutation) -> Permutation {
        debug_assert!(
            self.mapping
                .iter()
                .all(|(left, _)| !other.mapping.iter().any(|(right, _)| right == left)),
            "There should be no overlap between the two permutations being concatenated."
        );

        let mut mapping = self.mapping;
        mapping.extend_from_slice(&other.mapping);

        Permutation::from_mapping(mapping)
    }

    /// Returns the value of the permutation at the given key.
    pub fn value(&self, key: usize) -> usize {
        for (d, v) in &self.mapping {
            if *d == key {
                return *v;
            }
        }

        key // It is the identity on unspecified elements.
    }
}

/// Display the permutation in cycle notation.
///
/// Cycle notation is a standard way to present permutations, where each cycle
/// is represented by parentheses. For example, the permutation that maps 0->2,
/// 1->0, 2->1 would be represented as (0 2 1). Cycles containing a single
/// element (fixed points) are omitted for brevity.
impl fmt::Display for Permutation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Determine the maximum value in the permutation mapping.
        let max_value = self.mapping.iter().map(|(d, _)| *d + 1).max().unwrap_or(0);

        let mut visited = vec![false; max_value];
        let mut identity = true;

        // The mapping is sorted by domain, so we can iterate over it directly.
        for (start, value) in &self.mapping {
            if visited[*value] || self.value(*start) == *start {
                // We have already visited this element, or it is a fixed point.
                visited[*value] = true;
                continue;
            }

            write!(f, "(")?;
            let mut current = *start;
            let mut first_in_cycle = true;
            identity = false; // At least one non-trivial cycle found.

            loop {
                if !first_in_cycle {
                    // Print space between elements in the cycle.
                    write!(f, " ")?;
                }
                first_in_cycle = false;

                write!(f, "{}", current)?;
                visited[current] = true;
                current = self.value(current);

                if current == *start {
                    break;
                }
                assert!(!visited[current], "This is not a valid permutation!");
            }
            write!(f, ")")?;
        }

        if identity {
            write!(f, "()")?;
        }

        Ok(())
    }
}

impl fmt::Debug for Permutation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[")?;
        for (i, (d, v)) in self.mapping.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{} -> {}", d, v)?;
        }
        write!(f, "]")
    }
}

/// Given a set of indices, generate the permutation group on these indices.
///
/// For the variables {0, 3, 4} this would generate the permutations in cycle notation:
/// - Identity: ()
/// - (0 3)
/// - (0 4)
/// - (3 4)
/// - (0 3 4)
/// - (0 4 3)
pub fn permutation_group(indices: Vec<usize>) -> impl Iterator<Item = Permutation> + Clone {
    let n = indices.len();
    let indices2 = indices.clone();
    indices.into_iter().permutations(n).map(move |perm| {
        let mapping: Vec<(usize, usize)> = indices2
            .iter()
            .cloned()
            .zip(perm.into_iter())
            .map(|(a, b)| (a, b))
            .collect();
        Permutation::from_mapping(mapping)
    })
}

/// Returns the number of permutations in a given group.
pub fn permutation_group_size(n: usize) -> usize {
    (1..=n).product()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permutation_from_input() {
        let permutation = Permutation::from_input("[0->   2, 1   ->0, 2->1]").unwrap();

        assert!(permutation.mapping == vec![(0, 2), (1, 0), (2, 1)]);
    }

    #[test]
    fn test_cycle_notation() {
        let permutation = Permutation::from_input("[0->2, 1->0, 2->1, 3->4, 4->3]").unwrap();
        println!("{:?}", permutation.mapping);

        assert_eq!(permutation.to_string(), "(0 2 1)(3 4)");
    }

    #[test]
    fn test_permutation_group() {
        let indices = vec![0, 3, 5];
        let permutations: Vec<Permutation> = permutation_group(indices.clone()).collect();
        for p in &permutations {
            println!("{}", p);
        }

        assert_eq!(permutations.len(), permutation_group_size(indices.len()));
    }
}
