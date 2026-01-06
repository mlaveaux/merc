use std::fmt;

use mcrl2_macros::mcrl2_derive_terms;

use crate::ATermRef;

pub fn is_aterm_string(term: &ATermRef<'_>) -> bool {
    term.get_head_symbol().arity() == 0
}

#[mcrl2_derive_terms]
mod inner {
    use mcrl2_macros::mcrl2_term;

    use crate::ATerm;
    use crate::ATermRef;
    use crate::Markable;
    use crate::Todo;
    use crate::is_aterm_string;

    /// Represents an atermpp::aterm_string from the mCRL2 toolset.
    #[mcrl2_term(is_aterm_string)]
    pub struct ATermString {
        term: ATerm,
    }

    impl ATermString {
        /// Returns the string value.
        pub fn str(&self) -> String {
            // The Rust::Str should ensure that this is a valid string.
            self.term.get_head_symbol().name().to_string()
        }
    }
}

pub use inner::*;

impl fmt::Display for ATermString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.str())
    }
}

impl fmt::Display for ATermStringRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.str())
    }
}

