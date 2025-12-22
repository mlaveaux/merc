use std::fmt;
use std::ops::Deref;

use delegate::delegate;

use merc_aterm::ATerm;
use merc_aterm::ATermArgs;
use merc_aterm::ATermIndex;
use merc_aterm::ATermRef;
use merc_aterm::Markable;
use merc_aterm::Symb;
use merc_aterm::SymbolRef;
use merc_aterm::Term;
use merc_aterm::TermIterator;
use merc_aterm::Transmutable;
use merc_aterm::storage::Marker;
use merc_macros::merc_derive_terms;
use merc_macros::merc_term;

use crate::DATA_SYMBOLS;
use crate::is_sort_expression;

// This module is only used internally to run the proc macro.
#[merc_derive_terms]
mod inner {
    use merc_aterm::ATermString;

    use super::*;

    #[merc_term(is_sort_expression)]
    pub struct SortExpression {
        term: ATerm,
    }

    impl SortExpression {
        /// Returns the name of the sort.
        pub fn name(&self) -> &str {
            self.term.arg(0).get_head_symbol().name()
        }

        /// Creates a sort expression with the unknown value.
        pub fn unknown_sort() -> SortExpression {
            DATA_SYMBOLS.with_borrow(|ds| SortExpression {
                term: ATerm::with_args(ds.sort_id_symbol.deref(), &[ATermString::new("@no_value@")]).protect(),
            })
        }
    }

    impl fmt::Display for SortExpression {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.name())
        }
    }
}

pub use inner::*;
