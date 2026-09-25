use std::fmt;

use mcrl2_macros::mcrl2_derive_terms;

use crate::ATermRef;

pub(crate) fn is_aterm_string(term: &ATermRef<'_>) -> bool {
    term.get_head_symbol().arity() == 0
}

#[mcrl2_derive_terms]
mod inner {
    use mcrl2_macros::mcrl2_term;

    use crate::ATerm;
    use crate::ATermRef;
    use crate::Markable;
    use crate::Todo;
    use crate::is_aterm_string;

    /// Represents an atermpp::aterm_string from the mCRL2 toolset.
    #[mcrl2_term(is_aterm_string)]
    pub struct ATermString {
        term: ATerm,
    }

    impl ATermString {
        /// Returns the string value.
        pub fn str(&self) -> String {
            // The Rust::Str should ensure that this is a valid string.
            self.term.get_head_symbol().name().to_string()
        }
    }
}

pub use inner::ATermString;
pub use inner::ATermStringRef;

impl ATermStringRef<'static> {
    /// Creates a reference to the maximally shared aterm_string at `term`.
    ///
    /// Two occurrences of the same name are the same term, so the resulting
    /// reference can be used as a hash key that identifies a name without
    /// rendering it to a `String`.
    ///
    /// # Safety
    ///
    /// Requires: `term` is non-null, names an `aterm_string` (arity-0) node,
    /// and stays reachable from some GC root for as long as the caller
    /// actually *reads* through the returned reference — not merely for as
    /// long as whatever value `term` was borrowed from happens to be in
    /// scope syntactically (that owner may be a temporary already dropped by
    /// the time the reference is read), and not necessarily forever: the
    /// chosen lifetime `'static` is a type-level upper bound the caller
    /// promises to respect, not a runtime guarantee this function creates —
    /// unlike `ATerm::from_ptr`/`Symbol::from_ptr`, this is a bare reference
    /// and registers no new GC root of its own. See callers such as
    /// `merc_pbes::explore_pbes::name_key` for how that root is actually
    /// supplied (typically a longer-lived owning value, like a `Pbes` kept
    /// alive by the caller, that transitively keeps `term` reachable).
    /// Guarantees: the returned `ATermStringRef<'static>` is safe to copy,
    /// compare and hash unconditionally (no dereference), and safe to
    /// dereference (`str()`, `Display`) for as long as the precondition is
    /// upheld.
    pub unsafe fn from_address(term: *const crate::_aterm) -> ATermStringRef<'static> {
        // SAFETY: the caller upholds that the term stays live for `'static`.
        ATermStringRef::new(unsafe { ATermRef::new(term) })
    }
}

impl fmt::Display for ATermString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.str())
    }
}

impl fmt::Display for ATermStringRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.str())
    }
}
