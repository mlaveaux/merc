use std::fmt;

use mcrl2_sys::data::ffi::mcrl2_data_expression_to_string;
use mcrl2_sys::data::ffi::mcrl2_variable_name;
use mcrl2_sys::data::ffi::mcrl2_variable_sort;

use crate::Aterm;
use crate::AtermString;

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

/// Represents a data::sort from the mCRL2 toolset.
#[derive(PartialEq, Eq)]
pub struct DataSort {
    term: Aterm,
}

impl DataSort {
    /// Creates a new data::sort from the given term.
    pub(crate) fn new(term: Aterm) -> Self {
        DataSort {
            term,
        }
    }
}

impl fmt::Debug for DataSort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.term)
    }
}

/// Represents a data::data_expression from the mCRL2 toolset.
#[derive(Clone, PartialEq, Eq)]
pub struct DataExpression {
    term: Aterm,
}

impl DataExpression {
    /// Creates a new data::data_expression from the given term.
    pub(crate) fn new(term: Aterm) -> Self {
        DataExpression {
            term,
        }
    }
}

impl fmt::Debug for DataExpression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", mcrl2_data_expression_to_string(self.term.get()))
    }
}
