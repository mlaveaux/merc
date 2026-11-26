use std::fmt;
use std::ops::Deref;

use ahash::AHashSet;
use delegate::delegate;

use merc_aterm::ATerm;
use merc_aterm::ATermArgs;
use merc_aterm::ATermIndex;
use merc_aterm::ATermInt;
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
use merc_aterm::is_int_term;
use merc_aterm::storage::Marker;
use merc_aterm::storage::THREAD_TERM_POOL;
use merc_macros::merc_derive_terms;
use merc_macros::merc_ignore;
use merc_macros::merc_term;

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
            match self.kind() {
                DataExpressionKind::Application(_) => self.term.arg(0).into(),
                DataExpressionKind::FunctionSymbol(symbol) => symbol,
                _ => panic!("data_function_symbol not implemented for {self}"),
            }
        }

        /// Same as [DataExpression::data_function_symbol], but returns `None` for a variable or
        /// a binder instead of panicking.
        ///
        /// Pattern matching uses this to observe a symbol: a variable or an undecided
        /// quantifier in the subject term has no head symbol and therefore matches no pattern
        /// position.
        ///
        /// # Panics
        ///
        /// Panics for machine numbers and where clauses.
        pub fn try_data_function_symbol(&self) -> Option<DataFunctionSymbolRef<'_>> {
            match self.kind() {
                DataExpressionKind::Application(_) => Some(self.term.arg(0).into()),
                DataExpressionKind::FunctionSymbol(symbol) => Some(symbol),
                DataExpressionKind::Variable(_) | DataExpressionKind::Abstraction(_) => None,
                DataExpressionKind::MachineNumber(_) | DataExpressionKind::WhereClause(_) => {
                    panic!("try_data_function_symbol not implemented for {self}")
                }
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
            let skip = data_argument_skip_count(&self.kind())
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
            match self.kind() {
                DataExpressionKind::FunctionSymbol(symbol) => symbol.sort().protect(),
                DataExpressionKind::Variable(variable) => variable.sort().protect(),
                _ => panic!("data_sort not implemented for {self}"),
            }
        }

        /// The top-level shape of this expression.
        #[merc_ignore]
        pub fn kind(&self) -> DataExpressionKind<'_> {
            self.copy().kind()
        }
    }

    impl fmt::Display for DataExpression {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self.kind() {
                DataExpressionKind::FunctionSymbol(symbol) => write!(f, "{symbol}"),
                DataExpressionKind::Application(application) => write!(f, "{application}"),
                DataExpressionKind::Variable(variable) => write!(f, "{variable}"),
                DataExpressionKind::MachineNumber(number) => write!(f, "{number}"),
                DataExpressionKind::Abstraction(_) | DataExpressionKind::WhereClause(_) => write!(f, "{}", self.term),
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

        /// Creates a function symbol with the given name and the function sort `domain -> codomain`.
        #[merc_ignore]
        pub fn with_signature<N: Into<ATermString>>(
            name: N,
            domain: &[SortExpression],
            codomain: &SortExpression,
        ) -> DataFunctionSymbol {
            let sort = SortExpression::from(crate::SortArrow::new(domain, codomain));
            DataFunctionSymbol::with_sort(name, sort.copy())
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

        /// The kind of binder.
        pub fn binder_type(&self) -> super::BinderType {
            DATA_SYMBOLS.with_borrow(|ds| ds.binder_type(&self.term.arg(0)))
        }

        /// The bound variables.
        pub fn variables(&self) -> ATermList<DataVariable> {
            ATermList::from(self.term.arg(1).protect())
        }

        /// The body, in which the variables are bound.
        pub fn body(&self) -> DataExpressionRef<'_> {
            self.term.arg(2).into()
        }
    }

    impl fmt::Display for DataAbstraction {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.term)
        }
    }

    /// A single where-clause binding `identifier := expr`. mCRL2 uses the same term for the
    /// assignments of a linear process.
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

        /// The variable this declaration binds.
        pub fn variable(&self) -> DataVariableRef<'_> {
            self.term.arg(0).into()
        }

        /// The value of the variable, which is not in the scope of the declarations.
        pub fn expression(&self) -> DataExpressionRef<'_> {
            self.term.arg(1).into()
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

        /// The body, in which the declared variables are bound.
        pub fn body(&self) -> DataExpressionRef<'_> {
            self.term.arg(0).into()
        }

        /// The declarations `x := e`.
        pub fn declarations(&self) -> ATermList<DataWhrDecl> {
            ATermList::from(self.term.arg(1).protect())
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
        DataExpression::bool_literal(true)
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
pub use inner::DataAbstractionRef;
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
pub use inner::DataWhereClauseRef;
pub use inner::DataWhrDecl;
pub use inner::DataWhrDeclRef;
pub use inner::MachineNumber;
pub use inner::MachineNumberRef;

/// The top-level shape of a [DataExpression], borrowing its subterms.
pub enum DataExpressionKind<'a> {
    Variable(DataVariableRef<'a>),
    FunctionSymbol(DataFunctionSymbolRef<'a>),
    /// An application `h(t_0, ..., t_n)`, where the head `h` is a function symbol or, in a
    /// higher-order term, any other data expression.
    Application(DataApplicationRef<'a>),
    MachineNumber(MachineNumberRef<'a>),
    /// A lambda, quantifier or set/bag comprehension.
    Abstraction(DataAbstractionRef<'a>),
    WhereClause(DataWhereClauseRef<'a>),
}

/// Returns the number of leading `ATerm` arguments that are *not* data sub-expressions and
/// must therefore be skipped by `data_arguments`, or `None` for binders/where clauses which
/// have no flat argument list.
///
///   - application `f(t_0, ..., t_n)` -> skip 1 (the head function symbol)
///   - function symbol / variable     -> skip 2 (name and sort)
///   - machine number                 -> skip 0 (int terms carry no `ATerm` children)
fn data_argument_skip_count(kind: &DataExpressionKind<'_>) -> Option<usize> {
    match kind {
        DataExpressionKind::Application(_) => Some(1),
        DataExpressionKind::FunctionSymbol(_) | DataExpressionKind::Variable(_) => Some(2),
        DataExpressionKind::MachineNumber(_) => Some(0),
        DataExpressionKind::Abstraction(_) | DataExpressionKind::WhereClause(_) => None,
    }
}

impl<'a> DataExpressionRef<'a> {
    /// The top-level shape of this expression. The subterms live as long as this reference, so
    /// that a traversal can keep them on an explicit stack.
    ///
    /// # Panics
    ///
    /// Panics if the term is not a data expression, which the conversion into one only checks
    /// in debug builds.
    pub fn kind(&self) -> DataExpressionKind<'a> {
        let term: ATermRef<'a> = self.term.copy();

        // Only the constructor is chosen while the symbols are borrowed: a conversion checks its
        // own predicate in debug builds, which borrows them again. An application is recognised
        // by a lookup on its arity, so it is tested after the single symbol comparisons.
        let construct = DATA_SYMBOLS.with_borrow_mut(|ds| -> fn(ATermRef<'a>) -> DataExpressionKind<'a> {
            if ds.is_data_variable(&term) {
                |term| DataExpressionKind::Variable(term.into())
            } else if ds.is_data_function_symbol(&term) {
                |term| DataExpressionKind::FunctionSymbol(term.into())
            } else if ds.is_data_machine_number(&term) {
                |term| DataExpressionKind::MachineNumber(term.into())
            } else if ds.is_data_application(&term) {
                |term| DataExpressionKind::Application(term.into())
            } else if ds.is_data_binder(&term) {
                |term| DataExpressionKind::Abstraction(term.into())
            } else if ds.is_data_where_clause(&term) {
                |term| DataExpressionKind::WhereClause(term.into())
            } else {
                panic!("not a data expression: {term}")
            }
        });

        construct(term)
    }

    pub fn data_arguments(&self) -> impl ExactSizeIterator<Item = DataExpressionRef<'a>> + use<'a> {
        let skip = data_argument_skip_count(&self.kind())
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
                    if is_int_term(&t) {
                        // Numerals such as '0' are untyped constants, like any other identifier.
                        return Ok(Yield::Term(
                            DataFunctionSymbol::new(ATermInt::from(t).value().to_string().as_str()).into(),
                        ));
                    }

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
            .expect("The closures never return an error")
            .into()
    })
}
