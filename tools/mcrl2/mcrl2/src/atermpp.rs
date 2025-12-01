use std::marker::PhantomData;

use mcrl2_sys::atermpp::ffi::aterm;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_list_size;
use mcrl2_sys::cxx::UniquePtr;

/// Represents a term from the mCRL2 toolset.
pub struct Mcrl2ATerm {
    term: UniquePtr<aterm>,
}

impl Mcrl2ATerm {
    /// Creates a new `Mcrl2AtermList` from the given term.
    pub(crate) fn new(term: UniquePtr<aterm>) -> Self {
        Self { term }
    }

    /// Returns a reference to the underlying term.
    pub fn get(&self) -> &aterm {
        self.term.as_ref().expect("ATerm is null")
    }
}

pub struct Mcrl2PropositionVariable {
    term: Mcrl2ATerm,
}

impl Mcrl2PropositionVariable {
    pub fn parameters(&self) -> Mcrl2AtermList<Mcrl2ATerm> {
        unimplemented!()
    }

    /// Creates a new `Mcrl2PropositionVariable` from the given term.
    pub(crate) fn new(term: Mcrl2ATerm) -> Self {
        Mcrl2PropositionVariable { term }
    }
}

/// Represents a list of terms from the mCRL2 toolset.
pub struct Mcrl2AtermList<T> {
    term: Mcrl2ATerm,
    _marker: PhantomData<T>,
}

impl<T> Mcrl2AtermList<T> {
    /// Returns the length of the list.
    pub fn len(&self) -> usize {
        mcrl2_aterm_list_size(&self.term.get())
    }

    /// Converts the list to a `Vec<T>`.
    pub fn to_vec(&self) -> Vec<T>
    where
        T: From<Mcrl2ATerm>,
    {
        unimplemented!()
    }

    /// Creates a new `Mcrl2AtermList` from the given term.
    pub(crate) fn new(term: Mcrl2ATerm) -> Self {
        Mcrl2AtermList {
            term,
            _marker: PhantomData,
        }
    }
}
