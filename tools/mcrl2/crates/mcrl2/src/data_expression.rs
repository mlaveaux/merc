use std::fmt;

use mcrl2_sys::data::ffi::mcrl2_data_expression_is_abstraction;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_application;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_function_symbol;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_variable;
use mcrl2_sys::data::ffi::mcrl2_data_expression_to_string;
use mcrl2_sys::data::ffi::mcrl2_variable_name;
use mcrl2_sys::data::ffi::mcrl2_variable_sort;

use crate::Aterm;
use crate::AtermString;
use crate::DataSort;

/// Represents a data::data_expression from the mCRL2 toolset.
#[derive(Clone, PartialEq, Eq)]
pub struct DataExpression {
    term: Aterm,
}

impl DataExpression {
    /// Returns a reference to the underlying Aterm.
    pub fn get(&self) -> &Aterm {
        &self.term
    }

    /// Creates a new data::data_expression from the given term.
    pub(crate) fn new(term: Aterm) -> Self {
        DataExpression { term }
    }
}

impl From<DataVariable> for DataExpression {
    fn from(var: DataVariable) -> Self {
        DataExpression::new(var.term)
    }
}

impl fmt::Debug for DataExpression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", mcrl2_data_expression_to_string(self.term.get()))
    }
}

/// Represents a data::variable from the mCRL2 toolset.
#[derive(Clone)]
pub struct DataVariable {
    term: Aterm,
}

impl DataVariable {
    /// Returns the name of the variable.
    pub fn name(&self) -> AtermString {
        AtermString::new(Aterm::new(mcrl2_variable_name(self.term.get())))
    }

    /// Returns the sort of the variable.
    pub fn sort(&self) -> DataSort {
        DataSort::new(Aterm::new(mcrl2_variable_sort(self.term.get())))
    }

    /// Creates a new data::variable from the given aterm.
    pub(crate) fn new(term: Aterm) -> Self {
        debug_assert!(mcrl2_data_expression_is_variable(term.get()));
        DataVariable { term }
    }
}

impl fmt::Debug for DataVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {:?}", self.name(), self.sort())
    }
}

impl From<Aterm> for DataVariable {
    fn from(term: Aterm) -> Self {
        DataVariable::new(term)
    }
}

/// Represents a data::application from the mCRL2 toolset.
pub struct DataApplication {
    term: Aterm,
}

impl DataApplication {
    /// Creates a new data::application from the given term.
    pub(crate) fn new(term: Aterm) -> Self {
        debug_assert!(!mcrl2_data_expression_is_application(term.get()));
        DataApplication { term }
    }
}

/// Represents a data::abstraction from the mCRL2 toolset.
pub struct DataAbstraction {
    term: Aterm,
}

impl DataAbstraction {
    /// Creates a new data::abstraction from the given term.
    pub(crate) fn new(term: Aterm) -> Self {
        debug_assert!(!mcrl2_data_expression_is_abstraction(term.get()));
        DataAbstraction { term }
    }
}

/// Represents a data::function_symbol from the mCRL2 toolset.
pub struct DataFunctionSymbol {
    term: Aterm,
}

impl DataFunctionSymbol {
    /// Creates a new data::function_symbol from the given term.
    pub(crate) fn new(term: Aterm) -> Self {
        debug_assert!(!mcrl2_data_expression_is_function_symbol(term.get()));
        DataFunctionSymbol { term }
    }
}
