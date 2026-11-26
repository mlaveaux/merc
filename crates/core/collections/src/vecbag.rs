use std::cmp::Ordering;
use std::fmt;
use std::marker::PhantomData;
use std::slice::Iter;

use itertools::Itertools;

use crate::vec_difference::Difference;

#[macro_export]
macro_rules! vecbag {
    () => {
        $crate::VecBag::new()
    };
    ($elem:expr; $n:expr) => {
        $crate::VecBag::from_vec(vec![$elem; $n])
    };
    ($($x:expr),+ $(,)?) => {{
        let mut __bag = $crate::VecBag::new();
        $( __bag.insert($x); )*
        __bag
    }};
}

/// A bag (multiset) internally represented by a sorted vector; compact for bags that are not changed often.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VecBag<T: Ord> {
    /// The internal storage with the invariant that the array is sorted.
    sorted_array: Vec<T>,
}

impl<T: Ord> VecBag<T> {
    /// Creates a new, empty VecBag.
    pub fn new() -> Self {
        Self {
            sorted_array: Vec::new(),
        }
    }

    /// Creates a VecBag from the given vector without assuming anything of the given vector.
    pub fn from_vec(mut vec: Vec<T>) -> Self {
        vec.sort_unstable();
        Self { sorted_array: vec }
    }

    /// Returns the capacity of the bag.
    pub fn capacity(&self) -> usize {
        self.sorted_array.capacity()
    }

    /// Returns true iff the bag contains at least one occurrence of the given element.
    pub fn contains(&self, element: &T) -> bool {
        self.sorted_array.binary_search(element).is_ok()
    }

    /// Clears the bag, removing all elements.
    pub fn clear(&mut self) {
        self.sorted_array.clear();
    }

    /// Retains only the elements specified by the predicate.
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&T) -> bool,
    {
        // Removing elements does not change the order.
        self.sorted_array.retain(|e| f(e));
    }

    /// Returns true iff this bag is a subbag of the other bag.
    pub fn is_subset(&self, other: &VecBag<T>) -> bool {
        let mut self_iter = self.sorted_array.iter();
        let mut other_iter = other.sorted_array.iter();

        // Traverse both bags in order, checking multiplicity as well.
        let mut self_next = self_iter.next();
        let mut other_next = other_iter.next();

        while let Some(self_val) = self_next {
            match other_next {
                Some(other_val) => match self_val.cmp(other_val) {
                    Ordering::Equal => {
                        self_next = self_iter.next();
                        other_next = other_iter.next();
                    }
                    Ordering::Greater => other_next = other_iter.next(),
                    Ordering::Less => return false,
                },
                None => return false,
            }
        }

        true
    }

    /// Returns a new bag only containing the given element.
    pub fn singleton(element: T) -> Self {
        Self {
            sorted_array: vec![element],
        }
    }

    /// Returns true iff the bag is empty.
    pub fn is_empty(&self) -> bool {
        self.sorted_array.is_empty()
    }

    /// Returns the difference of this bag and the other bag.
    pub fn difference<'a>(&'a self, other: &'a VecBag<T>) -> impl Iterator<Item = &'a T> {
        Difference::<'a, T, _> {
            self_iter: self.sorted_array.iter(),
            other_iter: other.sorted_array.iter(),
            other_next: None,
            marker: PhantomData,
        }
    }

    /// Inserts one occurrence of the given element into the bag.
    pub fn insert(&mut self, element: T) {
        // Finds the location where to insert the element to keep the array sorted.
        let position = match self.sorted_array.binary_search(&element) {
            Ok(position) | Err(position) => position,
        };

        self.sorted_array.insert(position, element);
    }

    /// Extends this bag with the elements from the given iterator.
    pub fn extend<'a, I: IntoIterator<Item = &'a T>>(&mut self, iter: I)
    where
        T: Clone + 'a,
    {
        for element in iter {
            self.insert(element.clone());
        }
    }

    /// Returns an iterator over the elements in the bag, they are yielded in sorted order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.sorted_array.iter()
    }

    /// Returns the number of elements in the bag.
    pub fn len(&self) -> usize {
        self.sorted_array.len()
    }

    /// Consumes the bag and returns a vector with the elements in sorted order.
    pub fn to_vec(self) -> Vec<T> {
        self.sorted_array
    }
}

impl<T: Ord> FromIterator<T> for VecBag<T> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::from_vec(iter.into_iter().collect())
    }
}

impl<T: Ord> Default for VecBag<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, T: Ord> IntoIterator for &'a VecBag<T> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.sorted_array.iter()
    }
}

impl<T: Ord + fmt::Debug> fmt::Debug for VecBag<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{:?}}}", self.sorted_array.iter().format(", "))
    }
}
