use std::fmt;

use mcrl2_sys::atermpp::ffi::aterm;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_are_equal;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_clone;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_string_to_string;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_to_string;
use mcrl2_sys::cxx::UniquePtr;

// TODO: For this we could use the local protection set similar to the `mcrl2-rust` project.
/// Represents a atermpp::aterm from the mCRL2 toolset.
pub struct Aterm {
    term: UniquePtr<aterm>,
}

impl Aterm {
    /// Creates a new `Mcrl2AtermList` from the given term.
    pub(crate) fn new(term: UniquePtr<aterm>) -> Self {
        Self { term }
    }

    /// Returns a reference to the underlying term.
    pub fn get(&self) -> &aterm {
        self.term.as_ref().expect("ATerm is null")
    }
}

impl Clone for Aterm {
    fn clone(&self) -> Self {
        Aterm {
            term: mcrl2_aterm_clone(self.get()),
        }
    }
}

impl PartialEq for Aterm {
    fn eq(&self, other: &Self) -> bool {
        mcrl2_aterm_are_equal(self.get(), other.get())
    }
}

// The ordering is total.
impl Eq for Aterm {}

impl fmt::Debug for Aterm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", mcrl2_aterm_to_string(&self.term))
    }
}

/// Represents a atermpp::aterm_string from the mCRL2 toolset.
#[derive(PartialEq, Eq)]
pub struct AtermString {
    term: Aterm,
}

impl AtermString {
    /// Creates a new `ATermString` from the given term.
    pub(crate) fn new(term: Aterm) -> Self {
        Self { term }
    }
}

impl fmt::Debug for AtermString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", mcrl2_aterm_string_to_string(self.term.get()))
    }
}