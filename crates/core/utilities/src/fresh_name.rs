use std::collections::HashMap;
use std::collections::HashSet;

/// Generates names of the form `<base><index>` that are distinct from a set of names already in
/// use, and from every name it generated before, whatever their base.
pub struct FreshNameGenerator {
    used: HashSet<String>,
    /// The index to try first for each base: every smaller index is already used, so repeated
    /// requests for one base take amortised constant time instead of rescanning from zero.
    next: HashMap<String, usize>,
}

impl FreshNameGenerator {
    /// Creates a generator that avoids every name in `used`.
    pub fn new<I>(used: I) -> Self
    where
        I: IntoIterator<Item = String>,
    {
        FreshNameGenerator {
            used: used.into_iter().collect(),
            next: HashMap::new(),
        }
    }

    /// Returns `base` suffixed with the smallest natural number that makes it fresh, and marks
    /// the result as used.
    pub fn generate(&mut self, base: &str) -> String {
        let mut index = self.next.get(base).copied().unwrap_or(0);
        loop {
            let candidate = format!("{base}{index}");
            index += 1;
            if self.used.insert(candidate.clone()) {
                self.next.insert(base.to_string(), index);
                return candidate;
            }
        }
    }
}
