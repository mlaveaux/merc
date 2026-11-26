use std::cmp::Ordering;

use crate::ATermIntRef;
use crate::ATermRef;
use crate::Symb;
use crate::Term;
use crate::is_int_term;

/// Wraps a term so that [Ord] compares it structurally instead of by its
/// address in the term pool.
///
/// The default [Ord] on terms is cheap, but because it compares addresses the
/// resulting order differs between runs (address space layout randomisation,
/// thread scheduling, garbage collection). Use this wrapper wherever the order
/// becomes observable, for example in printed or written output, or when it
/// decides which of several candidates is tried first.
///
/// Terms are ordered by the name of their head symbol, then its arity, then
/// their arguments from left to right; integer terms are ordered by value and
/// precede all other terms, as in mCRL2.
/// Equality and hashing are those of `T`, which agree with this order since
/// terms are maximally shared.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct StructuralOrd<T>(pub T);

impl<'a, 'b, T: Term<'a, 'b> + Eq> PartialOrd for StructuralOrd<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<'a, 'b, T: Term<'a, 'b> + Eq> Ord for StructuralOrd<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        structural_cmp(&self.0.borrow(), &other.0.borrow())
    }
}

/// Compares two terms structurally, see [StructuralOrd].
pub fn structural_cmp(lhs: &ATermRef<'_>, rhs: &ATermRef<'_>) -> Ordering {
    let mut lhs = lhs.copy();
    let mut rhs = rhs.copy();

    // Maximal sharing makes structurally equal subterms identical.
    while lhs != rhs {
        match (is_int_term(&lhs), is_int_term(&rhs)) {
            (true, true) => return ATermIntRef::from(lhs).value().cmp(&ATermIntRef::from(rhs).value()),
            (true, false) => return Ordering::Less,
            (false, true) => return Ordering::Greater,
            (false, false) => {}
        }

        let lhs_symbol = lhs.get_head_symbol();
        let rhs_symbol = rhs.get_head_symbol();
        if lhs_symbol != rhs_symbol {
            // Function symbols with the same name and arity are equal.
            return lhs_symbol
                .name()
                .cmp(rhs_symbol.name())
                .then_with(|| lhs_symbol.arity().cmp(&rhs_symbol.arity()));
        }

        // The last argument is handled iteratively, such that long lists do not lead to deep recursion.
        let arity = lhs_symbol.arity();
        debug_assert!(arity > 0, "distinct terms with the same symbol differ in an argument");
        for index in 0..arity - 1 {
            let ordering = structural_cmp(&lhs.arg(index), &rhs.arg(index));
            if ordering != Ordering::Equal {
                return ordering;
            }
        }

        let last = arity - 1;
        (lhs, rhs) = (lhs.arg(last), rhs.arg(last));
    }

    Ordering::Equal
}
