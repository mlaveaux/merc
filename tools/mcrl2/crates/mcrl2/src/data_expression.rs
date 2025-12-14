#![allow(dead_code)]
use std::fmt;

use mcrl2_sys::data::ffi::mcrl2_data_expression_is_abstraction;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_application;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_data_expression;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_function_symbol;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_machine_number;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_untyped_identifier;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_variable;
use mcrl2_sys::data::ffi::mcrl2_data_expression_is_where_clause;

use crate::ATerm;
use crate::ATermString;
use crate::DataSort;

/// Checks if this term is a data variable.
pub fn is_variable(term: &ATerm) -> bool {
    mcrl2_data_expression_is_variable(term.get())
}

/// Checks if this term is a data application.
pub fn is_application(term: &ATerm) -> bool {
    mcrl2_data_expression_is_application(term.get())
}

/// Checks if this term is a data abstraction.
pub fn is_abstraction(term: &ATerm) -> bool {
    mcrl2_data_expression_is_abstraction(term.get())
}

/// Checks if this term is a data function symbol.
pub fn is_function_symbol(term: &ATerm) -> bool {
    mcrl2_data_expression_is_function_symbol(term.get())
}

/// Checks if this term is a data where clause.
pub fn is_where_clause(term: &ATerm) -> bool {
    mcrl2_data_expression_is_where_clause(term.get())
}

/// Checks if this term is a data machine number.
pub fn is_machine_number(term: &ATerm) -> bool {
    mcrl2_data_expression_is_machine_number(term.get())
}

/// Checks if this term is a data untyped identifier.
pub fn is_untyped_identifier(term: &ATerm) -> bool {
    mcrl2_data_expression_is_untyped_identifier(term.get())
}

/// Checks if this term is a data expression.
pub fn is_data_expression(term: &ATerm) -> bool {
    mcrl2_data_expression_is_data_expression(term.get())
}

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
        DataSort::new(self.term.arg(1).protect())
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
        debug_assert!(mcrl2_data_expression_is_application(term.get()));
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
        debug_assert!(mcrl2_data_expression_is_abstraction(term.get()));
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
        debug_assert!(mcrl2_data_expression_is_function_symbol(term.get()));
        DataFunctionSymbol { term }
    }
}
