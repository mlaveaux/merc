#![forbid(unsafe_code)]

use std::fmt::Write;

use merc_data::DataVariable;
use merc_data::SortExpressionRef;

/// Generates fresh variable names, all sharing one `prefix`, guaranteed not
/// to collide with a caller-supplied set of names already in scope.
///
/// Every name this generates has the form `<prefix><index>`, so avoiding the
/// caller-supplied names reduces to seeding an index one past the highest
/// suffix already in use for `prefix`: every later candidate is then larger
/// than anything seeded, so generation needs no per-candidate lookup.
pub struct FreshVariableGenerator {
    /// The prefix shared by every generated name.
    prefix: String,
    /// The next index to try.
    next_index: u64,
    /// The value `next_index` had right after seeding in [`Self::new`],
    /// restored by [`Self::reset`].
    initial_index: u64,
    /// Reused across [`Self::generate`] calls to build the candidate name
    /// without a fresh heap allocation each time.
    scratch: String,
}

impl FreshVariableGenerator {
    /// Builds a generator of names `<prefix><index>` that avoids every name
    /// of that form already present in `used`.
    pub fn new<I>(prefix: &str, used: I) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        let mut next_index = 0;
        for name in used {
            if let Some(index) = split_index(&name, prefix) {
                next_index = next_index.max(index + 1);
            }
        }

        FreshVariableGenerator {
            prefix: prefix.to_string(),
            initial_index: next_index,
            next_index,
            scratch: String::new(),
        }
    }

    /// Restores this generator to the state [`Self::new`] left it in,
    /// discarding every name generated since.
    ///
    /// Lets a generator seeded once (e.g. from every name that could ever be
    /// in scope across a whole LPS) be reused indefinitely — one call per
    /// state during exploration.
    pub fn reset(&mut self) {
        self.next_index = self.initial_index;
    }

    /// Generates a fresh variable of `sort`, named `prefix` suffixed with
    /// the smallest natural number, no smaller than any this generator has
    /// already tried, that keeps it out of the used set.
    pub fn generate(&mut self, sort: SortExpressionRef<'_>) -> DataVariable {
        let index = self.next_index;
        self.next_index += 1;

        self.scratch.clear();
        write!(self.scratch, "{}{index}", self.prefix).expect("writing to a String never fails");
        DataVariable::with_sort(self.scratch.as_str(), sort)
    }
}

/// Returns `index` when `name` is exactly `prefix` followed by the canonical
/// decimal digits of `index` — the form [`FreshVariableGenerator::generate`]
/// itself produces, e.g. `"v03"` does not match `prefix` `"v"` since
/// `generate` would write `"v3"`.
fn split_index(name: &str, prefix: &str) -> Option<u64> {
    let digits = name.strip_prefix(prefix)?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    let index: u64 = digits.parse().ok()?;
    (index.to_string() == digits).then_some(index)
}

#[cfg(test)]
mod tests {
    use merc_data::BasicSort;
    use merc_data::SortExpression;

    use super::FreshVariableGenerator;
    use super::split_index;

    fn nat() -> SortExpression {
        SortExpression::from(BasicSort::new("Nat"))
    }

    #[test]
    fn test_generate_avoids_seeded_names() {
        let mut generator = FreshVariableGenerator::new("v", ["v0".to_string(), "v1".to_string()]);
        let fresh = generator.generate(nat().copy());
        assert_eq!(fresh.name(), "v2");
    }

    #[test]
    fn test_reset_undoes_generated_names_but_keeps_the_seed() {
        let mut generator = FreshVariableGenerator::new("v", ["v0".to_string()]);
        let first = generator.generate(nat().copy());
        assert_eq!(first.name(), "v1");

        generator.reset();

        // The seed name is still avoided, but the generated one is forgotten,
        // so the same fresh name is produced again.
        let second = generator.generate(nat().copy());
        assert_eq!(second.name(), "v1");
    }

    #[test]
    fn test_generate_avoids_its_own_earlier_output() {
        let mut generator = FreshVariableGenerator::new("v", std::iter::empty());
        let first = generator.generate(nat().copy());
        let second = generator.generate(nat().copy());
        assert_ne!(first.name(), second.name());
    }

    #[test]
    fn test_generate_ignores_names_with_a_different_prefix() {
        let mut generator = FreshVariableGenerator::new("v", ["w5".to_string()]);
        assert_eq!(generator.generate(nat().copy()).name(), "v0");
    }

    #[test]
    fn test_split_index_requires_a_canonical_suffix() {
        assert_eq!(split_index("v12", "v"), Some(12));
        assert_eq!(split_index("v0", "v"), Some(0));
        assert_eq!(split_index("v", "v"), None);
        assert_eq!(split_index("w12", "v"), None);
        // "v012" would not be produced by `generate`, which always writes the
        // canonical decimal form, so it must not shadow the candidate "v12".
        assert_eq!(split_index("v012", "v"), None);
    }
}
