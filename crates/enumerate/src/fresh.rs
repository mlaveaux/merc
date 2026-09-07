use ahash::AHashSet;
use ahash::HashMap;
use ahash::HashMapExt;
use merc_data::DataVariable;
use merc_data::SortExpressionRef;

/// Generates fresh variable names guaranteed not to collide with a
/// caller-supplied set of names already in scope.
pub struct FreshVariableGenerator {
    used: AHashSet<String>,
    /// The next index to try per `base`, for cheap generation of fresh names.
    next_index: HashMap<String, u64>,
}

impl FreshVariableGenerator {
    /// Builds a generator that avoids every name in `used`.
    pub fn new(used: impl IntoIterator<Item = String>) -> Self {
        FreshVariableGenerator {
            used: used.into_iter().collect(),
            next_index: HashMap::new(),
        }
    }

    /// Generates a fresh variable of `sort`, named `base` suffixed with the
    /// smallest natural number, no smaller than any this generator has
    /// already tried for `base`, that keeps it out of the used set.
    pub fn generate(&mut self, base: &str, sort: SortExpressionRef<'_>) -> DataVariable {
        let mut index = self.next_index.get(base).copied().unwrap_or(0);
        let name = loop {
            let candidate = format!("{base}{index}");
            if !self.used.contains(&candidate) {
                break candidate;
            }
            index += 1;
        };

        if let Some(next) = self.next_index.get_mut(base) {
            *next = index + 1;
        } else {
            self.next_index.insert(base.to_string(), index + 1);
        }

        self.used.insert(name.clone());
        DataVariable::with_sort(name.as_str(), sort)
    }
}

#[cfg(test)]
mod tests {
    use merc_data::BasicSort;
    use merc_data::SortExpression;

    use super::FreshVariableGenerator;

    fn nat() -> SortExpression {
        SortExpression::from(BasicSort::new("Nat"))
    }

    #[test]
    fn test_generate_avoids_seeded_names() {
        let mut generator = FreshVariableGenerator::new(["v0".to_string(), "v1".to_string()]);
        let fresh = generator.generate("v", nat().copy());
        assert_eq!(fresh.name(), "v2");
    }

    #[test]
    fn test_generate_avoids_its_own_earlier_output() {
        let mut generator = FreshVariableGenerator::new(std::iter::empty());
        let first = generator.generate("v", nat().copy());
        let second = generator.generate("v", nat().copy());
        assert_ne!(first.name(), second.name());
    }
}
