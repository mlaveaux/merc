use ahash::AHashSet;
use merc_data::DataVariable;
use merc_data::SortExpressionRef;

/// Generates fresh variable names guaranteed not to collide with a
/// caller-supplied set of names already in scope.
///
/// Deliberately **not** a single process-wide generator seeded by scanning
/// the whole symbol table, unlike mCRL2's `enumerator_identifier_generator`
/// (`docs/enumeration-crate-plan.md` §4.8, which this corrects): this crate
/// has no notion of "the whole symbol table", and inventing one from just the
/// one `(vars, body)` pair an [`Enumerator`](crate::Enumerator) call happens
/// to see would still be wrong whenever more names are in scope than that —
/// e.g. an LPS's *other* summands, sharing the same process-parameter
/// substitution, whose variables this enumerator's fresh names must also
/// avoid even though they never appear in the one guard being enumerated.
///
/// So the caller builds the generator from exactly the names that must not
/// collide — the same pattern `merc_vpg::FreshStateVarGenerator` already uses
/// for modal formulas — and threads it through one [`Enumerator`](crate::Enumerator) instance
/// for that instance's whole lifetime (across every `enumerate`/`find_witness`
/// call it serves), rather than the enumerator inventing its own notion of
/// "the context" internally.
pub struct FreshVariableGenerator {
    used: AHashSet<String>,
}

impl FreshVariableGenerator {
    /// Builds a generator that avoids every name in `used`.
    pub fn new(used: impl IntoIterator<Item = String>) -> Self {
        FreshVariableGenerator {
            used: used.into_iter().collect(),
        }
    }

    /// Generates a fresh variable of `sort`, named `base` suffixed with the
    /// smallest natural number that keeps it out of the used set.
    pub fn generate(&mut self, base: &str, sort: SortExpressionRef<'_>) -> DataVariable {
        let mut index = 0;
        let name = loop {
            let candidate = format!("{base}{index}");
            if !self.used.contains(&candidate) {
                break candidate;
            }
            index += 1;
        };
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
