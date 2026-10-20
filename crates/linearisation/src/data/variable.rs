use std::collections::HashSet;

use merc_data::DataVariable;

/// Hands out identifiers that clash with nothing in the specification being linearised.
///
/// Invariant: `used` contains the identifiers of the specification's declarations and process
/// bodies, and every name returned so far. Variables bound inside data expressions are not
/// included, which is why [NameGenerator::variable] never returns the name it is given.
pub(crate) struct NameGenerator {
    used: HashSet<String>,
}

impl NameGenerator {
    pub(crate) fn new<I: IntoIterator<Item = String>>(used: I) -> Self {
        NameGenerator {
            used: used.into_iter().collect(),
        }
    }

    /// `hint` when it is unused, and otherwise `hint` with a number appended.
    pub(crate) fn fresh(&mut self, hint: &str) -> String {
        if self.used.insert(hint.to_string()) {
            return hint.to_string();
        }
        self.numbered(hint)
    }

    /// A variable to rename `hint` to: of the same sort, with a fresh name that differs from the
    /// name of `hint` even when that name was not known to be in use.
    pub(crate) fn variable(&mut self, hint: &DataVariable) -> DataVariable {
        self.used.insert(hint.name().to_string());
        DataVariable::with_sort(self.numbered(hint.name()).as_str(), hint.sort())
    }

    /// `hint` with the first number appended that gives an unused name.
    fn numbered(&mut self, hint: &str) -> String {
        let base = hint.trim_end_matches(|c: char| c.is_ascii_digit());
        let base = if base.is_empty() { "x" } else { base };
        (1..)
            .map(|index| format!("{base}{index}"))
            .find(|candidate| self.used.insert(candidate.clone()))
            .expect("the counter is unbounded")
    }
}
