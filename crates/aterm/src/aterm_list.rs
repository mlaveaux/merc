//!
//! A list of terms, where T is the type of the elements in the list.
//!
#![forbid(unsafe_code)]

use std::fmt;
use std::marker::PhantomData;

use delegate::delegate;
use itertools::Itertools;
use merc_utilities::MercError;

use crate::ATerm;
use crate::ATermArgs;
use crate::ATermIndex;
use crate::ATermRef;
use crate::SymbolRef;
use crate::Term;
use crate::TermIterator;
use crate::storage::THREAD_TERM_POOL;

/// Returns true iff the term is a list term.
pub fn is_list_term<'a, 'b>(t: &'b impl Term<'a, 'b>) -> bool {
    THREAD_TERM_POOL.with_borrow(|tp| *tp.list_symbol() == t.get_head_symbol())
}

/// Returns true iff the term is an empty list.
pub fn is_empty_list_term<'a, 'b>(t: &'b impl Term<'a, 'b>) -> bool {
    THREAD_TERM_POOL.with_borrow(|tp| *tp.empty_list_symbol() == t.get_head_symbol())
}

/// Represents a list of ATerms of type T.
///
/// # Details
///
/// Internally, uses two standard function symbols `cons` and `[]` to represent
/// lists. The `cons` function symbol has arity 2, where the first argument is
/// the head of the list and the second argument is the tail of the list. The
/// `[]` function symbol has arity 0 and represents the empty list.
pub struct ATermList<T> {
    term: ATerm,
    _marker: PhantomData<T>,
}

// TODO: This should use the trait Term<'a, 'b>
impl<T: From<ATerm>> ATermList<T> {
    /// Obtain the head, i.e. the first element, of the list.
    pub fn head(&self) -> T {
        self.term.arg(0).protect().into()
    }

    /// Converts the list into a vector.
    pub fn to_vec(&self) -> Vec<T> {
        self.iter().collect()
    }
}

impl<T> ATermList<T> {
    /// Constructs a new list from an iterator that is consumed.
    pub fn from_double_iter(iter: impl DoubleEndedIterator<Item = T>) -> Self
    where
        T: Into<ATerm>,
    {
        let mut list = Self::empty();
        for item in iter.rev() {
            list = list.cons(item);
        }
        list
    }

    /// Constructs a new list from an iterator that is consumed.
    pub fn try_from_double_iter(iter: impl DoubleEndedIterator<Item = Result<T, MercError>>) -> Result<Self, MercError>
    where
        T: Into<ATerm>,
    {
        let mut list = Self::empty();
        for item in iter.rev() {
            list = list.cons(item?);
        }
        Ok(list)
    }

    /// Constructs a new list with the given item as the head and the current list as the tail.
    pub fn cons(&self, item: T) -> Self
    where
        T: Into<ATerm>,
    {
        ATermList {
            term: THREAD_TERM_POOL.with_borrow(|tp| {
                ATerm::with_args(tp.list_symbol(), &[item.into().copy(), self.term.copy()]).protect()
            }),
            _marker: PhantomData,
        }
    }

    /// Constructs the empty list.
    pub fn empty() -> Self {
        ATermList {
            term: THREAD_TERM_POOL.with_borrow(|tp| ATerm::constant(tp.empty_list_symbol())),
            _marker: PhantomData,
        }
    }

    /// Returns true iff the list is empty.
    pub fn is_empty(&self) -> bool {
        is_empty_list_term(&self.term)
    }

    /// Obtain the tail, i.e. the remainder, of the list.
    pub fn tail(&self) -> ATermList<T> {
        self.term.arg(1).into()
    }

    /// Returns an iterator over all elements in the list.
    pub fn iter(&self) -> ATermListIter<T> {
        ATermListIter { current: self.clone() }
    }
}

impl<'a, 'b, T> Term<'a, 'b> for ATermList<T>
where
    'b: 'a,
{
    delegate! {
        to self.term {
            fn protect(&self) -> ATerm;
            fn arg(&'b self, index: usize) -> ATermRef<'a>;
            fn arguments(&'b self) -> ATermArgs<'a>;
            fn copy(&'b self) -> ATermRef<'a>;
            fn get_head_symbol(&'b self) -> SymbolRef<'a>;
            fn iter(&'b self) -> TermIterator<'a>;
            fn index(&self) -> usize;
            fn shared(&self) -> &ATermIndex;
            fn annotation(&self) -> Option<usize>;
        }
    }
}

impl<T> Clone for ATermList<T> {
    fn clone(&self) -> Self {
        ATermList {
            term: self.term.clone(),
            _marker: PhantomData,
        }
    }
}

impl<T> From<ATermList<T>> for ATerm {
    fn from(value: ATermList<T>) -> Self {
        value.term
    }
}

impl<T: From<ATerm>> Iterator for ATermListIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current.is_empty() {
            None
        } else {
            let head = self.current.head();
            self.current = self.current.tail();
            Some(head)
        }
    }
}

impl<T> From<ATerm> for ATermList<T> {
    fn from(value: ATerm) -> Self {
        debug_assert!(
            is_list_term(&value) || is_empty_list_term(&value),
            "Can only convert an aterm_list"
        );
        ATermList::<T> {
            term: value,
            _marker: PhantomData,
        }
    }
}

impl<'a, T> From<ATermRef<'a>> for ATermList<T> {
    fn from(value: ATermRef<'a>) -> Self {
        debug_assert!(
            is_list_term(&value) || is_empty_list_term(&value),
            "Can only convert an aterm_list"
        );
        ATermList::<T> {
            term: value.protect(),
            _marker: PhantomData,
        }
    }
}

impl<T: From<ATerm>> IntoIterator for ATermList<T> {
    type IntoIter = ATermListIter<T>;
    type Item = T;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T: From<ATerm>> IntoIterator for &ATermList<T> {
    type IntoIter = ATermListIter<T>;
    type Item = T;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<T: From<ATerm> + fmt::Display> fmt::Display for ATermList<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}]", self.iter().format(","))
    }
}

/// The iterator over the elements of an [ATermList].
pub struct ATermListIter<T> {
    current: ATermList<T>,
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_list_term() {
        use super::*;
        use crate::ATermInt;

        let list = ATermList::from_double_iter(vec![ATermInt::new(1), ATermInt::new(2), ATermInt::new(3)].into_iter());
        assert_eq!(list.head().value(), 1);
        assert_eq!(list.tail().head().value(), 2);
        assert_eq!(list.tail().tail().head().value(), 3);
        assert!(list.tail().tail().tail().is_empty());
    }
}
