use std::borrow::Borrow;
use std::fmt;
use std::marker::PhantomData;
use std::mem::transmute;
use std::ops::Deref;

use delegate::delegate;

use mcrl3_aterm::ATerm;
use mcrl3_aterm::ATermArgs;
use mcrl3_aterm::ATermIndex;
use mcrl3_aterm::ATermRef;
use mcrl3_aterm::Markable;
use mcrl3_aterm::Marker;
use mcrl3_aterm::Symb;
use mcrl3_aterm::SymbolRef;
use mcrl3_aterm::Term;
use mcrl3_aterm::TermIterator;
use mcrl3_aterm::Transmutable;
use mcrl3_macros::mcrl3_derive_terms;
use mcrl3_macros::mcrl3_term;

use crate::DATA_SYMBOLS;
use crate::is_sort_expression;

// This module is only used internally to run the proc macro.
#[mcrl3_derive_terms]
mod inner {
    use mcrl3_aterm::ATermString;

    use super::*;

    #[mcrl3_term(is_sort_expression)]
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
