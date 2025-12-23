use std::fmt;
use std::ops::Deref;

use ahash::AHashSet;
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
use merc_aterm::TermBuilder;
use merc_aterm::TermIterator;
use merc_aterm::Transmutable;
use merc_aterm::Yield;
use merc_aterm::storage::Marker;
use merc_aterm::storage::THREAD_TERM_POOL;
use merc_macros::merc_derive_terms;
use merc_macros::merc_ignore;
use merc_macros::merc_term;

use crate::DATA_SYMBOLS;
use crate::SortExpression;
use crate::SortExpressionRef;
use crate::is_data_application;
use crate::is_data_expression;
use crate::is_data_function_symbol;
use crate::is_data_machine_number;
use crate::is_data_variable;

// This module is only used internally to run the proc macro.
#[merc_derive_terms]
mod inner {

    use std::iter;

    use merc_aterm::ATermStringRef;
    use merc_utilities::MercError;

    use super::*;

    /// A data expression is an [merc_aterm::ATerm] with additional structure.
    ///
    /// # Details
    ///
    /// A data expression can be any of:
    ///     - a variable
    ///     - a function symbol, i.e. f without arguments.
    ///     - a term applied to a number of arguments, i.e., t_0(t1, ..., tn).
    ///     - an abstraction lambda x: Sort . e, or forall and exists.
    ///     - machine number, a value [0, ..., 2^64-1].
    ///
    /// Not supported:
    ///     - a where clause "e where [x := f, ...]"
    ///     - set enumeration
    ///     - bag enumeration
    ///
    #[merc_term(is_data_expression)]
    pub struct DataExpression {
        term: ATerm,
    }

    impl DataExpression {
        /// Returns the head symbol a data expression
        ///     - function symbol                  f -> f
        ///     - application       f(t_0, ..., t_n) -> f
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

        /// Returns the arguments of a data expression
        ///     - function symbol                  f -> []
        ///     - application       f(t_0, ..., t_n) -> [t_0, ..., t_n]
        #[merc_ignore]
        pub fn data_arguments(&self) -> impl ExactSizeIterator<Item = DataExpressionRef<'_>> + use<'_> {
            let mut result = self.term.arguments();
            if is_data_application(&self.term) {
                result.next();
            } else if is_data_function_symbol(&self.term) || is_data_variable(&self.term) {
                result.next();
                result.next();
            } else {
                // This can only happen if the term is an incorrect data expression.
                panic!("data_arguments not implemented for {self}");
            }

            result.map(|t| t.into())
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

        /// Returns the arguments of a data expression
        ///     - function symbol                  f -> []
        ///     - application       f(t_0, ..., t_n) -> [t_0, ..., t_n]
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
        pub fn new(name: impl Into<String> + AsRef<str>) -> DataFunctionSymbol {
            DATA_SYMBOLS.with_borrow(|ds| DataFunctionSymbol {
                term: ATerm::with_args(
                    ds.data_function_symbol.deref(),
                    &[
                        Into::<ATerm>::into(ATermString::new(name)),
                        SortExpression::unknown_sort().into(),
                    ],
                )
                .protect(),
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
        pub fn new(name: impl Into<ATermString>) -> DataVariable {
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
        pub fn with_sort(name: impl Into<ATermString>, sort: SortExpressionRef<'_>) -> DataVariable {
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
        pub fn with_args<'a, 'b>(head: &'b impl Term<'a, 'b>, arguments: &'b [impl Term<'a, 'b>]) -> DataApplication {
            DATA_SYMBOLS.with_borrow_mut(|ds| {
                let symbol = ds.get_data_application_symbol(arguments.len() + 1).copy();

                let args = iter::once(head.copy()).chain(arguments.iter().map(|t| t.copy()));
                let term = ATerm::with_iter(&symbol, args);

                DataApplication { term }
            })
        }

        /// Create a new data application with the given head and arguments.
        ///
        /// arity must be equal to the number of arguments + 1.
        #[merc_ignore]
        pub fn with_iter<'a, 'b, 'c, 'd, T, I>(
            head: &'b impl Term<'a, 'b>,
            arity: usize,
            arguments: I,
        ) -> DataApplication
        where
            I: Iterator<Item = T>,
            T: Term<'c, 'd>,
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

        /// Returns the sort of a data application.
        pub fn sort(&self) -> SortExpressionRef<'_> {
            // We only change the lifetime, but that is fine since it is derived from the current term.
            SortExpressionRef::from(self.term.arg(0))
        }
    }

    impl fmt::Display for DataApplication {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.data_function_symbol())?;

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
    struct MachineNumber {
        pub term: ATerm,
    }

    impl MachineNumber {
        /// Obtain the underlying value of a machine number.
        pub fn value(&self) -> u64 {
            self.term
                .copy()
                .annotation()
                .expect("MachineNumber must have an integer annotation") as u64
        }
    }

    impl fmt::Display for MachineNumber {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.value())
        }
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
    impl From<DataVariable> for DataExpression {
        fn from(value: DataVariable) -> Self {
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
    impl<'a> From<DataExpressionRef<'a>> for DataVariableRef<'a> {
        fn from(value: DataExpressionRef<'a>) -> Self {
            value.term.into()
        }
    }
}

pub use inner::*;

impl<'a> DataExpressionRef<'a> {
    pub fn data_arguments(&self) -> impl ExactSizeIterator<Item = DataExpressionRef<'a>> + use<'a> {
        let mut result = self.term.arguments();
        if is_data_application(&self.term) {
            result.next();
        } else if is_data_function_symbol(&self.term) || is_data_variable(&self.term) {
            result.next();
            result.next();
        } else {
            // This can only happen if the term is not a data expression.
            panic!("data_arguments not implemented for {self}");
        }

        result.map(|t| t.into())
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
    THREAD_TERM_POOL.with_borrow(|tp| {
        builder
            .evaluate(
                tp,
                t,
                |_tp, args, t| {
                    if variables.is_some_and(|v| v.contains(t.get_head_symbol().name())) {
                        // Convert a constant variable, for example 'x', into an untyped variable.
                        Ok(Yield::Term(DataVariable::new(t.get_head_symbol().name()).into()))
                    } else if t.get_head_symbol().arity() == 0 {
                        Ok(Yield::Term(DataFunctionSymbol::new(t.get_head_symbol().name()).into()))
                    } else {
                        // This is a function symbol applied to a number of arguments
                        let head = DataFunctionSymbol::new(t.get_head_symbol().name());

                        for arg in t.arguments() {
                            args.push(arg.protect());
                        }

                        Ok(Yield::Construct(head.into()))
                    }
                },
                |_tp, input, args| {
                    let arity = args.clone().count();
                    Ok(DataApplication::with_iter(&input, arity, args).into())
                },
            )
            .unwrap()
            .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use merc_aterm::ATerm;

    #[test]
    fn test_print() {
        let _ = merc_utilities::test_logger();

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
}
