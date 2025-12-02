use std::collections::HashMap;
use std::fmt;

use merc_utilities::MercError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Permutation {
    /// We represent a permutation by a (sorted) mapping from old indices to new indices.
    mapping: Vec<usize>,
}

impl Permutation {
    /// Create a permutation from a given mapping (does not assume anything about the mapping).
    pub fn from_mapping(mut mapping: Vec<usize>) -> Self {
        mapping.sort();
        mapping.dedup();

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

        // Parse all the commas.
        let mut mapping = HashMap::new();
        for token in input_no_brackets.split(',') {
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

            if mapping.contains_key(&from) {
                return Err(MercError::from(format!(
                    "Invalid permutation: multiple mappings for {}",
                    from
                )));
            }

            mapping.insert(from, to);
        }

        // Convert HashMap to sorted Vec
        let mut mapping_vec: Vec<_> = mapping.into_iter().collect();
        mapping_vec.sort_by_key(|(k, _)| *k);
        let mapping = mapping_vec.into_iter().map(|(_, v)| v).collect();

        Ok(Permutation { mapping })
    }

    /// Construct a new permutation by concatenating two (disjoint) permutations.
    pub fn concat(self, other: &Permutation) -> Permutation {
        debug_assert!(
            self.mapping.iter().all(|x| !other.mapping.contains(x)),
            "There should be no overlap between the two permutations being concatenated."
        );

        let mut mapping = self.mapping;
        mapping.extend_from_slice(&other.mapping);

        Permutation::from_mapping(mapping)
    }
}

impl fmt::Display for Permutation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "(")?;
        let mut visited = vec![false; self.mapping.len()];
        let mut first_cycle = true;

        for start in 0..self.mapping.len() {
            if visited[start] || self.mapping[start] == start {
                visited[start] = true;
                continue;
            }

            if !first_cycle {
                write!(f, " ")?;
            }
            first_cycle = false;

            write!(f, "(")?;
            let mut current = start;
            let mut first_in_cycle = true;

            loop {
                if !first_in_cycle {
                    write!(f, " ")?;
                }
                first_in_cycle = false;

                write!(f, "{}", current)?;
                visited[current] = true;
                current = self.mapping[current];

                if current == start {
                    break;
                }
            }
            write!(f, ")")?;
        }

        if first_cycle {
            write!(f, ")")?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permutation_from_input() {
        let permutation = Permutation::from_input("[0->   2, 1   ->0, 2->1]").unwrap();

        assert!(permutation.mapping == vec![2, 0, 1]);
    }

    #[test]
    fn test_cycle_notation() {
        let permutation = Permutation::from_input("[0->2, 1->0, 2->1, 3->4, 4->3]").unwrap();

        assert_eq!(permutation.to_string(), "((0 2 1) (3 4))");
    }
}
