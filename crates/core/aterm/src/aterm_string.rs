#![forbid(unsafe_code)]

use merc_macros::merc_derive_terms;
use merc_utilities::MercError;

use crate::ATermRead;
use crate::ATermStreamable;
use crate::ATermWrite;
use crate::Symb;
use crate::Term;

/// Returns true if the term is a string term
fn is_string_term<'a, 'b, T: Term<'a, 'b>>(t: &'b T) -> bool {
    t.get_head_symbol().arity() == 0
}

#[merc_derive_terms]
mod inner {
    use std::fmt;

    use delegate::delegate;

    use merc_macros::merc_ignore;
    use merc_macros::merc_term;

    use crate::ATerm;
    use crate::ATermArgs;
    use crate::ATermIndex;
    use crate::ATermRef;
    use crate::Markable;
    use crate::Symb;
    use crate::Symbol;
    use crate::SymbolRef;
    use crate::Term;
    use crate::TermIterator;
    use crate::Transmutable;
    use crate::aterm_string::is_string_term;
    use crate::storage::Marker;
    use crate::storage::THREAD_TERM_POOL;

    #[merc_term(is_string_term)]
    pub struct ATermString {
        term: ATerm,
    }

    impl ATermString {
        /// Creates a new [ATermString] from a string slice.
        ///
        /// # Panics
        ///
        /// This function will panic if the string is a reserved symbol name.
        #[merc_ignore]
        pub fn new<S>(string: S) -> Self
        where
            S: Into<String> + AsRef<str>,
        {
            THREAD_TERM_POOL.with(|tp| ATermString {
                term: tp.create_constant(&Symbol::new(string, 0)),
            })
        }

        /// Creates the [ATermString] whose value is the name of `symbol`, which must be a constant.
        #[merc_ignore]
        pub fn with_symbol(symbol: &Symbol) -> Self {
            debug_assert_eq!(symbol.arity(), 0, "a string is a constant");
            THREAD_TERM_POOL.with(|tp| ATermString {
                term: tp.create_constant(symbol),
            })
        }

        /// Get the value of the string
        pub fn value(&self) -> &str {
            self.term.get_head_symbol().name()
        }
    }

    #[merc_ignore]
    impl From<&str> for ATermString {
        fn from(s: &str) -> Self {
            ATermString::new(s)
        }
    }

    impl fmt::Display for ATermString {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.value())
        }
    }
}

pub use inner::*;

impl PartialEq<str> for ATermString {
    fn eq(&self, other: &str) -> bool {
        self.value() == other
    }
}

impl PartialEq<&str> for ATermStringRef<'_> {
    fn eq(&self, other: &&str) -> bool {
        self.value() == *other
    }
}

// Helper to write a string immediately.
impl ATermStreamable for String {
    fn write<W: ATermWrite>(&self, writer: &mut W) -> Result<(), MercError> {
        writer.write_aterm(&ATermString::new(self.clone()))
    }

    fn read<R: ATermRead>(reader: &mut R) -> Result<Self, MercError>
    where
        Self: Sized,
    {
        let term: ATermString = reader.read_aterm()?.ok_or("Expected a string ATerm")?.into();
        Ok(term.value().to_string())
    }
}
