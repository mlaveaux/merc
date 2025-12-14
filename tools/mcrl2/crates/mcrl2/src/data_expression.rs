use std::fmt;

use mcrl2_sys::data::ffi::mcrl2_data_expression_is_abstraction;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_application;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_function_symbol;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_variable;

use crate::ATerm;
use crate::ATermString;
use crate::DataSort;

/// Represents a data::data_expression from the mCRL2 toolset.
#[derive(Clone, PartialEq, Eq)]
pub struct DataExpression {
    term: ATerm,
}

impl DataExpression {
    /// Creates a new data::data_expression from the given term.
    pub fn new(term: ATerm) -> Self {
        DataExpression { term }
    }

    /// Returns a reference to the underlying Aterm.
    pub fn get(&self) -> &ATerm {
        &self.term
    }
}

impl From<DataVariable> for DataExpression {
    fn from(var: DataVariable) -> Self {
        DataExpression::new(var.term)
    }
}

impl fmt::Debug for DataExpression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.term)
    }
}

/// Represents a data::variable from the mCRL2 toolset.
#[derive(Clone)]
pub struct DataVariable {
    term: ATerm,
}

impl DataVariable {
    /// Creates a new data::variable from the given aterm.
    pub fn new(term: ATerm) -> Self {
        debug_assert!(mcrl2_data_expression_is_variable(term.get()));
        DataVariable { term }
    }

    /// Returns the name of the variable.
    pub fn name(&self) -> ATermString {
        ATermString::new(self.term.arg(0).protect())
    }

    /// Returns the sort of the variable.
    pub fn sort(&self) -> DataSort {
        DataSort::new(self.term.arg(2).protect())
    }
}

impl fmt::Debug for DataVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {:?}", self.name(), self.sort())
    }
}

impl From<ATerm> for DataVariable {
    fn from(term: ATerm) -> Self {
        DataVariable::new(term)
    }
}

/// Represents a data::application from the mCRL2 toolset.
pub struct DataApplication {
    term: ATerm,
}

impl DataApplication {
    /// Creates a new data::application from the given term.
    pub(crate) fn new(term: ATerm) -> Self {
        debug_assert!(!mcrl2_data_expression_is_application(term.get()));
        DataApplication { term }
    }
}

/// Represents a data::abstraction from the mCRL2 toolset.
pub struct DataAbstraction {
    term: ATerm,
}

impl DataAbstraction {
    /// Creates a new data::abstraction from the given term.
    pub(crate) fn new(term: ATerm) -> Self {
        debug_assert!(!mcrl2_data_expression_is_abstraction(term.get()));
        DataAbstraction { term }
    }
}

/// Represents a data::function_symbol from the mCRL2 toolset.
pub struct DataFunctionSymbol {
    term: ATerm,
}

impl DataFunctionSymbol {
    /// Creates a new data::function_symbol from the given term.
    pub(crate) fn new(term: ATerm) -> Self {
        debug_assert!(!mcrl2_data_expression_is_function_symbol(term.get()));
        DataFunctionSymbol { term }
    }
}
