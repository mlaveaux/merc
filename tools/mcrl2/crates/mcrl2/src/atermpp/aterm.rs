use std::borrow::Borrow;
use std::cmp::Ordering;
use std::collections::VecDeque;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::marker::PhantomData;
use std::ops::Deref;

use mcrl2_sys::atermpp::ffi;
use mcrl2_sys::atermpp::ffi::aterm;
use mcrl2_sys::atermpp::ffi::_aterm;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_get_address;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_get_argument;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_get_function_symbol;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_is_empty_list;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_is_int;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_is_list;
use mcrl2_sys::atermpp::ffi::mcrl2_aterm_print;
use mcrl2_sys::cxx::UniquePtr;
use merc_utilities::PhantomUnsend;
use merc_utilities::ProtectionIndex;

use crate::atermpp::SymbolRef;
use crate::atermpp::THREAD_TERM_POOL;

/// This represents a lifetime bound reference to an existing ATerm that is
/// protected somewhere statically.
///
/// Can be 'static if the term is protected in a container or ATerm. That means
/// we either return &'a ATermRef<'static> or with a concrete lifetime
/// ATermRef<'a>. However, this means that the functions for ATermRef cannot use
/// the associated lifetime for the results parameters, as that would allow us
/// to acquire the 'static lifetime. This occasionally gives rise to issues
/// where we look at the argument of a term and want to return it's name, but
/// this is not allowed since the temporary returned by the argument is dropped.
#[derive(Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ATermRef<'a> {
    term: *const ffi::_aterm,
    marker: PhantomData<&'a ()>,
}

/// These are safe because terms are never modified. Garbage collection is
/// always performed with exclusive access and uses relaxed atomics to perform
/// some interior mutability.
unsafe impl Send for ATermRef<'_> {}
unsafe impl Sync for ATermRef<'_> {}

impl Default for ATermRef<'_> {
    fn default() -> Self {
        ATermRef {
            term: std::ptr::null(),
            marker: PhantomData,
        }
    }
}

impl<'a> ATermRef<'a> {
    /// Protects the reference on the thread local protection pool.
    pub fn protect(&self) -> ATerm {
        if self.is_default() {
            ATerm::default()
        } else {
            THREAD_TERM_POOL.with_borrow_mut(|tp| tp.protect(self.term))
        }
    }

    /// This allows us to extend our borrowed lifetime from 'a to 'b based on
    /// existing parent term which has lifetime 'b.
    ///
    /// The main usecase is to establish transitive lifetimes. For example given
    /// a term t from which we borrow `u = t.arg(0)` then we cannot have
    /// u.arg(0) live as long as t since the intermediate temporary u is
    /// dropped. However, since we know that u.arg(0) is a subterm of `t` we can
    /// upgrade its lifetime to the lifetime of `t` using this function.
    ///
    /// # Safety
    ///
    /// This function might only be used if witness is a parent term of the
    /// current term.
    pub fn upgrade<'b: 'a>(&'a self, parent: &ATermRef<'b>) -> ATermRef<'b> {
        debug_assert!(
            parent.iter().any(|t| t.copy() == *self),
            "Upgrade has been used on a witness that is not a parent term"
        );

        ATermRef::new(self.term)
    }

    /// A private unchecked version of [`ATermRef::upgrade`] to use in iterators.
    unsafe fn upgrade_unchecked<'b: 'a>(&'a self, _parent: &ATermRef<'b>) -> ATermRef<'b> {
        ATermRef::new(self.term)
    }

    /// Obtains the underlying pointer
    pub(crate) fn get(&self) -> &ffi::_aterm {
        self.require_valid();
        // # Safety
        //
        // If we have a reference to the ATermRef, it must also be safe
        // to dereference the pointer.
        unsafe { self.term.as_ref().expect("The pointer should be defined") }
    }
}

impl<'a> ATermRef<'a> {
    pub(crate) fn new(term: *const ffi::_aterm) -> ATermRef<'a> {
        ATermRef {
            term,
            marker: PhantomData,
        }
    }
}

impl ATermRef<'_> {
    /// Returns the indexed argument of the term
    pub fn arg(&self, index: usize) -> ATermRef<'_> {
        self.require_valid();
        debug_assert!(
            index < self.get_head_symbol().arity(),
            "arg({index}) is not defined for term {:?}",
            self
        );

        ATermRef {
            term: mcrl2_aterm_get_argument(self.get(), index),
            marker: PhantomData,
        }
    }

    /// Returns the list of arguments as a collection
    pub fn arguments(&self) -> ATermArgs<'_> {
        self.require_valid();

        ATermArgs::new(self.copy())
    }

    /// Makes a copy of the term with the same lifetime as itself.
    pub fn copy(&self) -> ATermRef<'_> {
        ATermRef::new(self.term)
    }

    /// Returns whether the term is the default term (not initialised)
    pub fn is_default(&self) -> bool {
        self.term.is_null()
    }

    /// Returns true iff this is an aterm_list
    pub fn is_list(&self) -> bool {
        mcrl2_aterm_is_list(self.get())
    }

    /// Returns true iff this is the empty aterm_list
    pub fn is_empty_list(&self) -> bool {
        mcrl2_aterm_is_empty_list(self.get())
    }

    /// Returns true iff this is a aterm_int
    pub fn is_int(&self) -> bool {
        mcrl2_aterm_is_int(self.get())
    }

    /// Returns the head function symbol of the term.
    pub fn get_head_symbol(&self) -> SymbolRef<'_> {
        mcrl2_aterm_get_function_symbol(self.get()).into()
    }

    /// Returns an iterator over all arguments of the term that runs in pre order traversal of the term trees.
    pub fn iter(&self) -> TermIterator<'_> {
        TermIterator::new(self.copy())
    }

    /// Panics if the term is default
    pub fn require_valid(&self) {
        debug_assert!(
            !self.is_default(),
            "This function can only be called on valid terms, i.e., not default terms"
        );
    }
}

impl fmt::Display for ATermRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.require_valid();
        write!(f, "{:?}", self)
    }
}

impl fmt::Debug for ATermRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_default() {
            write!(f, "None")?;
        } else {
            write!(f, "{}", mcrl2_aterm_print(self.get()))?;
        }

        Ok(())
    }
}

/// The protected version of [ATermRef], mostly derived from it.
#[derive(Default)]
pub struct ATerm {
    pub(crate) term: ATermRef<'static>,
    pub(crate) root: ProtectionIndex,

    // ATerm is not Send because it uses thread-local state for its protection
    // mechanism.
    _marker: PhantomUnsend,
}

impl ATerm {

    /// Constructs an ATerm from a UniquePtr<aterm>. Note that we still do the 
    /// protection here, so the term is copied into the thread local term pool.
    pub(crate) fn from_unique_ptr(term: UniquePtr<aterm>) -> Self {
        if term.is_null() {
            ATerm::default()
        } else {
            THREAD_TERM_POOL.with_borrow_mut(|tp| tp.protect(mcrl2_aterm_get_address(term.as_ref().expect("Pointer is valid"))))
        }
    }

    pub(crate) fn from_ptr(term: *const ffi::_aterm) -> Self {
        if term.is_null() {
            ATerm::default()
        } else {
            THREAD_TERM_POOL.with_borrow_mut(|tp| tp.protect(term))
        }
    }

    /// Obtains the underlying pointer
    pub(crate) fn get(&self) -> &_aterm {
        self.term.get()
    }

    /// Creates a new term from the given reference and protection set root
    /// entry.
    pub(crate) fn new(term: ATermRef<'static>, root: ProtectionIndex) -> ATerm {
        ATerm {
            term,
            root,
            _marker: PhantomData,
        }
    }

    /// Returns the address of the underlying aterm
    pub(crate) fn address(&self) -> *const ffi::_aterm {
        self.term.term
    }
}

impl Drop for ATerm {
    fn drop(&mut self) {
        if !self.is_default() {
            THREAD_TERM_POOL.with_borrow_mut(|tp| {
                tp.drop(self);
            })
        }
    }
}

impl Clone for ATerm {
    fn clone(&self) -> Self {
        self.copy().protect()
    }
}

impl Deref for ATerm {
    type Target = ATermRef<'static>;

    fn deref(&self) -> &Self::Target {
        &self.term
    }
}

impl<'a> Borrow<ATermRef<'a>> for ATerm {
    fn borrow(&self) -> &ATermRef<'a> {
        &self.term
    }
}

impl fmt::Display for ATerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.copy())
    }
}

impl fmt::Debug for ATerm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.copy())
    }
}

impl Hash for ATerm {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.term.hash(state)
    }
}

impl PartialEq for ATerm {
    fn eq(&self, other: &Self) -> bool {
        self.term.eq(&other.term)
    }
}

impl PartialOrd for ATerm {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.term.cmp(&other.term))
    }
}

impl Ord for ATerm {
    fn cmp(&self, other: &Self) -> Ordering {
        self.term.cmp(&other.term)
    }
}

impl Eq for ATerm {}

/// An iterator over the arguments of a term.
#[derive(Default)]
pub struct ATermArgs<'a> {
    term: ATermRef<'a>,
    arity: usize,
    index: usize,
}

impl<'a> ATermArgs<'a> {
    fn new(term: ATermRef<'a>) -> ATermArgs<'a> {
        let arity = term.get_head_symbol().arity();
        ATermArgs { term, arity, index: 0 }
    }

    pub fn is_empty(&self) -> bool {
        self.arity == 0
    }
}

impl<'a> Iterator for ATermArgs<'a> {
    type Item = ATermRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index < self.arity {
            let res = unsafe { Some(self.term.arg(self.index).upgrade_unchecked(&self.term)) };

            self.index += 1;
            res
        } else {
            None
        }
    }
}

impl DoubleEndedIterator for ATermArgs<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.index < self.arity {
            let res = unsafe { Some(self.term.arg(self.arity - 1).upgrade_unchecked(&self.term)) };

            self.arity -= 1;
            res
        } else {
            None
        }
    }
}

impl ExactSizeIterator for ATermArgs<'_> {
    fn len(&self) -> usize {
        self.arity - self.index
    }
}

/// An iterator over all subterms of the given [ATerm] in preorder traversal, i.e.,
/// for f(g(a), b) we visit f(g(a), b), g(a), a, b.
pub struct TermIterator<'a> {
    queue: VecDeque<ATermRef<'a>>,
}

impl TermIterator<'_> {
    pub fn new(t: ATermRef) -> TermIterator {
        TermIterator {
            queue: VecDeque::from([t]),
        }
    }
}

impl<'a> Iterator for TermIterator<'a> {
    type Item = ATermRef<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.queue.pop_back() {
            Some(term) => {
                // Put subterms in the queue
                for argument in term.arguments().rev() {
                    unsafe {
                        self.queue.push_back(argument.upgrade_unchecked(&term));
                    }
                }

                Some(term)
            }
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ATermList;
    use crate::atermpp::TermPool;

    use super::*;

    /// Make sure that the term has the same number of arguments as its arity.
    fn verify_term(term: &ATermRef<'_>) {
        for subterm in term.iter() {
            assert_eq!(
                subterm.get_head_symbol().arity(),
                subterm.arguments().len(),
                "The arity matches the number of arguments."
            )
        }
    }

    #[test]
    fn test_term_iterator() {
        let mut tp = TermPool::new();
        let t = tp.from_string("f(g(a),b)").unwrap();

        let mut result = t.iter();
        assert_eq!(result.next().unwrap(), tp.from_string("f(g(a),b)").unwrap().copy());
        assert_eq!(result.next().unwrap(), tp.from_string("g(a)").unwrap().copy());
        assert_eq!(result.next().unwrap(), tp.from_string("a").unwrap().copy());
        assert_eq!(result.next().unwrap(), tp.from_string("b").unwrap().copy());
    }

    #[test]
    fn test_aterm_list() {
        let mut tp = TermPool::new();
        let list: ATermList<ATerm> = tp.from_string("[f,g,h,i]").unwrap().into();

        assert!(!list.is_empty());

        // Convert into normal vector.
        let values: Vec<ATerm> = list.iter().collect();

        assert_eq!(values[0], tp.from_string("f").unwrap());
        assert_eq!(values[1], tp.from_string("g").unwrap());
        assert_eq!(values[2], tp.from_string("h").unwrap());
        assert_eq!(values[3], tp.from_string("i").unwrap());
    }
}
