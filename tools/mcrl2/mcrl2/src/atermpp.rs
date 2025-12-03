use std::fmt;
use std::marker::PhantomData;

use mcrl2_sys::atermpp::ffi::aterm;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_list_size;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_to_string;
use mcrl2_sys::cxx::UniquePtr;

/// Represents a term from the mCRL2 toolset.
pub struct ATerm {
    term: UniquePtr<aterm>,
}

impl ATerm {
    /// Creates a new `Mcrl2AtermList` from the given term.
    pub(crate) fn new(term: UniquePtr<aterm>) -> Self {
        Self { term }
    }

    /// Returns a reference to the underlying term.
    pub fn get(&self) -> &aterm {
        self.term.as_ref().expect("ATerm is null")
    }
}

impl fmt::Debug for ATerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mcrl2ATerm {}", mcrl2_aterm_to_string(&self.term).unwrap())
    }
}

/// Represents a list of terms from the mCRL2 toolset.
pub struct AtermList<T> {
    term: ATerm,
    _marker: PhantomData<T>,
}

impl<T> AtermList<T> {
    /// Returns the length of the list.
    pub fn len(&self) -> usize {
        mcrl2_aterm_list_size(&self.term.get())
    }

    /// Converts the list to a `Vec<T>`.
    pub fn to_vec(&self) -> Vec<T>
    where
        T: From<ATerm>,
    {
        unimplemented!()
    }

    /// Creates a new `Mcrl2AtermList` from the given term.
    pub(crate) fn new(term: ATerm) -> Self {
        AtermList {
            term,
            _marker: PhantomData,
        }
    }
}
