use mcrl2_macros::mcrl2_derive_terms;
use mcrl2_sys::data::ffi::assignment_pair;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_abstraction;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_application;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_binder_bag_comp;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_binder_exists;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_binder_forall;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_binder_lambda;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_binder_set_comp;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_binder_untyped_set_bag_comp;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_data_expression;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_function_symbol;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_machine_number;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_untyped_identifier;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_variable;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_where_clause;
use mcrl2_sys::data::ffi::mcrl2_data_expression_remove_index;
use mcrl2_sys::data::ffi::mcrl2_data_expression_replace_variables;
use mcrl2_sys::data::ffi::mcrl2_data_expression_to_string;
use mcrl2_sys::data::ffi::mcrl2_is_data_sort_expression;
use mcrl2_sys::data::ffi::mcrl2_sort_expression_to_string;

use crate::ATerm;
use crate::ATermRef;

/// Strips the internal rewriter-only `OpId(name, sort, index)` markers from `term`.
pub fn remove_index(term: &ATermRef<'_>) -> ATerm {
    term.require_valid();
    ATerm::from_unique_ptr(mcrl2_data_expression_remove_index(term.get()))
}

/// Checks if this term is a data variable.
pub fn is_variable(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    mcrl2_data_expression_is_variable(term.get())
}

/// Checks if this term is a data application.
pub fn is_application(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    mcrl2_data_expression_is_application(term.get())
}

/// Checks if this term is a binding operator, i.e., lambda, forall, or exists.
pub(crate) fn is_binding_operator(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    is_lambda_binder(term)
        || is_forall_binder(term)
        || is_exists_binder(term)
        || is_set_comprehension_binder(term)
        || is_bag_comprehension_binder(term)
}

/// Checks if this term is a lambda binder
pub(crate) fn is_lambda_binder(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    mcrl2_data_expression_is_binder_lambda(term.get())
}

/// Checks if this term is a forall binder
pub(crate) fn is_forall_binder(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    mcrl2_data_expression_is_binder_forall(term.get())
}

/// Checks if this term is a exists binder
pub(crate) fn is_exists_binder(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    mcrl2_data_expression_is_binder_exists(term.get())
}

/// Checks if this term is a set comprehension binder
pub(crate) fn is_set_comprehension_binder(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    mcrl2_data_expression_is_binder_set_comp(term.get())
}

/// Checks if this term is a bag comprehension binder
pub(crate) fn is_bag_comprehension_binder(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    mcrl2_data_expression_is_binder_bag_comp(term.get())
}

/// Checks if this term is an untyped set or bag comprehension binder
pub(crate) fn is_untyped_set_bag_comprehension_binder(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    mcrl2_data_expression_is_binder_untyped_set_bag_comp(term.get())
}

/// Checks if this term is a data abstraction.
pub fn is_abstraction(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    mcrl2_data_expression_is_abstraction(term.get())
}

/// Checks if this term is a data function symbol.
pub fn is_function_symbol(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    mcrl2_data_expression_is_function_symbol(term.get())
}

/// Checks if this term is a data where clause.
pub fn is_where_clause(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    mcrl2_data_expression_is_where_clause(term.get())
}

/// Checks if this term is a data machine number.
pub fn is_machine_number(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    mcrl2_data_expression_is_machine_number(term.get())
}

/// Checks if this term is a data untyped identifier.
pub fn is_untyped_identifier(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    mcrl2_data_expression_is_untyped_identifier(term.get())
}

/// Checks if this term is a data expression.
pub(crate) fn is_data_expression(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    mcrl2_data_expression_is_data_expression(term.get())
}

/// Checks if this term is a sort expression.
pub(crate) fn is_sort_expression(term: &ATermRef<'_>) -> bool {
    term.require_valid();
    mcrl2_is_data_sort_expression(term.get())
}

// This module is only used internally to run the proc macro.
#[mcrl2_derive_terms]
mod inner {
    use std::fmt;

    use mcrl2_macros::mcrl2_term;

    use crate::ATerm;
    use crate::ATermArgs;
    use crate::ATermIntRef;
    use crate::ATermList;
    use crate::ATermRef;
    use crate::ATermStringRef;
    use crate::Markable;
    use crate::Todo;

    use super::is_abstraction;
    use super::is_application;
    use super::is_bag_comprehension_binder;
    use super::is_binding_operator;
    use super::is_data_expression;
    use super::is_exists_binder;
    use super::is_forall_binder;
    use super::is_function_symbol;
    use super::is_lambda_binder;
    use super::is_machine_number;
    use super::is_set_comprehension_binder;
    use super::is_sort_expression;
    use super::is_untyped_identifier;
    use super::is_untyped_set_bag_comprehension_binder;
    use super::is_variable;
    use super::is_where_clause;
    use super::mcrl2_data_expression_to_string;
    use super::mcrl2_sort_expression_to_string;

    /// Represents a data::data_expression from the mCRL2 toolset.
    ///  A data expression can be any of:
    ///     - a variable
    ///     - a function symbol, i.e. f without arguments.
    ///     - a term applied to a number of arguments, i.e., t_0(t1, ..., tn).
    ///     - an abstraction, binding a list of variables to an expression. Can be lambda, forall, or exists, set or bag comprehension.
    ///     - machine number, a value [0, ..., 2^64-1].
    ///
    /// Not supported:
    ///     - a where clause `e whr [x := f, ...]`
    ///     - set enumeration
    ///     - bag enumeration
    ///
    #[mcrl2_term(is_data_expression)]
    pub struct DataExpression {
        term: ATerm,
    }

    impl DataExpression {
        /// Returns the head symbol a data expression
        ///     - function symbol                  f -> f
        ///     - application       f(t_0, ..., t_n) -> f
        pub fn data_function_symbol(&self) -> DataFunctionSymbolRef<'_> {
            if is_application(&self.term) {
                // SAFETY: `arg(0)` is a direct subterm of `self.term`, so it is
                // a parent term.
                unsafe { self.term.arg(0).upgrade(&self.term) }.into()
            } else if is_function_symbol(&self.term) {
                self.term.copy().into()
            } else {
                panic!("data_function_symbol not implemented for {}", self);
            }
        }

        /// Returns the arguments of a data expression
        ///     - function symbol                  f -> []
        ///     - application       f(t_0, ..., t_n) -> [t_0, ..., t_n]
        pub fn data_arguments(&self) -> ATermArgs<'_> {
            if is_application(&self.term) {
                let mut result = self.term.arguments();
                result.next();
                result
            } else if is_function_symbol(&self.term) {
                Default::default()
            } else {
                panic!("data_arguments not implemented for {}", self);
            }
        }

        /// Returns the arguments of a data expression
        ///     - function symbol                  f -> []
        ///     - application       f(t_0, ..., t_n) -> [t_0, ..., t_n]
        pub fn data_sort(&self) -> SortExpression {
            if is_function_symbol(&self.term) {
                DataFunctionSymbolRef::from(self.term.copy()).sort().protect()
            } else if is_variable(&self.term) {
                DataVariableRef::from(self.term.copy()).sort().protect()
            } else {
                panic!("data_sort not implemented for {}", self);
            }
        }

        /// Pretty prints the data expression.
        pub fn pretty_print(&self) -> String {
            mcrl2_data_expression_to_string(self.term.get())
        }
    }

    impl fmt::Display for DataExpression {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            if is_function_symbol(&self.term) {
                write!(f, "{}", DataFunctionSymbolRef::from(self.term.copy()))
            } else if is_application(&self.term) {
                write!(f, "{}", DataApplicationRef::from(self.term.copy()))
            } else if is_variable(&self.term) {
                write!(f, "{}", DataVariableRef::from(self.term.copy()))
            } else if is_machine_number(&self.term) {
                write!(f, "{}", DataMachineNumberRef::from(self.term.copy()))
            } else {
                write!(f, "{}", self.term)
            }
        }
    }

    /// Represents a data::variable from the mCRL2 toolset.
    #[mcrl2_term(is_variable)]
    pub struct DataVariable {
        term: ATerm,
    }

    impl DataVariable {
        /// Returns the name of the variable.
        pub fn name(&self) -> ATermStringRef<'_> {
            self.term.arg(0).into()
        }

        /// Returns the sort of the variable.
        pub fn sort(&self) -> SortExpressionRef<'_> {
            self.term.arg(1).into()
        }
    }

    impl fmt::Display for DataVariable {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}: {}", self.name(), self.sort())
        }
    }

    /// Represents a data::application from the mCRL2 toolset.
    #[mcrl2_term(is_application)]
    pub struct DataApplication {
        term: ATerm,
    }

    impl DataApplication {
        /// Returns the head symbol a data application
        pub fn data_function_symbol(&self) -> DataFunctionSymbolRef<'_> {
            // SAFETY: `arg(0)` is a direct subterm of `self.term`, so it is a
            // parent term.
            unsafe { self.term.arg(0).upgrade(&self.term) }.into()
        }

        /// Returns the arguments of a data application
        pub fn data_arguments(&self) -> ATermArgs<'_> {
            let mut result = self.term.arguments();
            result.next();
            result
        }

        /// Returns the sort of a data application.
        ///
        /// The result sort of `f(t_0, ..., t_n)` is the codomain of `f`'s own
        /// sort when that sort is a function (arrow) sort, or `f`'s sort
        /// directly otherwise. This mirrors the `application` case of
        /// `data_expression::sort()` in the vendored mCRL2 C++ (`data.cpp`).
        pub fn sort(&self) -> SortExpressionRef<'_> {
            // SAFETY: `arg(0)` (the applied head symbol `f`) and its own
            // `arg(1)` (`f`'s sort) are transitive subterms of `self.term`;
            // when that sort is itself a `SortArrow(domain, codomain)` term,
            // its `arg(1)` (the codomain) is a transitive subterm too.
            // `self.term` is therefore a valid parent witness for widening
            // any of these borrows to `&self`.
            unsafe {
                if self.term.arg(0).arg(1).get_head_symbol().name() == "SortArrow" {
                    self.term.arg(0).arg(1).arg(1).upgrade(&self.term)
                } else {
                    self.term.arg(0).arg(1).upgrade(&self.term)
                }
            }
            .into()
        }
    }

    impl fmt::Display for DataApplication {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.data_function_symbol())?;

            // Write no brackets when there are no arguments.
            let mut first = true;
            for arg in self.data_arguments() {
                if !first {
                    write!(f, ", ")?;
                } else {
                    write!(f, "(")?;
                }

                write!(f, "{}", DataExpressionRef::from(arg.copy()))?;
                first = false;
            }

            if !first {
                write!(f, ")")?;
            }

            Ok(())
        }
    }

    /// Represents a binder operator constant from the mCRL2 toolset (e.g. `Lambda`, `Forall`, `Exists`).
    #[mcrl2_term(is_binding_operator)]
    pub struct DataBinder {
        term: ATerm,
    }

    impl DataBinder {
        pub fn is_lambda(&self) -> bool {
            is_lambda_binder(&self.term.copy())
        }

        pub fn is_forall(&self) -> bool {
            is_forall_binder(&self.term.copy())
        }

        pub fn is_exists(&self) -> bool {
            is_exists_binder(&self.term.copy())
        }

        pub fn is_set_comprehension(&self) -> bool {
            is_set_comprehension_binder(&self.term.copy())
        }

        pub fn is_bag_comprehension(&self) -> bool {
            is_bag_comprehension_binder(&self.term.copy())
        }

        pub fn is_untyped_set_bag_comprehension(&self) -> bool {
            is_untyped_set_bag_comprehension_binder(&self.term.copy())
        }
    }

    /// Represents a data::abstraction from the mCRL2 toolset.
    #[mcrl2_term(is_abstraction)]
    pub struct DataAbstraction {
        term: ATerm,
    }

    impl DataAbstraction {
        /// Returns the binder operator of the abstraction (e.g. `Lambda`, `Forall`, `Exists`).
        pub fn binding_operator(&self) -> DataBinderRef<'_> {
            // SAFETY: `arg(0)` is a direct subterm of `self.term`, so it is a
            // parent term.
            unsafe { self.term.arg(0).upgrade(&self.term) }.into()
        }

        /// Returns the list of variables
        pub fn variables(&self) -> ATermList<DataVariable> {
            self.term.arg(1).protect().into()
        }

        /// Returns the body of the abstraction.
        pub fn body(&self) -> DataExpressionRef<'_> {
            self.term.arg(2).into()
        }
    }

    /// Represents a data::function_symbol from the mCRL2 toolset.
    #[mcrl2_term(is_function_symbol)]
    pub struct DataFunctionSymbol {
        term: ATerm,
    }

    impl DataFunctionSymbol {
        /// Returns the sort of the function symbol.
        pub fn sort(&self) -> SortExpressionRef<'_> {
            self.term.arg(1).into()
        }

        /// Returns the name of the function symbol
        pub fn name(&self) -> &str {
            // SAFETY: the returned `&str` points into the global function-symbol
            // pool, which keeps it alive as long as `self.term` is protected.
            unsafe { std::mem::transmute::<&str, &str>(self.term.arg(0).get_head_symbol().name()) }
        }
    }

    impl fmt::Display for DataFunctionSymbol {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            if !self.is_default() {
                write!(f, "{}", self.name())
            } else {
                write!(f, "<default>")
            }
        }
    }

    /// Represents a data::sort_expression from the mCRL2 toolset.   
    #[mcrl2_term(is_sort_expression)]
    pub struct SortExpression {
        term: ATerm,
    }

    impl SortExpression {
        /// Returns the name of the sort.
        pub fn name(&self) -> &str {
            // SAFETY: the returned `&str` points into the global function-symbol
            // pool, which keeps it alive as long as `self.term` is protected.
            unsafe { std::mem::transmute::<&str, &str>(self.term.arg(0).get_head_symbol().name()) }
        }

        /// Pretty prints the sort expression, e.g. `Pos -> Bool` for a function sort.
        pub fn pretty_print(&self) -> String {
            mcrl2_sort_expression_to_string(self.term.get())
        }
    }

    impl fmt::Display for SortExpression {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.pretty_print())
        }
    }

    /// Represents a data::machine_number from the mCRL2 toolset.
    #[mcrl2_term(is_machine_number)]
    pub struct DataMachineNumber {
        pub term: ATerm,
    }

    impl DataMachineNumber {
        /// Obtain the underlying value of a machine number.
        pub fn value(&self) -> u64 {
            ATermIntRef::from(self.term.copy()).value()
        }
    }

    impl fmt::Display for DataMachineNumber {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.value())
        }
    }

    /// Represents a data::where_clause from the mCRL2 toolset.
    #[mcrl2_term(is_where_clause)]
    pub struct DataWhereClause {
        pub term: ATerm,
    }

    impl DataWhereClause {
        /// Returns the body of the where clause.
        pub fn body(&self) -> DataExpressionRef<'_> {
            self.term.arg(0).into()
        }

        /// Returns the declarations of the where clause as a list of assignments.
        /// Each element is an assignment term with `arg(0)` = bound variable and
        /// `arg(1)` = defining expression.
        pub fn declarations(&self) -> ATermList<ATerm> {
            self.term.arg(1).protect().into()
        }
    }

    /// Represents a data::untyped_identifier from the mCRL2 toolset.
    #[mcrl2_term(is_untyped_identifier)]
    pub struct DataUntypedIdentifier {
        pub term: ATerm,
    }
}

pub use inner::DataAbstraction;
pub use inner::DataAbstractionRef;
pub use inner::DataApplication;
pub use inner::DataApplicationRef;
pub use inner::DataBinder;
pub use inner::DataBinderRef;
pub use inner::DataExpression;
pub use inner::DataExpressionRef;
pub use inner::DataFunctionSymbol;
pub use inner::DataFunctionSymbolRef;
pub use inner::DataMachineNumber;
pub use inner::DataMachineNumberRef;
pub use inner::DataUntypedIdentifier;
pub use inner::DataUntypedIdentifierRef;
pub use inner::DataVariable;
pub use inner::DataVariableRef;
pub use inner::DataWhereClause;
pub use inner::DataWhereClauseRef;
pub use inner::SortExpression;
pub use inner::SortExpressionRef;

// A `DataExpressionRef` is just an address into the maximally shared term pool,
// so it can be freely copied. Liveness is the responsibility of whoever keeps
// the underlying term alive (for example a [`crate::Protected`] container).
impl Clone for DataExpressionRef<'_> {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for DataExpressionRef<'_> {}

impl DataExpressionRef<'static> {
    /// Creates a `'static` reference to a data expression from its raw maximally
    /// shared term address.
    ///
    /// # Safety
    ///
    /// The term at `term` must stay live for as long as the returned `'static`
    /// reference is used; in particular it must be held by a garbage-collection
    /// container such as [`crate::Protected`]. Otherwise garbage collection may
    /// free the term while the reference is still reachable, which is undefined
    /// behaviour.
    pub unsafe fn from_address(term: *const crate::_aterm) -> DataExpressionRef<'static> {
        // SAFETY: the caller upholds that the term stays live for `'static`.
        DataExpressionRef::new(unsafe { ATermRef::new(term) })
    }
}

/// Converts a substitution into the pairs of raw term addresses that the FFI
/// expects.
///
/// The pairs only borrow the terms, so `sigma` must be kept alive for the
/// duration of the call that consumes the result.
pub(crate) fn to_assignment_pairs(sigma: &[(DataExpression, DataExpression)]) -> Vec<assignment_pair> {
    sigma
        .iter()
        .map(|(lhs, rhs)| assignment_pair {
            lhs: lhs.address(),
            rhs: rhs.address(),
        })
        .collect()
}

/// Substitutes variables in a data expression according to the given substitution sigma.
pub fn substitute_variables(
    data_expression: &DataExpressionRef,
    sigma: Vec<(DataExpression, DataExpression)>,
) -> DataExpression {
    // Do not into_iter here, as we need to keep sigma alive for the call.
    let pairs = to_assignment_pairs(&sigma);

    DataExpression::new(ATerm::from_unique_ptr(mcrl2_data_expression_replace_variables(
        data_expression.get(),
        &pairs,
    )))
}

// Allowed conversions
impl From<DataVariable> for DataExpression {
    fn from(var: DataVariable) -> Self {
        Self::new(var.into())
    }
}

impl From<DataAbstraction> for DataExpression {
    fn from(var: DataAbstraction) -> Self {
        Self::new(var.into())
    }
}

impl From<DataApplication> for DataExpression {
    fn from(var: DataApplication) -> Self {
        Self::new(var.into())
    }
}

impl From<DataFunctionSymbol> for DataExpression {
    fn from(var: DataFunctionSymbol) -> Self {
        Self::new(var.into())
    }
}

impl From<DataMachineNumber> for DataExpression {
    fn from(var: DataMachineNumber) -> Self {
        Self::new(var.into())
    }
}

impl From<DataWhereClause> for DataExpression {
    fn from(var: DataWhereClause) -> Self {
        Self::new(var.into())
    }
}

impl From<DataUntypedIdentifier> for DataExpression {
    fn from(var: DataUntypedIdentifier) -> Self {
        Self::new(var.into())
    }
}

// Reference variants
impl<'a> From<DataVariableRef<'a>> for DataExpressionRef<'a> {
    fn from(var: DataVariableRef<'a>) -> Self {
        Self::new(var.into())
    }
}

impl<'a> From<DataAbstractionRef<'a>> for DataExpressionRef<'a> {
    fn from(var: DataAbstractionRef<'a>) -> Self {
        Self::new(var.into())
    }
}

impl<'a> From<DataApplicationRef<'a>> for DataExpressionRef<'a> {
    fn from(var: DataApplicationRef<'a>) -> Self {
        Self::new(var.into())
    }
}

impl<'a> From<DataFunctionSymbolRef<'a>> for DataExpressionRef<'a> {
    fn from(var: DataFunctionSymbolRef<'a>) -> Self {
        Self::new(var.into())
    }
}

impl<'a> From<DataMachineNumberRef<'a>> for DataExpressionRef<'a> {
    fn from(var: DataMachineNumberRef<'a>) -> Self {
        Self::new(var.into())
    }
}

impl<'a> From<DataWhereClauseRef<'a>> for DataExpressionRef<'a> {
    fn from(var: DataWhereClauseRef<'a>) -> Self {
        Self::new(var.into())
    }
}

impl<'a> From<DataUntypedIdentifierRef<'a>> for DataExpressionRef<'a> {
    fn from(var: DataUntypedIdentifierRef<'a>) -> Self {
        Self::new(var.into())
    }
}

// Allowed conversions
impl From<DataExpression> for DataVariable {
    fn from(var: DataExpression) -> Self {
        Self::new(var.into())
    }
}

impl From<DataExpression> for DataAbstraction {
    fn from(var: DataExpression) -> Self {
        Self::new(var.into())
    }
}

impl From<DataExpression> for DataApplication {
    fn from(var: DataExpression) -> Self {
        Self::new(var.into())
    }
}

impl From<DataExpression> for DataFunctionSymbol {
    fn from(var: DataExpression) -> Self {
        Self::new(var.into())
    }
}

impl From<DataExpression> for DataMachineNumber {
    fn from(var: DataExpression) -> Self {
        Self::new(var.into())
    }
}

impl From<DataExpression> for DataWhereClause {
    fn from(var: DataExpression) -> Self {
        Self::new(var.into())
    }
}

impl From<DataExpression> for DataUntypedIdentifier {
    fn from(var: DataExpression) -> Self {
        Self::new(var.into())
    }
}

// Reference variants
impl<'a> From<DataExpressionRef<'a>> for DataVariableRef<'a> {
    fn from(var: DataExpressionRef<'a>) -> Self {
        Self::new(var.into())
    }
}

impl<'a> From<DataExpressionRef<'a>> for DataAbstractionRef<'a> {
    fn from(var: DataExpressionRef<'a>) -> Self {
        Self::new(var.into())
    }
}

impl<'a> From<DataExpressionRef<'a>> for DataApplicationRef<'a> {
    fn from(var: DataExpressionRef<'a>) -> Self {
        Self::new(var.into())
    }
}

impl<'a> From<DataExpressionRef<'a>> for DataFunctionSymbolRef<'a> {
    fn from(var: DataExpressionRef<'a>) -> Self {
        Self::new(var.into())
    }
}

impl<'a> From<DataExpressionRef<'a>> for DataMachineNumberRef<'a> {
    fn from(var: DataExpressionRef<'a>) -> Self {
        Self::new(var.into())
    }
}

impl<'a> From<DataExpressionRef<'a>> for DataWhereClauseRef<'a> {
    fn from(var: DataExpressionRef<'a>) -> Self {
        Self::new(var.into())
    }
}

impl<'a> From<DataExpressionRef<'a>> for DataUntypedIdentifierRef<'a> {
    fn from(var: DataExpressionRef<'a>) -> Self {
        Self::new(var.into())
    }
}

/// Serializes a [`DataExpression`] as a syntax tree of its head function
/// symbols and arguments.
///
/// For example `f(a, b)` becomes:
///
/// ```json
/// { "symbol": "f", "sort": "A # B -> C", "args": [
///     { "symbol": "a", "sort": "A", "args": [] },
///     { "symbol": "b", "sort": "B", "args": [] }
/// ] }
/// ```
impl serde::Serialize for DataExpression {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.copy().serialize(serializer)
    }
}

impl serde::Serialize for DataExpressionRef<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("DataExpression", 3)?;

        if is_function_symbol(&self.term) || is_application(&self.term) {
            // A function symbol `f` or an application `f(t_0, ..., t_n)`: emit
            // the head symbol, its sort and recursively serialize the arguments.
            let function_symbol = self.data_function_symbol();
            state.serialize_field("symbol", function_symbol.name())?;
            state.serialize_field("sort", &function_symbol.sort().to_string())?;

            let arguments: Vec<DataExpressionRef<'_>> = self.data_arguments().map(DataExpressionRef::from).collect();
            state.serialize_field("args", &arguments)?;
        } else if is_variable(&self.term) {
            // A variable: emit just its name as the symbol and its sort
            // separately.
            let variable = DataVariableRef::from(self.term.copy());
            state.serialize_field("symbol", &variable.name().to_string())?;
            state.serialize_field("sort", &variable.sort().to_string())?;
            state.serialize_field("args", &[(); 0])?;
        } else {
            // A leaf without a data function symbol or sort, such as a machine
            // number. Use its displayed form as the symbol.
            state.serialize_field("symbol", &self.to_string())?;
            state.serialize_field("sort", &Option::<String>::None)?;
            state.serialize_field("args", &[(); 0])?;
        }

        state.end()
    }
}
