use std::cell::RefCell;
use std::mem::ManuallyDrop;
use std::ops::Deref;

use delegate::delegate;

use merc_aterm::ATerm;
use merc_aterm::ATermArgs;
use merc_aterm::ATermIndex;
use merc_aterm::ATermRef;
use merc_aterm::ATermString;
use merc_aterm::Markable;
use merc_aterm::Symb;
use merc_aterm::SymbolRef;
use merc_aterm::Term;
use merc_aterm::TermIterator;
use merc_aterm::Transmutable;
use merc_aterm::storage::Marker;
use merc_macros::merc_derive_terms;
use merc_macros::merc_ignore;
use merc_macros::merc_term;

use crate::BasicSort;
use crate::DataApplication;
use crate::DataExpression;
use crate::DataExpressionRef;
use crate::DataFunctionSymbol;
use crate::DataFunctionSymbolRef;
use crate::SortArrow;
use crate::SortExpression;
use crate::is_data_application;

/// Function symbol names for mCRL2's `Bool` connectives, exactly as they occur
/// in the standard prelude.
const AND: &str = "&&";
const OR: &str = "||";
const NOT: &str = "!";
const EQ: &str = "==";
const NEQ: &str = "!=";
const IMPLIES: &str = "=>";

fn bool_sort() -> SortExpression {
    SortExpression::from(BasicSort::new("Bool"))
}

/// Builds the function symbol named `name` with the function sort `arrow`.
fn function_symbol(name: &'static str, arrow: SortArrow) -> DataFunctionSymbol {
    let sort: SortExpression = arrow.into();
    DataFunctionSymbol::with_sort(name, sort.copy())
}

thread_local! {
    /// Canonical terms for mCRL2's `Bool` connectives, built once per thread so that
    /// `is_and`/`is_or`/`is_not`/`is_implies` reduce to a term comparison instead of
    /// rebuilding a sort and function symbol and comparing names on every call, and
    /// `make_and`/`make_or`/`make_not`/`make_implies` reuse the built symbol.
    static BOOL_SYMBOLS: RefCell<BoolSymbols> = RefCell::new(BoolSymbols::new());
}

/// `&&`, `||`, `!`, and `=>` always have the same (monomorphic) `Bool` sort, so each is built
/// once as the exact function symbol every application and comparison uses. `==` and `!=` are
/// polymorphic in their domain sort, so only their name is canonicalized; `is_equal` and
/// `is_not_equal` match on the name alone and ignore the domain, same as before.
///
/// All fields are wrapped in `ManuallyDrop` so that their destructors never run at thread
/// exit: a term must never be dropped after its owning thread-local term pool has already
/// been torn down, and thread-local destruction order is unspecified (see
/// [`crate::data_terms::DataSymbols`]).
struct BoolSymbols {
    and: ManuallyDrop<DataFunctionSymbol>,
    or: ManuallyDrop<DataFunctionSymbol>,
    not: ManuallyDrop<DataFunctionSymbol>,
    implies: ManuallyDrop<DataFunctionSymbol>,
    eq_name: ManuallyDrop<ATermString>,
    neq_name: ManuallyDrop<ATermString>,
}

impl BoolSymbols {
    fn new() -> Self {
        Self {
            and: ManuallyDrop::new(function_symbol(
                AND,
                SortArrow::new(&[bool_sort(), bool_sort()], bool_sort()),
            )),
            or: ManuallyDrop::new(function_symbol(
                OR,
                SortArrow::new(&[bool_sort(), bool_sort()], bool_sort()),
            )),
            not: ManuallyDrop::new(function_symbol(NOT, SortArrow::new(&[bool_sort()], bool_sort()))),
            implies: ManuallyDrop::new(function_symbol(
                IMPLIES,
                SortArrow::new(&[bool_sort(), bool_sort()], bool_sort()),
            )),
            eq_name: ManuallyDrop::new(ATermString::new(EQ)),
            neq_name: ManuallyDrop::new(ATermString::new(NEQ)),
        }
    }
}

/// Returns whether `term` is a data application to exactly `arity` arguments: its `DataAppl`
/// head symbol carries `arity + 1` children (the function symbol plus the arguments), which is
/// cheaper to check than materializing the argument list.
fn is_application_of_arity<'a, 'b, T: Term<'a, 'b>>(term: &'b T, arity: usize) -> bool {
    is_data_application(term) && term.get_head_symbol().arity() == arity + 1
}

/// Returns the function symbol applied by the data application `term`.
fn application_function_symbol<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> DataFunctionSymbolRef<'a> {
    term.arg(0).into()
}

/// Returns whether `term` is a conjunction (`lhs && rhs`).
pub fn is_and<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    is_application_of_arity(term, 2)
        && BOOL_SYMBOLS.with_borrow(|bs| application_function_symbol(term) == bs.and.copy())
}

/// Returns whether `term` is a disjunction (`lhs || rhs`).
pub fn is_or<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    is_application_of_arity(term, 2) && BOOL_SYMBOLS.with_borrow(|bs| application_function_symbol(term) == bs.or.copy())
}

/// Returns whether `term` is a negation (`!operand`).
pub fn is_not<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    is_application_of_arity(term, 1)
        && BOOL_SYMBOLS.with_borrow(|bs| application_function_symbol(term) == bs.not.copy())
}

/// Returns whether `term` is an implication (`lhs => rhs`).
pub fn is_implies<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    is_application_of_arity(term, 2)
        && BOOL_SYMBOLS.with_borrow(|bs| application_function_symbol(term) == bs.implies.copy())
}

/// Returns whether `term` is an equality (`lhs == rhs`), for any domain sort.
pub fn is_equal<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    is_application_of_arity(term, 2)
        && BOOL_SYMBOLS.with_borrow(|bs| application_function_symbol(term).name() == bs.eq_name.copy())
}

/// Returns whether `term` is a disequality (`lhs != rhs`), for any domain sort.
pub fn is_not_equal<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    is_application_of_arity(term, 2)
        && BOOL_SYMBOLS.with_borrow(|bs| application_function_symbol(term).name() == bs.neq_name.copy())
}

// This module is only used internally to run the proc macro.
#[merc_derive_terms]
mod inner {
    use super::*;

    /// A conjunction `lhs && rhs`.
    #[merc_term(is_and)]
    pub struct AndExpression {
        term: ATerm,
    }

    impl AndExpression {
        /// Builds `lhs && rhs`.
        #[merc_ignore]
        pub fn new(lhs: DataExpression, rhs: DataExpression) -> AndExpression {
            BOOL_SYMBOLS.with_borrow(|bs| AndExpression {
                term: DataApplication::with_args(bs.and.deref(), &[lhs, rhs]).into(),
            })
        }

        /// Returns the left-hand side.
        pub fn lhs(&self) -> DataExpressionRef<'_> {
            self.term.arg(1).into()
        }

        /// Returns the right-hand side.
        pub fn rhs(&self) -> DataExpressionRef<'_> {
            self.term.arg(2).into()
        }
    }

    #[merc_ignore]
    impl From<DataExpression> for AndExpression {
        fn from(value: DataExpression) -> Self {
            let term: ATerm = value.into();
            term.into()
        }
    }

    #[merc_ignore]
    impl From<AndExpression> for DataExpression {
        fn from(value: AndExpression) -> Self {
            value.term.into()
        }
    }

    /// A disjunction `lhs || rhs`.
    #[merc_term(is_or)]
    pub struct OrExpression {
        term: ATerm,
    }

    impl OrExpression {
        /// Builds `lhs || rhs`.
        #[merc_ignore]
        pub fn new(lhs: DataExpression, rhs: DataExpression) -> OrExpression {
            BOOL_SYMBOLS.with_borrow(|bs| OrExpression {
                term: DataApplication::with_args(bs.or.deref(), &[lhs, rhs]).into(),
            })
        }

        /// Returns the left-hand side.
        pub fn lhs(&self) -> DataExpressionRef<'_> {
            self.term.arg(1).into()
        }

        /// Returns the right-hand side.
        pub fn rhs(&self) -> DataExpressionRef<'_> {
            self.term.arg(2).into()
        }
    }

    #[merc_ignore]
    impl From<DataExpression> for OrExpression {
        fn from(value: DataExpression) -> Self {
            let term: ATerm = value.into();
            term.into()
        }
    }

    #[merc_ignore]
    impl From<OrExpression> for DataExpression {
        fn from(value: OrExpression) -> Self {
            value.term.into()
        }
    }

    /// A negation `!operand`.
    #[merc_term(is_not)]
    pub struct NotExpression {
        term: ATerm,
    }

    impl NotExpression {
        /// Builds `!operand`.
        #[merc_ignore]
        pub fn new(operand: DataExpression) -> NotExpression {
            BOOL_SYMBOLS.with_borrow(|bs| NotExpression {
                term: DataApplication::with_args(bs.not.deref(), &[operand]).into(),
            })
        }

        /// Returns the negated operand.
        pub fn operand(&self) -> DataExpressionRef<'_> {
            self.term.arg(1).into()
        }
    }

    #[merc_ignore]
    impl From<DataExpression> for NotExpression {
        fn from(value: DataExpression) -> Self {
            let term: ATerm = value.into();
            term.into()
        }
    }

    #[merc_ignore]
    impl From<NotExpression> for DataExpression {
        fn from(value: NotExpression) -> Self {
            value.term.into()
        }
    }

    /// An implication `lhs => rhs`.
    #[merc_term(is_implies)]
    pub struct ImpliesExpression {
        term: ATerm,
    }

    impl ImpliesExpression {
        /// Builds `lhs => rhs`.
        #[merc_ignore]
        pub fn new(lhs: DataExpression, rhs: DataExpression) -> ImpliesExpression {
            BOOL_SYMBOLS.with_borrow(|bs| ImpliesExpression {
                term: DataApplication::with_args(bs.implies.deref(), &[lhs, rhs]).into(),
            })
        }

        /// Returns the antecedent.
        pub fn lhs(&self) -> DataExpressionRef<'_> {
            self.term.arg(1).into()
        }

        /// Returns the consequent.
        pub fn rhs(&self) -> DataExpressionRef<'_> {
            self.term.arg(2).into()
        }
    }

    #[merc_ignore]
    impl From<DataExpression> for ImpliesExpression {
        fn from(value: DataExpression) -> Self {
            let term: ATerm = value.into();
            term.into()
        }
    }

    #[merc_ignore]
    impl From<ImpliesExpression> for DataExpression {
        fn from(value: ImpliesExpression) -> Self {
            value.term.into()
        }
    }

    /// An equality `lhs == rhs`, for some domain sort shared by both sides.
    #[merc_term(is_equal)]
    pub struct EqualExpression {
        term: ATerm,
    }

    impl EqualExpression {
        /// Builds `lhs == rhs`, where both sides have sort `domain`.
        #[merc_ignore]
        pub fn new(domain: SortExpression, lhs: DataExpression, rhs: DataExpression) -> EqualExpression {
            let symbol = function_symbol(EQ, SortArrow::new(&[domain.clone(), domain], bool_sort()));
            EqualExpression {
                term: DataApplication::with_args(&symbol, &[lhs, rhs]).into(),
            }
        }

        /// Returns the left-hand side.
        pub fn lhs(&self) -> DataExpressionRef<'_> {
            self.term.arg(1).into()
        }

        /// Returns the right-hand side.
        pub fn rhs(&self) -> DataExpressionRef<'_> {
            self.term.arg(2).into()
        }
    }

    #[merc_ignore]
    impl From<DataExpression> for EqualExpression {
        fn from(value: DataExpression) -> Self {
            let term: ATerm = value.into();
            term.into()
        }
    }

    #[merc_ignore]
    impl From<EqualExpression> for DataExpression {
        fn from(value: EqualExpression) -> Self {
            value.term.into()
        }
    }

    /// A disequality `lhs != rhs`, for some domain sort shared by both sides.
    #[merc_term(is_not_equal)]
    pub struct NotEqualExpression {
        term: ATerm,
    }

    impl NotEqualExpression {
        /// Builds `lhs != rhs`, where both sides have sort `domain`.
        #[merc_ignore]
        pub fn new(domain: SortExpression, lhs: DataExpression, rhs: DataExpression) -> NotEqualExpression {
            let symbol = function_symbol(NEQ, SortArrow::new(&[domain.clone(), domain], bool_sort()));
            NotEqualExpression {
                term: DataApplication::with_args(&symbol, &[lhs, rhs]).into(),
            }
        }

        /// Returns the left-hand side.
        pub fn lhs(&self) -> DataExpressionRef<'_> {
            self.term.arg(1).into()
        }

        /// Returns the right-hand side.
        pub fn rhs(&self) -> DataExpressionRef<'_> {
            self.term.arg(2).into()
        }
    }

    #[merc_ignore]
    impl From<DataExpression> for NotEqualExpression {
        fn from(value: DataExpression) -> Self {
            let term: ATerm = value.into();
            term.into()
        }
    }

    #[merc_ignore]
    impl From<NotEqualExpression> for DataExpression {
        fn from(value: NotEqualExpression) -> Self {
            value.term.into()
        }
    }
}

pub use inner::AndExpression;
pub use inner::AndExpressionRef;
pub use inner::EqualExpression;
pub use inner::EqualExpressionRef;
pub use inner::ImpliesExpression;
pub use inner::ImpliesExpressionRef;
pub use inner::NotEqualExpression;
pub use inner::NotEqualExpressionRef;
pub use inner::NotExpression;
pub use inner::NotExpressionRef;
pub use inner::OrExpression;
pub use inner::OrExpressionRef;

/// Builds `lhs && rhs`.
pub fn make_and(lhs: DataExpression, rhs: DataExpression) -> DataExpression {
    AndExpression::new(lhs, rhs).into()
}

/// Builds `lhs || rhs`.
pub fn make_or(lhs: DataExpression, rhs: DataExpression) -> DataExpression {
    OrExpression::new(lhs, rhs).into()
}

/// Builds `!operand`.
pub fn make_not(operand: DataExpression) -> DataExpression {
    NotExpression::new(operand).into()
}

/// Builds `lhs => rhs`.
pub fn make_implies(lhs: DataExpression, rhs: DataExpression) -> DataExpression {
    ImpliesExpression::new(lhs, rhs).into()
}

/// Builds `lhs == rhs`, where both sides have sort `domain`.
pub fn make_equal(domain: SortExpression, lhs: DataExpression, rhs: DataExpression) -> DataExpression {
    EqualExpression::new(domain, lhs, rhs).into()
}

/// Builds `lhs != rhs`, where both sides have sort `domain`.
pub fn make_not_equal(domain: SortExpression, lhs: DataExpression, rhs: DataExpression) -> DataExpression {
    NotEqualExpression::new(domain, lhs, rhs).into()
}
