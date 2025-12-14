use std::marker::PhantomData;

use crate::ATerm;
use crate::ATermRef;

pub struct ATermList<T> {
    term: ATerm,
    _marker: PhantomData<T>,
}

impl<T: From<ATerm>> ATermList<T> {
    /// Obtain the head, i.e. the first element, of the list.
    pub fn head(&self) -> T {
        self.term.arg(0).protect().into()
    }
}

impl<T> ATermList<T> {
    /// Creates a new ATermList from the given term.
    pub fn new(term: ATerm) -> Self {
        debug_assert!(term.term.is_list(), "Term is not a list: {:?}", term);
        ATermList {
            term,
            _marker: PhantomData,
        }
    }

    /// Returns true iff the list is empty.
    pub fn is_empty(&self) -> bool {
        self.term.is_empty_list()
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

impl<T: From<ATerm>> ATermList<T> {
    /// Converts the list to a `Vec<T>`.
    pub fn to_vec(&self) -> Vec<T> {
        self.iter().collect()
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
        Self::new(value)
    }
}

impl<'a, T> From<ATermRef<'a>> for ATermList<T> {
    fn from(value: ATermRef<'a>) -> Self {
        Self::new(value.protect())
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

pub struct ATermListIter<T> {
    current: ATermList<T>,
}

#[cfg(test)]
mod tests {
    use merc_utilities::test_logger;

    use crate::ATerm;
    use crate::ATermList;

    #[test]
    fn test_aterm_list() {
        let _ = test_logger();
        let list: ATermList<ATerm> = ATerm::from_string("[f,g,h,i]").unwrap().into();

        assert!(!list.is_empty());

        // Convert into normal vector.
        let values: Vec<ATerm> = list.iter().collect();

        assert_eq!(values[0], ATerm::from_string("f").unwrap());
        assert_eq!(values[1], ATerm::from_string("g").unwrap());
        assert_eq!(values[2], ATerm::from_string("h").unwrap());
        assert_eq!(values[3], ATerm::from_string("i").unwrap());
    }
}
