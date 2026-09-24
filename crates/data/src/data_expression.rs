use std::fmt;
use std::ops::Deref;

use ahash::AHashSet;
use delegate::delegate;

use merc_aterm::ATerm;
use merc_aterm::ATermArgs;
use merc_aterm::ATermIndex;
use merc_aterm::ATermList;
use merc_aterm::ATermRef;
use merc_aterm::ATermString;
use merc_aterm::Markable;
use merc_aterm::Symb;
use merc_aterm::SymbolRef;
use merc_aterm::Term;
use merc_aterm::TermBuilder;
use merc_aterm::TermIterator;
use merc_aterm::Transmutable;
use merc_aterm::Yield;
use merc_aterm::storage::Marker;
use merc_aterm::storage::THREAD_TERM_POOL;
use merc_macros::merc_derive_terms;
use merc_macros::merc_ignore;
use merc_macros::merc_term;

use crate::BasicSort;
use crate::DATA_SYMBOLS;
use crate::SortExpression;
use crate::SortExpressionRef;
use crate::is_data_application;
use crate::is_data_binder;
use crate::is_data_equation;
use crate::is_data_expression;
use crate::is_data_function_symbol;
use crate::is_data_machine_number;
use crate::is_data_variable;
use crate::is_data_where_clause;
use crate::is_data_whr_decl;

/// The kind of a binder in a `DataAbstraction`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BinderType {
    Lambda,
    Forall,
    Exists,
    SetComp,
    BagComp,
}

// This module is only used internally to run the proc macro.
#[merc_derive_terms]
mod inner {

    use std::iter;

    use merc_aterm::ATermInt;
    use merc_aterm::ATermIntRef;
    use merc_aterm::ATermStringRef;
    use merc_utilities::MercError;

    use super::*;

    /// A data expression: a variable, a nullary function symbol, an application of a function
    /// symbol to arguments, a machine number (a value in `[0, 2^64-1]`), a binder (lambda,
    /// forall/exists, or a set/bag comprehension), or a where clause.
    ///
    /// Set and bag enumeration literals are not represented. Methods that require a flat
    /// argument list (e.g. [`DataExpression::data_arguments`], [`DataExpression::data_sort`])
    /// panic for binders and where clauses.
    #[merc_term(is_data_expression)]
    pub struct DataExpression {
        term: ATerm,
    }

    impl DataExpression {
        /// Returns the head symbol of a data expression: itself for a function symbol, or the
        /// applied symbol for an application.
        ///
        /// # Panics
        ///
        /// Panics for variables, machine numbers, binders, and where clauses, which have no
        /// head symbol.
        pub fn data_function_symbol(&self) -> DataFunctionSymbolRef<'_> {
            if is_data_application(&self.term) {
                self.term.arg(0).into()
            } else if is_data_function_symbol(&self.term) {
                self.term.copy().into()
            } else {
                // This can only happen if the term is an incorrect data expression.
                panic!("data_function_symbol not implemented for {self}");
            }
        }

        /// Same as [DataExpression::data_function_symbol], but returns `None` for a variable
        /// instead of panicking.
        ///
        /// Pattern matching uses this to observe a symbol: a variable in the subject term has no
        /// head symbol and therefore matches no pattern position. The variable is only tested
        /// after the two cases that do have one, so the common path costs the same.
        ///
        /// # Panics
        ///
        /// Panics for machine numbers, binders, and where clauses.
        pub fn try_data_function_symbol(&self) -> Option<DataFunctionSymbolRef<'_>> {
            if is_data_application(&self.term) {
                Some(self.term.arg(0).into())
            } else if is_data_function_symbol(&self.term) {
                Some(self.term.copy().into())
            } else if is_data_variable(&self.term) {
                None
            } else {
                panic!("try_data_function_symbol not implemented for {self}");
            }
        }

        /// Returns the data sub-expressions of a data expression.
        ///     - function symbol                  f -> []
        ///     - variable                         x -> []
        ///     - machine number                   n -> []
        ///     - application       f(t_0, ..., t_n) -> [t_0, ..., t_n]
        ///
        /// # Panics
        ///
        /// Panics for binders and where clauses, which have structured sub-terms that do not
        /// map cleanly to a flat argument list.
        #[merc_ignore]
        pub fn data_arguments(&self) -> impl ExactSizeIterator<Item = DataExpressionRef<'_>> + use<'_> {
            let skip = data_argument_skip_count(&self.term)
                .unwrap_or_else(|| panic!("data_arguments is not defined for binders and where clauses: {self}"));
            self.term.arguments().skip(skip).map(|t| t.into())
        }

        /// Creates a closed [DataExpression] from a string, i.e., has no free variables.
        #[merc_ignore]
        pub fn from_string(text: &str) -> Result<DataExpression, MercError> {
            Ok(to_untyped_data_expression(ATerm::from_string(text)?, None))
        }

        /// Creates a [DataExpression] from a string with free untyped variables indicated by the set of names.
        #[merc_ignore]
        pub fn from_string_untyped(text: &str, variables: &AHashSet<String>) -> Result<DataExpression, MercError> {
            Ok(to_untyped_data_expression(ATerm::from_string(text)?, Some(variables)))
        }

        /// Returns the ith argument of a data application.
        #[merc_ignore]
        pub fn data_arg(&self, index: usize) -> DataExpressionRef<'_> {
            debug_assert!(is_data_application(self), "Term {self:?} is not a data application");
            debug_assert!(
                index + 1 < self.get_head_symbol().arity(),
                "data_arg({index}) is not defined for term {self:?}"
            );

            self.term.arg(index + 1).into()
        }

        /// Returns the sort of a data expression.
        ///
        /// Only defined for function symbols and variables. Panics for applications (the result
        /// sort requires traversing the SortArrow chain), machine numbers, binders, and where
        /// clauses.
        pub fn data_sort(&self) -> SortExpression {
            if is_data_function_symbol(&self.term) {
                DataFunctionSymbolRef::from(self.term.copy()).sort().protect()
            } else if is_data_variable(&self.term) {
                DataVariableRef::from(self.term.copy()).sort().protect()
            } else {
                panic!("data_sort not implemented for {self}");
            }
        }
    }

    impl fmt::Display for DataExpression {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            if is_data_function_symbol(&self.term) {
                write!(f, "{}", DataFunctionSymbolRef::from(self.term.copy()))
            } else if is_data_application(&self.term) {
                write!(f, "{}", DataApplicationRef::from(self.term.copy()))
            } else if is_data_variable(&self.term) {
                write!(f, "{}", DataVariableRef::from(self.term.copy()))
            } else if is_data_machine_number(&self.term) {
                write!(f, "{}", MachineNumberRef::from(self.term.copy()))
            } else {
                write!(f, "{}", self.term)
            }
        }
    }

    #[merc_term(is_data_function_symbol)]
    pub struct DataFunctionSymbol {
        term: ATerm,
    }

    impl DataFunctionSymbol {
        #[merc_ignore]
        pub fn new<N>(name: N) -> DataFunctionSymbol
        where
            N: Into<ATermString> + AsRef<str>,
        {
            DATA_SYMBOLS.with_borrow(|ds| DataFunctionSymbol {
                term: ATerm::with_args(
                    ds.data_function_symbol_no_index.deref(),
                    &[Into::<ATerm>::into(name.into()), SortExpression::unknown_sort().into()],
                )
                .protect(),
            })
        }

        /// Creates a function symbol with the given name and sort.
        #[merc_ignore]
        pub fn with_sort<N: Into<ATermString>>(name: N, sort: SortExpressionRef<'_>) -> DataFunctionSymbol {
            DATA_SYMBOLS.with_borrow(|ds| {
                let t = name.into();
                let args: &[ATermRef<'_>] = &[t.copy().into(), sort.into()];
                DataFunctionSymbol {
                    term: ATerm::with_args(ds.data_function_symbol_no_index.deref(), args).protect(),
                }
            })
        }

        /// Returns the name of the function symbol
        pub fn name(&self) -> ATermStringRef<'_> {
            ATermStringRef::from(self.term.arg(0))
        }

        /// Returns the sort of the function symbol.
        pub fn sort(&self) -> SortExpressionRef<'_> {
            self.term.arg(1).into()
        }

        /// Returns the internal operation id (a unique number) for the data::function_symbol.
        ///
        /// This is the term's pool index, which is only a stable identifier for indexed `OpId`
        /// symbols; it is not meaningful for the `OpIdNoIndex` variant.
        pub fn operation_id(&self) -> usize {
            self.term.index()
        }
    }

    impl fmt::Display for DataFunctionSymbol {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.name())
        }
    }

    #[merc_term(is_data_variable)]
    pub struct DataVariable {
        term: ATerm,
    }

    impl DataVariable {
        /// Create a new untyped variable with the given name.
        #[merc_ignore]
        pub fn new<N: Into<ATermString>>(name: N) -> DataVariable {
            DATA_SYMBOLS.with_borrow(|ds| {
                // TODO: Storing terms temporarily is not optimal.
                let t = name.into();
                let args: &[ATerm] = &[t.into(), SortExpression::unknown_sort().into()];

                DataVariable {
                    term: ATerm::with_args(ds.data_variable.deref(), args).protect(),
                }
            })
        }

        /// Create a variable with the given sort and name.
        pub fn with_sort<N: Into<ATermString>>(name: N, sort: SortExpressionRef<'_>) -> DataVariable {
            DATA_SYMBOLS.with_borrow(|ds| {
                // TODO: Storing terms temporarily is not optimal.
                let t = name.into();
                let args: &[ATermRef<'_>] = &[t.copy().into(), sort.into()];

                DataVariable {
                    term: ATerm::with_args(ds.data_variable.deref(), args).protect(),
                }
            })
        }

        /// Returns the name of the variable.
        pub fn name(&self) -> &str {
            // We only change the lifetime, but that is fine since it is derived from the current term.
            self.term.arg(0).get_head_symbol().name()
        }

        /// Returns the sort of the variable.
        pub fn sort(&self) -> SortExpressionRef<'_> {
            self.term.arg(1).into()
        }
    }

    impl fmt::Display for DataVariable {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.name())
        }
    }

    #[merc_term(is_data_application)]
    pub struct DataApplication {
        term: ATerm,
    }

    impl DataApplication {
        /// Create a new data application with the given head and arguments.
        #[merc_ignore]
        pub fn with_args<'a, 'b, H: Term<'a, 'b>, T: Term<'a, 'b>>(head: &'b H, arguments: &'b [T]) -> DataApplication {
            DATA_SYMBOLS.with_borrow_mut(|ds| {
                let symbol = ds.get_data_application_symbol(arguments.len() + 1).copy();

                let args = iter::once(head.copy()).chain(arguments.iter().map(|t| t.copy()));
                let term = ATerm::with_iter(&symbol, args);

                DataApplication { term }
            })
        }

        /// Create a new data application with the given head and arguments.
        ///
        /// `arity` must equal the number of elements the `arguments` iterator yields.
        #[merc_ignore]
        pub fn with_iter<'a, 'b, 'c, 'd, T, H, I>(head: &'b H, arity: usize, arguments: I) -> DataApplication
        where
            I: Iterator<Item = T>,
            T: Term<'c, 'd>,
            H: Term<'a, 'b>,
        {
            DATA_SYMBOLS.with_borrow_mut(|ds| {
                let symbol = ds.get_data_application_symbol(arity + 1).copy();

                let term = ATerm::with_iter_head(&symbol, head, arguments);

                DataApplication { term }
            })
        }

        /// Returns the head symbol a data application
        pub fn data_function_symbol(&self) -> DataFunctionSymbolRef<'_> {
            self.term.arg(0).into()
        }

        /// Returns the arguments of a data application
        pub fn data_arguments(&self) -> ATermArgs<'_> {
            let mut result = self.term.arguments();
            result.next();
            result
        }

        /// Returns the ith argument of a data application.
        pub fn data_arg(&self, index: usize) -> DataExpressionRef<'_> {
            debug_assert!(
                index + 1 < self.get_head_symbol().arity(),
                "data_arg({index}) is not defined for term {self:?}"
            );

            self.term.arg(index + 1).into()
        }
    }

    impl fmt::Display for DataApplication {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            // The head can be a term in a higher order application.
            write!(f, "{}", DataExpressionRef::from(self.term.arg(0)))?;

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

    #[merc_term(is_data_machine_number)]
    pub struct MachineNumber {
        pub term: ATerm,
    }

    impl MachineNumber {
        /// Builds a machine number data expression wrapping `value`.
        ///
        /// A machine number is stored as a raw [`merc_aterm::ATermInt`]; the
        /// `u64` value is reinterpreted as the platform integer bit pattern.
        #[merc_ignore]
        pub fn new(value: u64) -> MachineNumber {
            MachineNumber {
                term: ATermInt::new(value as usize).into(),
            }
        }

        /// Obtain the underlying value of a machine number.
        ///
        /// Assumes the term is an integer term, which is guaranteed by the constructor
        /// and [`is_data_machine_number`]. The cast reinterprets the stored `i64` bit
        /// pattern as `u64`, recovering values in `[0, 2^64-1]`.
        pub(crate) fn value(&self) -> u64 {
            Into::<ATermIntRef<'_>>::into(self.term.copy()).value() as u64
        }
    }

    impl fmt::Display for MachineNumber {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.value())
        }
    }

    /// A data equation. `condition -> lhs = rhs`. Not itself a data expression.
    #[merc_term(is_data_equation)]
    pub struct DataEquation {
        term: ATerm,
    }

    impl DataEquation {
        /// Builds the equation `variables. condition => lhs = rhs`. `condition: None`
        /// is an unconditional equation, encoded as the literal `true` — mCRL2's own
        /// `data_equation` class has no separate "no condition" state at this layer.
        #[merc_ignore]
        pub fn new(
            variables: &[DataVariable],
            condition: Option<DataExpression>,
            lhs: DataExpression,
            rhs: DataExpression,
        ) -> DataEquation {
            let condition = condition.unwrap_or_else(true_literal);
            DATA_SYMBOLS.with_borrow(|ds| {
                let variables: ATermList<DataVariable> = ATermList::from_double_iter(variables.iter().cloned());
                let args: [ATerm; 4] = [variables.into(), condition.into(), lhs.into(), rhs.into()];
                DataEquation {
                    term: ATerm::with_args(ds.data_equation_symbol.deref(), &args).protect(),
                }
            })
        }

        /// Returns the equation's bound variables.
        pub fn variables(&self) -> ATermList<DataVariable> {
            self.term.arg(0).into()
        }

        /// Returns the equation's condition, or `None` for an unconditional equation
        /// (the literal `true`).
        pub fn condition(&self) -> Option<DataExpressionRef<'_>> {
            let condition: DataExpressionRef<'_> = self.term.arg(1).into();
            if condition.protect() == true_literal() {
                None
            } else {
                Some(condition)
            }
        }

        /// Returns the left-hand side of the equation.
        pub fn lhs(&self) -> DataExpressionRef<'_> {
            self.term.arg(2).into()
        }

        /// Returns the right-hand side of the equation.
        pub fn rhs(&self) -> DataExpressionRef<'_> {
            self.term.arg(3).into()
        }
    }

    impl fmt::Display for DataEquation {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            if let Some(condition) = self.condition() {
                write!(f, "{condition} -> ")?;
            }
            write!(f, "{} = {}", self.lhs(), self.rhs())
        }
    }

    /// A data abstraction (lambda, forall/exists quantifier, or set/bag comprehension).
    /// Wire format: `Binder(binder_type, [var…], body)` — arity 3.
    #[merc_term(is_data_binder)]
    pub struct DataAbstraction {
        term: ATerm,
    }

    impl DataAbstraction {
        #[merc_ignore]
        pub fn new(binder: super::BinderType, variables: &[DataVariable], body: DataExpression) -> DataAbstraction {
            DATA_SYMBOLS.with_borrow(|ds| {
                let binder_sym = match binder {
                    super::BinderType::Lambda => ds.data_lambda_symbol.deref(),
                    super::BinderType::Forall => ds.data_forall_symbol.deref(),
                    super::BinderType::Exists => ds.data_exists_symbol.deref(),
                    super::BinderType::SetComp => ds.data_set_comprehension_symbol.deref(),
                    super::BinderType::BagComp => ds.data_bag_comprehension_symbol.deref(),
                };
                let empty: &[ATerm] = &[];
                let binder_term: ATerm = ATerm::with_args(binder_sym, empty).protect();
                let vars: ATermList<DataVariable> = ATermList::from_double_iter(variables.iter().cloned());
                let args: [ATerm; 3] = [binder_term, vars.into(), body.into()];
                DataAbstraction {
                    term: ATerm::with_args(ds.data_binder_symbol.deref(), &args).protect(),
                }
            })
        }

        /// Returns the kind of binder (lambda, forall/exists, or a set/bag comprehension).
        pub fn binder_type(&self) -> super::BinderType {
            let symbol = self.term.arg(0).get_head_symbol();
            DATA_SYMBOLS.with_borrow(|ds| {
                if symbol == ds.data_lambda_symbol.copy() {
                    super::BinderType::Lambda
                } else if symbol == ds.data_forall_symbol.copy() {
                    super::BinderType::Forall
                } else if symbol == ds.data_exists_symbol.copy() {
                    super::BinderType::Exists
                } else if symbol == ds.data_set_comprehension_symbol.copy() {
                    super::BinderType::SetComp
                } else if symbol == ds.data_bag_comprehension_symbol.copy() {
                    super::BinderType::BagComp
                } else {
                    unreachable!("A DataAbstraction always carries one of the five binder kinds")
                }
            })
        }

        /// Returns the binder's bound variables.
        pub fn variables(&self) -> ATermList<DataVariable> {
            self.term.arg(1).into()
        }

        /// Returns the binder's body.
        pub fn body(&self) -> DataExpressionRef<'_> {
            self.term.arg(2).into()
        }
    }

    impl fmt::Display for DataAbstraction {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.term)
        }
    }

    /// A single where-clause binding `identifier := expr`.
    /// Wire format: `WhrDecl(DataVarId(name, sort), DataExpression)` — arity 2.
    #[merc_term(is_data_whr_decl)]
    pub struct DataWhrDecl {
        term: ATerm,
    }

    impl DataWhrDecl {
        #[merc_ignore]
        pub fn new(variable: DataVariable, expr: DataExpression) -> DataWhrDecl {
            DATA_SYMBOLS.with_borrow(|ds| {
                let args: [ATerm; 2] = [variable.into(), expr.into()];
                DataWhrDecl {
                    term: ATerm::with_args(ds.data_whr_decl_symbol.deref(), &args).protect(),
                }
            })
        }
    }

    impl fmt::Display for DataWhrDecl {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.term)
        }
    }

    /// A where clause `body whr [x := e, …] end`.
    /// Wire format: `Where(body, [WhrDecl…])` — arity 2.
    #[merc_term(is_data_where_clause)]
    pub struct DataWhereClause {
        term: ATerm,
    }

    impl DataWhereClause {
        #[merc_ignore]
        pub fn new(body: DataExpression, assignments: &[DataWhrDecl]) -> DataWhereClause {
            DATA_SYMBOLS.with_borrow(|ds| {
                let list: ATermList<DataWhrDecl> = ATermList::from_double_iter(assignments.iter().cloned());
                let args: [ATerm; 2] = [body.into(), list.into()];
                DataWhereClause {
                    term: ATerm::with_args(ds.data_where_clause.deref(), &args).protect(),
                }
            })
        }
    }

    impl fmt::Display for DataWhereClause {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.term)
        }
    }

    /// The canonical `Bool` literal `true` (mCRL2's `sort_bool::true_`), used as the
    /// wire-format placeholder for an unconditional equation's condition.
    #[merc_ignore]
    fn true_literal() -> DataExpression {
        DataFunctionSymbol::with_sort("true", SortExpression::from(BasicSort::new("Bool")).copy()).into()
    }

    /// Conversions to `DataExpression`
    #[merc_ignore]
    impl From<DataFunctionSymbol> for DataExpression {
        fn from(value: DataFunctionSymbol) -> Self {
            value.term.into()
        }
    }

    #[merc_ignore]
    impl From<DataApplication> for DataExpression {
        fn from(value: DataApplication) -> Self {
            value.term.into()
        }
    }

    #[merc_ignore]
    impl From<DataAbstraction> for DataExpression {
        fn from(value: DataAbstraction) -> Self {
            value.term.into()
        }
    }

    #[merc_ignore]
    impl From<DataWhereClause> for DataExpression {
        fn from(value: DataWhereClause) -> Self {
            value.term.into()
        }
    }

    #[merc_ignore]
    impl From<DataVariable> for DataExpression {
        fn from(value: DataVariable) -> Self {
            value.term.into()
        }
    }

    #[merc_ignore]
    impl From<MachineNumber> for DataExpression {
        fn from(value: MachineNumber) -> Self {
            value.term.into()
        }
    }

    #[merc_ignore]
    impl From<DataExpression> for DataFunctionSymbol {
        fn from(value: DataExpression) -> Self {
            value.term.into()
        }
    }

    #[merc_ignore]
    impl From<DataExpression> for DataVariable {
        fn from(value: DataExpression) -> Self {
            value.term.into()
        }
    }

    #[merc_ignore]
    impl From<DataExpression> for DataAbstraction {
        fn from(value: DataExpression) -> Self {
            value.term.into()
        }
    }

    #[merc_ignore]
    impl<'a> From<DataExpressionRef<'a>> for DataVariableRef<'a> {
        fn from(value: DataExpressionRef<'a>) -> Self {
            value.term.into()
        }
    }
}

pub use inner::DataAbstraction;
pub use inner::DataApplication;
pub use inner::DataApplicationRef;
pub use inner::DataEquation;
pub use inner::DataExpression;
pub use inner::DataExpressionRef;
pub use inner::DataFunctionSymbol;
pub use inner::DataFunctionSymbolRef;
pub use inner::DataVariable;
pub use inner::DataVariableRef;
pub use inner::DataWhereClause;
pub use inner::DataWhrDecl;
pub use inner::MachineNumber;
pub use inner::MachineNumberRef;

/// Returns the number of leading `ATerm` arguments that are *not* data sub-expressions and
/// must therefore be skipped by `data_arguments`, or `None` for binders/where clauses which
/// have no flat argument list.
///
///   - application `f(t_0, ..., t_n)` -> skip 1 (the head function symbol)
///   - function symbol / variable     -> skip 2 (name and sort)
///   - machine number                 -> skip 0 (int terms carry no `ATerm` children)
fn data_argument_skip_count<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> Option<usize> {
    if is_data_application(term) {
        Some(1)
    } else if is_data_function_symbol(term) || is_data_variable(term) {
        Some(2)
    } else if is_data_machine_number(term) {
        Some(0)
    } else {
        None
    }
}

impl<'a> DataExpressionRef<'a> {
    pub fn data_arguments(&self) -> impl ExactSizeIterator<Item = DataExpressionRef<'a>> + use<'a> {
        let skip = data_argument_skip_count(&self.term)
            .unwrap_or_else(|| panic!("data_arguments is not defined for binders and where clauses: {self}"));
        self.term.arguments().skip(skip).map(|t| t.into())
    }

    /// Returns the ith argument of a data application.
    pub fn data_arg(&self, index: usize) -> DataExpressionRef<'a> {
        debug_assert!(is_data_application(self), "Term {self:?} is not a data application");
        debug_assert!(
            index + 1 < self.get_head_symbol().arity(),
            "data_arg({index}) is not defined for term {self:?}"
        );

        self.term.arg(index + 1).into()
    }
}

/// Converts an [ATerm] to an untyped data expression.
pub fn to_untyped_data_expression(t: ATerm, variables: Option<&AHashSet<String>>) -> DataExpression {
    let mut builder = TermBuilder::<ATerm, ATerm>::new();
    THREAD_TERM_POOL.with(|tp| {
        builder
            .evaluate(
                tp,
                t,
                |_tp, args, t| {
                    let name = t.get_head_symbol().name();
                    if t.get_head_symbol().arity() == 0 {
                        if variables.is_some_and(|v| v.contains(name)) {
                            // Convert a constant identifier, for example 'x', into an untyped variable.
                            Ok(Yield::Term(DataVariable::new(name).into()))
                        } else {
                            Ok(Yield::Term(DataFunctionSymbol::new(name).into()))
                        }
                    } else {
                        // This is a function symbol applied to a number of arguments. Variables are
                        // only recognised in nullary position, so an applied identifier keeps its
                        // arguments instead of being silently collapsed to a variable.
                        let head = DataFunctionSymbol::new(name);

                        for arg in t.arguments() {
                            args.push(arg.protect());
                        }

                        Ok(Yield::Construct(head.into()))
                    }
                },
                |_tp, input, args| {
                    let args: Vec<ATerm> = args.cloned().collect();
                    Ok(DataApplication::with_args(&input, &args).into())
                },
            )
            .unwrap()
            .into()
    })
}

#[cfg(test)]
mod tests {
    use ahash::AHashSet;
    use merc_aterm::ATerm;
    use merc_aterm::ATermInt;

    use crate::is_data_application;
    use crate::is_data_equation;
    use crate::is_data_machine_number;
    use crate::is_data_variable;

    use crate::BasicSort;
    use crate::SortExpression;

    use super::DataApplication;
    use super::DataEquation;
    use super::DataExpression;
    use super::DataFunctionSymbol;
    use super::DataVariable;

    #[test]
    fn test_function_symbol_with_sort() {
        let sort: SortExpression = BasicSort::new("Nat").into();
        let f = DataFunctionSymbol::with_sort("f", sort.copy());

        assert_eq!(f.name(), "f");
        assert_eq!(f.sort().protect(), sort);
    }

    #[test]
    fn test_print() {
        merc_utilities::test_logger();

        let a = DataFunctionSymbol::new("a");
        assert_eq!("a", format!("{}", a));

        // Check printing of data applications.
        let f = DataFunctionSymbol::new("f");
        let appl = DataApplication::with_args(&f, &[a]);
        assert_eq!("f(a)", format!("{}", appl));
    }

    #[test]
    fn test_recognizers() {
        let a = DataFunctionSymbol::new("a");
        let f = DataFunctionSymbol::new("f");
        let appl = DataApplication::with_args(&f, &[a]);

        let term: ATerm = appl.into();
        assert!(is_data_application(&term));
    }

    #[test]
    fn test_data_arguments() {
        let a = DataFunctionSymbol::new("a");
        let f = DataFunctionSymbol::new("f");
        let appl = DataApplication::with_args(&f, &[a]);

        assert_eq!(appl.data_arguments().count(), 1);

        let data_expr: DataExpression = appl.clone().into();

        assert_eq!(data_expr.data_arguments().count(), 1);
    }

    #[test]
    fn test_to_data_expression() {
        let expression = DataExpression::from_string("s(s(a, b), c)").unwrap();

        assert_eq!(expression.data_arg(0).data_function_symbol().name(), "s");
        assert_eq!(expression.data_arg(0).data_arg(0).data_function_symbol().name(), "a");
    }

    #[test]
    fn test_machine_number() {
        let term: ATerm = ATermInt::new(42).into();
        assert!(is_data_machine_number(&term));

        let expr: DataExpression = term.into();
        assert_eq!(format!("{expr}"), "42");
        // Machine numbers have no data sub-expressions.
        assert_eq!(expr.data_arguments().count(), 0);
    }

    #[test]
    fn test_variable_sort() {
        let var = DataVariable::new("x");
        assert_eq!(var.name(), "x");

        let expr: DataExpression = var.into();
        assert!(is_data_variable(&expr));
        assert_eq!(expr.data_sort().name(), "@no_value@");
        assert_eq!(expr.data_arguments().count(), 0);
    }

    #[test]
    fn test_from_string_untyped_variable() {
        let vars = AHashSet::from_iter(["x".to_string()]);
        let expr = DataExpression::from_string_untyped("f(x, a)", &vars).unwrap();

        // 'x' is recognised as a variable, 'a' stays a function symbol.
        assert!(is_data_variable(&expr.data_arg(0)));
        assert_eq!(expr.data_arg(1).data_function_symbol().name(), "a");
    }

    #[test]
    fn test_from_string_untyped_applied_identifier_keeps_args() {
        // 'x' is in the variable set but appears applied; it must stay an application rather than
        // collapsing to a variable and silently dropping its argument.
        let vars = AHashSet::from_iter(["x".to_string()]);
        let expr = DataExpression::from_string_untyped("x(a)", &vars).unwrap();

        assert!(is_data_application(&expr));
        assert_eq!(expr.data_function_symbol().name(), "x");
        assert_eq!(expr.data_arguments().count(), 1);
    }

    #[test]
    fn test_data_equation_unconditional() {
        let sort: SortExpression = BasicSort::new("Nat").into();
        let x = DataVariable::with_sort("x", sort.copy());
        let lhs: DataExpression =
            DataApplication::with_args(&DataFunctionSymbol::new("f"), std::slice::from_ref(&x)).into();
        let rhs: DataExpression = x.clone().into();

        let equation = DataEquation::new(std::slice::from_ref(&x), None, lhs.clone(), rhs.clone());

        assert!(is_data_equation(&equation));
        assert!(equation.condition().is_none());
        assert_eq!(equation.lhs().protect(), lhs);
        assert_eq!(equation.rhs().protect(), rhs);
        assert_eq!(equation.variables().to_vec(), vec![x]);
        assert_eq!(format!("{equation}"), "f(x) = x");
    }

    #[test]
    fn test_data_equation_conditional() {
        let sort: SortExpression = BasicSort::new("Bool").into();
        let b = DataVariable::with_sort("b", sort.copy());
        let condition: DataExpression = b.clone().into();
        let lhs = DataFunctionSymbol::new("f").into();
        let rhs = DataFunctionSymbol::new("g").into();

        let equation = DataEquation::new(&[b], Some(condition), lhs, rhs);

        assert!(equation.condition().is_some());
        assert_eq!(format!("{equation}"), "b -> f = g");
    }
}
