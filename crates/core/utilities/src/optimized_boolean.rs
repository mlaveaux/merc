use crate::BinaryChain;
use crate::ChainIter;
use crate::ChainStack;

/// A term language with Boolean connectives, such as data expressions or PBES
/// expressions.
///
/// The `optimized_*` functions below simplify when an operand is a constant, so
/// that algorithms that build formulas incrementally do not accumulate `true &&
/// …` and `!!…`.
pub trait Boolean: Clone + PartialEq {
    fn true_constant() -> Self;
    fn false_constant() -> Self;
    fn is_true(&self) -> bool;
    fn is_false(&self) -> bool;

    /// When `self` is a negation `!x`, its operand `x`.
    fn negated(&self) -> Option<Self>;
    fn make_not(self) -> Self;
    fn make_and(self, rhs: Self) -> Self;
    fn make_or(self, rhs: Self) -> Self;
    fn make_imp(self, rhs: Self) -> Self;
}

/// `!arg`, simplified for constants and double negation.
pub fn optimized_not<T: Boolean>(arg: T) -> T {
    if arg.is_true() {
        T::false_constant()
    } else if arg.is_false() {
        T::true_constant()
    } else if let Some(inner) = arg.negated() {
        inner
    } else {
        arg.make_not()
    }
}

/// `lhs && rhs`, simplified for constants and equal operands.
pub fn optimized_and<T: Boolean>(lhs: T, rhs: T) -> T {
    if lhs.is_true() || rhs.is_false() {
        rhs
    } else if rhs.is_true() || lhs.is_false() || lhs == rhs {
        lhs
    } else {
        lhs.make_and(rhs)
    }
}

/// `lhs || rhs`, simplified for constants and equal operands.
pub fn optimized_or<T: Boolean>(lhs: T, rhs: T) -> T {
    if lhs.is_false() || rhs.is_true() {
        rhs
    } else if rhs.is_false() || lhs.is_true() || lhs == rhs {
        lhs
    } else {
        lhs.make_or(rhs)
    }
}

/// `lhs => rhs`, simplified for constants and equal operands.
pub fn optimized_implies<T: Boolean>(lhs: T, rhs: T) -> T {
    if lhs.is_true() {
        rhs
    } else if lhs.is_false() || rhs.is_true() || lhs == rhs {
        T::true_constant()
    } else if rhs.is_false() {
        optimized_not(lhs)
    } else {
        lhs.make_imp(rhs)
    }
}

/// The conjunction of `terms`, `true` when there are none. Stops at the first `false`.
pub fn optimized_conjunction<T: Boolean, I: IntoIterator<Item = T>>(terms: I) -> T {
    let mut result = T::true_constant();
    for term in terms {
        if term.is_false() {
            return term;
        }

        result = optimized_and(result, term);
    }

    result
}

/// The disjunction of `terms`, `false` when there are none. Stops at the first `true`.
pub fn optimized_disjunction<T: Boolean, I: IntoIterator<Item = T>>(terms: I) -> T {
    let mut result = T::false_constant();
    for term in terms {
        if term.is_true() {
            return term;
        }

        result = optimized_or(result, term);
    }

    result
}

/// A [BinaryChain] handle into a [Boolean] term language, which identifies the connectives that
/// form conjunctions and disjunctions.
pub trait BooleanChain: BinaryChain {
    type Owned: Boolean;

    fn conjunction() -> Self::Op;
    fn disjunction() -> Self::Op;

    /// The term this handle refers to.
    fn into_owned(self) -> Self::Owned;
}

/// Simplifies the conjunction chain rooted at `root` as a whole, rather than only its outermost
/// operands: it is `false` as soon as any operand is, and `true` operands are dropped.
pub fn optimized_conjunction_chain<H: BooleanChain, U>(root: H, storage: &mut ChainStack<U>) -> H::Owned {
    optimized_conjunction(ChainIter::new(root, H::conjunction(), storage).map(H::into_owned))
}

/// Simplifies the disjunction chain rooted at `root` as a whole, rather than only its outermost
/// operands: it is `true` as soon as any operand is, and `false` operands are dropped.
pub fn optimized_disjunction_chain<H: BooleanChain, U>(root: H, storage: &mut ChainStack<U>) -> H::Owned {
    optimized_disjunction(ChainIter::new(root, H::disjunction(), storage).map(H::into_owned))
}
