use std::marker::PhantomData;

use mcrl2_sys::atermpp::ffi::{mcrl2_aterm_list_front, mcrl2_aterm_list_is_empty, mcrl2_aterm_list_tail};

use crate::Aterm;



/// Represents a list of terms from the mCRL2 toolset.
#[derive(Clone)]
pub struct AtermList<T> {
    term: Aterm,
    _marker: PhantomData<T>,
}

impl<T: Clone + From<Aterm>> AtermList<T> {
    /// Returns the head of the list
    pub fn head(&self) -> T
    where
        T: From<Aterm>,
    {
        Aterm::new(mcrl2_aterm_list_front(&self.term.get())).into()
    }

    /// Returns the length of the list.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// Converts the list to a `Vec<T>`.
    pub fn to_vec(&self) -> Vec<T> {
        self.iter().collect()
    }

    /// Returns an iterator over the elements of the list.
    pub fn iter(&self) -> ATermListIter<T> {
        ATermListIter::new(self.clone())
    }
}


impl<T> AtermList<T> {
    /// Returns true if the list is empty.
    pub fn is_empty(&self) -> bool {
        mcrl2_aterm_list_is_empty(&self.term.get())
    }

    /// Returns the tail of the list
    pub fn tail(&self) -> AtermList<T> {
        AtermList::new(Aterm::new(mcrl2_aterm_list_tail(&self.term.get()).into()))
    }

    /// Creates a new list from the given term.
    pub(crate) fn new(term: Aterm) -> Self {
        AtermList {
            term,
            _marker: PhantomData,
        }
    }
}

impl From<Aterm> for AtermList<Aterm> {
    fn from(term: Aterm) -> Self {
        AtermList::new(term)
    }
}

pub struct ATermListIter<T> {
    list: AtermList<T>,
}

impl<T> ATermListIter<T> {
    pub fn new(list: AtermList<T>) -> Self {
        ATermListIter { list }
    }
}

impl<T: Clone + From<Aterm>> Iterator for ATermListIter<T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.list.is_empty() {
            None
        } else {
            let head = self.list.head();
            self.list = self.list.tail();
            Some(head)
        }
    }
}
