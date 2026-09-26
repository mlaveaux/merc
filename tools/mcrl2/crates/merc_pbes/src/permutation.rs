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
    /// Create a permutation from a given mapping of (domain -> image) pairs. Internally
    /// sorts the mapping by domain for a unique representation. The input must be
    /// a valid permutation (so a bijection).
    pub fn from_mapping(mut mapping: Vec<(usize, usize)>) -> Self {
        debug_assert!(
            is_valid_permutation(&mapping),
            "Input mapping is not a valid permutation: {:?}",
            mapping
        );

        // Sort by domain for deterministic representation.
        mapping.sort_unstable_by_key(|(d, _)| *d);
        debug_assert!(mapping.iter().is_sorted(), "Mapping should be sorted by domain.");
        debug_assert!(
            mapping.iter().all(|(from, to)| from != to),
            "Mapping should not contain identity mappings."
        );
        debug_assert!(
            mapping.iter().duplicates().count() == 0,
            "Mapping should not contain duplicate domain entries."
        );

        Permutation { mapping }
    }

    /// Parse a permutation from a string input of the form "[0->2, 1->0, 2->1]".
    pub fn from_mapping_notation(line: &str) -> Result<Self, MercError> {
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

            // Skip identity pairs: they name a fixed point and would otherwise
            // produce an identity pair, which `Permutation::from_mapping` rejects.
            if from == to {
                continue;
            }

            if pairs.iter().any(|(f, _)| *f == from) {
                return Err(MercError::from(format!(
                    "Invalid permutation: multiple mappings for {}",
                    from
                )));
            }

            pairs.push((from, to));
        }

        if !is_valid_permutation(&pairs) {
            return Err(MercError::from("Input mapping is not a valid permutation."));
        }

        Ok(Permutation::from_mapping(pairs))
    }

    /// Parse a permutation in cycle notation, e.g., (0 2 1)(3 4).
    pub fn from_cycle_notation(cycle_notation: &str) -> Result<Self, MercError> {
        let mut mapping: Vec<(usize, usize)> = Vec::new();

        // Split the input into cycles by finding all '(...)' groups
        for cycle_str in cycle_notation.split('(').skip(1) {
            // Find the closing parenthesis
            let cycle_content = cycle_str
                .split(')')
                .next()
                .ok_or_else(|| MercError::from("Invalid cycle notation: missing closing ')'"))?;

            // Skip empty cycles
            if cycle_content.trim().is_empty() {
                continue;
            }

            // Parse all numbers in this cycle
            let cycle_elements: Result<Vec<usize>, MercError> = cycle_content
                .split_whitespace()
                .map(|num_str| {
                    num_str
                        .parse::<usize>()
                        .map_err(|_| MercError::from(format!("Invalid number in cycle notation: {}", num_str)))
                })
                .collect();

            let cycle_elements = cycle_elements?;

            // Skip singleton cycles: they name a fixed point and would otherwise
            // produce an identity pair, which `Permutation::from_mapping` rejects.
            if cycle_elements.len() == 1 {
                continue;
            }

            // Create mappings for the current cycle (each element maps to the next)
            let len = cycle_elements.len();
            for i in 0..len {
                let from = cycle_elements[i];
                let to = cycle_elements[(i + 1) % len];
                mapping.push((from, to));
            }
        }

        if !is_valid_permutation(&mapping) {
            return Err(MercError::from("Input mapping is not a valid permutation."));
        }

        Ok(Permutation::from_mapping(mapping))
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

    /// Returns an iterator over the domain of this permutation.
    pub fn domain(&self) -> impl Iterator<Item = usize> + '_ {
        self.mapping.iter().map(|(d, _)| *d)
    }

    /// Check whether this permutation is the identity permutation.
    pub fn is_identity(&self) -> bool {
        self.mapping.iter().all(|(d, v)| d == v)
    }

    /// Returns the largest point this permutation mentions, or `None` when it
    /// moves nothing.
    ///
    /// Used to reject a generator whose points fall outside the parameter range
    /// before it reaches `crate::bsgs::DensePermutation`, which silently
    /// truncates to the group's degree.
    pub fn max_point(&self) -> Option<usize> {
        self.mapping.iter().map(|&(d, v)| d.max(v)).max()
    }
}

/// Checks whether the mapping represents a valid permutation
pub fn is_valid_permutation(mapping: &[(usize, usize)]) -> bool {
    let mut domain = HashSet::with_capacity(mapping.len());
    let mut image = HashSet::with_capacity(mapping.len());

    for (d, v) in mapping {
        if !domain.insert(*d) || !image.insert(*v) {
            // Duplicate found in domain or image
            return false;
        }
    }

    domain == image
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
        let max_value = self.mapping.iter().flat_map(|(d, e)| [*d, *e]).max().unwrap_or(0);

        let mut visited = vec![false; max_value + 1];
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

    // Clone the indices for use in the closure.
    let indices_rhs = indices.clone();
    indices.into_iter().permutations(n).map(move |perm| {
        // Remove identity mappings.
        let mapping: Vec<(usize, usize)> = indices_rhs.iter().cloned().zip(perm).filter(|(x, y)| x != y).collect();
        Permutation::from_mapping(mapping)
    })
}

/// Returns the number of permutations in a given group (`n!`), saturating at
/// `usize::MAX` instead of overflowing for large `n`.
pub fn permutation_group_size(n: usize) -> usize {
    (1..=n).fold(1usize, |acc, factor| acc.saturating_mul(factor))
}
