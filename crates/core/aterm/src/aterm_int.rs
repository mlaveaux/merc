use std::fmt;

use merc_macros::merc_derive_terms;

use crate::Symb;
use crate::Term;
use crate::storage::THREAD_TERM_POOL;

/// Returns true if the term is an [ATermInt] term.
pub fn is_int_term<'a, 'b, T: Term<'a, 'b>>(t: &'b T) -> bool {
    THREAD_TERM_POOL.with(|tp| *tp.int_symbol() == t.get_head_symbol())
}

/// Returns true if the given symbol is the special integer symbol used by [ATermInt].
pub(crate) fn is_int_symbol<'a, 'b, S: Symb<'a, 'b>>(f: &'b S) -> bool {
    THREAD_TERM_POOL.with(|tp| *tp.int_symbol() == f.copy())
}

#[merc_derive_terms]
mod inner {
    use delegate::delegate;

    use merc_macros::merc_ignore;
    use merc_macros::merc_term;

    use crate::ATerm;
    use crate::ATermArgs;
    use crate::ATermIndex;
    use crate::ATermRef;
    use crate::Markable;
    use crate::SymbolRef;
    use crate::Term;
    use crate::TermIterator;
    use crate::Transmutable;
    use crate::is_int_term;
    use crate::storage::Marker;
    use crate::storage::SharedTermInt;
    use crate::storage::THREAD_TERM_POOL;

    /// This is a wrapper around the [ATerm] type that stores a single `u64` using an annotation.
    #[merc_term(is_int_term)]
    pub struct ATermInt {
        term: ATerm,
    }

    impl ATermInt {
        #[merc_ignore]
        pub fn new(value: usize) -> ATermInt {
            THREAD_TERM_POOL.with(|tp| ATermInt {
                term: tp.create_int(value),
            })
        }

        /// Returns the value of the integer term.
        ///
        /// # Panics
        ///
        /// Panics if the term is not an integer term. Use
        /// [`ATermInt::value_unchecked`] to skip this check when the term is
        /// already known to be an integer term.
        pub fn value(&self) -> usize {
            assert!(is_int_term(self), "value() called on non-integer term {self:?}");

            // SAFETY: We just checked that this is an integer term.
            unsafe { self.value_unchecked() }
        }

        /// Returns the value of the integer term without checking that the
        /// term is actually an integer term.
        ///
        /// # Safety
        ///
        /// The caller must ensure that the term is an integer term, i.e. that
        /// [`is_int_term`] returns true for it. Calling this on a non-integer
        /// term reads past the end of the term's allocation, which is
        /// undefined behaviour.
        pub unsafe fn value_unchecked(&self) -> usize {
            // SAFETY: The caller guarantees that this term wraps an integer term, which has a
            // zero-arity symbol; passing 0 avoids reading the symbol header just to rebuild the
            // wide pointer that the cast to the sized `SharedTermInt` immediately discards.
            unsafe { self.shared().ptr_with_len(0).cast::<SharedTermInt>().as_ref().value() }
        }
    }
}

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

pub use inner::*;
