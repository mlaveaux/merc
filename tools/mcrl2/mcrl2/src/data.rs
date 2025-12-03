use mcrl2_sys::data::ffi::mcrl2_data_is_variable;

use crate::ATerm;

pub struct DataVariable {
    variable: ATerm,
}

impl DataVariable {
    pub fn name(&self) -> &str {
        unimplemented!();
    }

    /// Creates a new data::variable from the given aterm.
    pub(crate) fn new(term: ATerm) -> Self {
        debug_assert!(
            mcrl2_data_is_variable(&term.get()),
            "The term {:?} is not a variable.",
            term
        );

        DataVariable { variable: term }
    }
}
