use std::fmt;

use crate::ATerm;

/// Represents a atermpp::aterm_string from the mCRL2 toolset.
#[derive(PartialEq, Eq)]
pub struct ATermString {
    term: ATerm,
}

impl ATermString {
    /// Creates a new `ATermString` from the given term.
    pub fn new(term: ATerm) -> Self {
        Self { term }
    }

    /// Returns the string value.
    pub fn str(&self) -> String {
        self.term.get_head_symbol().name().to_string()
    }
}

impl fmt::Debug for ATermString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.str())
    }
}
