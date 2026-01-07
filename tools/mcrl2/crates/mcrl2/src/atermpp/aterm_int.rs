use std::fmt;

use mcrl2_macros::mcrl2_derive_terms;

use crate::ATermRef;

pub fn is_aterm_int(term: &ATermRef<'_>) -> bool {
    false
}

#[mcrl2_derive_terms]
mod inner {
    use mcrl2_macros::mcrl2_term;

    use crate::ATerm;
    use crate::ATermRef;
    use crate::Markable;
    use crate::Todo;
    use crate::is_aterm_int;

    /// Represents an atermpp::aterm_string from the mCRL2 toolset.
    #[mcrl2_term(is_aterm_int)]
    pub struct ATermInt {
        term: ATerm,
    }

    impl ATermInt {
        /// Returns the string value.
        pub fn value(&self) -> u64 {
            // The Rust::Str should ensure that this is a valid string.
            0
        }
    }
}

pub use inner::*;

impl fmt::Display for ATermInt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value())
    }
}

impl fmt::Display for ATermIntRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value())
    }
}

