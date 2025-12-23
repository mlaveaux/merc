use std::{fmt, slice::Iter};

use itertools::Itertools;

#[macro_export]
macro_rules! vecset {
    () => {
        $crate::vecset::VecSet::new()
    };
    ($elem:expr; $n:expr) => {{
        let mut __set = $crate::vecset::VecSet::new();
        let __count: usize = $n;
        if __count > 0 {
            __set.insert($elem);
        }
        __set
    }};
    ($($x:expr),+ $(,)?) => {{
        let mut __set = $crate::vecset::VecSet::new();
        $( let _ = __set.insert($x); )*
        __set
    }};
}

///
/// A set that is internally represented by a sorted vector. Mostly useful for
/// a compact representation of sets that are not changed often.
///
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VecSet<T> {
    /// The internal storage with the invariant that the array is sorted.
    sorted_array: Vec<T>,
}

impl<T: Ord> VecSet<T> {
    pub fn new() -> Self {
        Self {
            sorted_array: Vec::new(),
        }
    }

    /// Returns a new set only containing the given element.
    pub fn singleton(element: T) -> Self {
        Self {
            sorted_array: vec![element],
        }
    }

    /// Returns true iff the set is empty.
    pub fn is_empty(&self) -> bool {
        self.sorted_array.is_empty()
    }

    /// Inserts the given element into the set, returns true iff the element was
    /// inserted.
    pub fn insert(&mut self, element: T) -> bool {
        // Finds the location where to insert the element to keep the array sorted.
        if let Err(position) = self.sorted_array.binary_search(&element) {
            self.sorted_array.insert(position, element);
            return true;
        }

        false
    }

    /// Returns an iterator over the elements in the set, they are yielded in sorted order.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.sorted_array.iter()
    }

    /// Returns the number of elements in the set.
    pub fn len(&self) -> usize {
        self.sorted_array.len()
    }
}

impl<'a, T> IntoIterator for &'a VecSet<T> {
    type Item = &'a T;
    type IntoIter = Iter<'a, T>;

    fn into_iter(self) -> Self::IntoIter {
        self.sorted_array.iter()
    }
}

impl<T: fmt::Debug> fmt::Debug for VecSet<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{{{:?}}}", self.sorted_array.iter().format(", "))
    }
}
