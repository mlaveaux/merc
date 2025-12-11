use std::fmt;

use mcrl2_sys::cxx::UniquePtr;
use mcrl2_sys::data::ffi::RewriterJitty;
use mcrl2_sys::data::ffi::data_specification;
use mcrl2_sys::data::ffi::mcrl2_create_rewriter_jitty;
use mcrl2_sys::data::ffi::mcrl2_data_expression_to_string;
use mcrl2_sys::data::ffi::mcrl2_sort_to_string;
use mcrl2_sys::data::ffi::mcrl2_variable_name;
use mcrl2_sys::data::ffi::mcrl2_variable_sort;

#[cfg(feature = "mcrl2_jittyc")]
use mcrl2_sys::data::ffi::mcrl2_create_rewriter_jittyc;
#[cfg(feature = "mcrl2_jittyc")]
use mcrl2_sys::data::ffi::RewriterCompilingJitty;

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
        write!(f, "{:?}", mcrl2_sort_to_string(self.term.get()))
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

pub struct DataSpecification {
    spec: UniquePtr<data_specification>,
}

impl DataSpecification {
    /// Creates a new data specification from the given UniquePtr.
    pub(crate) fn new(spec: UniquePtr<data_specification>) -> Self {
        DataSpecification { spec }
    }

    /// Returns a reference to the underlying UniquePtr.
    pub(crate) fn get(&self) -> &UniquePtr<data_specification> {
        &self.spec
    }
}


/// Represents a mcrl2::data::detail::RewriterJitty from the mCRL2 toolset.
pub struct Mcrl2RewriterJitty {
    rewriter: UniquePtr<RewriterJitty>,
}

impl Mcrl2RewriterJitty {
    /// Creates a new Jitty rewriter from the given data specification.
    pub fn new(data_spec: &DataSpecification) -> Self {
        let rewriter = mcrl2_create_rewriter_jitty(data_spec.get());
        Self { rewriter }
    }
}

#[cfg(feature = "mcrl2_jittyc")]
/// Represents a mcrl2::data::detail::RewriterJittyCompiling from the mCRL2 toolset.
pub struct Mcrl2RewriterJittyCompiling {
    rewriter: UniquePtr<RewriterCompilingJitty>,
}

#[cfg(feature = "mcrl2_jittyc")]
impl Mcrl2RewriterJittyCompiling {
    /// Creates a new Jitty rewriter from the given data specification.
    pub fn new(data_spec: &DataSpecification) -> Self {
        let rewriter = mcrl2_create_rewriter_jittyc(data_spec.get());
        Self { rewriter }
    }
}