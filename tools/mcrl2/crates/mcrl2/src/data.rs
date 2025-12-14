use std::fmt;

use mcrl2_sys::cxx::UniquePtr;
use mcrl2_sys::data::ffi::RewriterJitty;
use mcrl2_sys::data::ffi::data_specification;
use mcrl2_sys::data::ffi::mcrl2_create_rewriter_jitty;

#[cfg(feature = "mcrl2_jittyc")]
use mcrl2_sys::data::ffi::RewriterCompilingJitty;
#[cfg(feature = "mcrl2_jittyc")]
use mcrl2_sys::data::ffi::mcrl2_create_rewriter_jittyc;

use crate::ATerm;

/// Represents a data::sort from the mCRL2 toolset.
#[derive(PartialEq, Eq)]
pub struct DataSort {
    term: ATerm,
}

impl DataSort {
    /// Creates a new data::sort from the given term.
    pub(crate) fn new(term: ATerm) -> Self {
        DataSort { term }
    }
}

impl fmt::Debug for DataSort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.term)
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
    _rewriter: UniquePtr<RewriterJitty>,
}

impl Mcrl2RewriterJitty {
    /// Creates a new Jitty rewriter from the given data specification.
    pub fn new(data_spec: &DataSpecification) -> Self {
        let rewriter = mcrl2_create_rewriter_jitty(data_spec.get());
        Self { _rewriter: rewriter }
    }
}

#[cfg(feature = "mcrl2_jittyc")]
/// Represents a mcrl2::data::detail::RewriterJittyCompiling from the mCRL2 toolset.
pub struct Mcrl2RewriterJittyCompiling {
    rewriter: UniquePtr<RewriterCompilingJitty>,
}

#[cfg(feature = "mcrl2_jittyc")]
impl Mcrl2RewriterJittyCompiling {
    /// Creates a new compiling Jitty rewriter from the given data specification.
    pub fn new(data_spec: &DataSpecification) -> Self {
        let rewriter = mcrl2_create_rewriter_jittyc(data_spec.get());
        Self { rewriter }
    }
}
