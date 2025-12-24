use std::fmt;
use std::hash::Hash;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ops::Index;
use std::ops::IndexMut;
use std::slice::SliceIndex;

/// An index is an index that can only be compared with equivalent tags. Note that the constructor does
/// not requires us to provide a tag, and as such anyone can make a tagged index. It is not a proof of a
/// valid index. This could be extended in the future.
///
/// Implement all the traits that are typically used for indices, e.g. PartialEq, Eq, PartialOrd, Ord and Hash.
///
/// Does not implement operations such as addition and subtraction since there are not natural. However, we do implement
/// Index for various containers for ease of usage. Otherwise, `value()` can be used to obtain the underlying `T`.
pub struct TagIndex<T, Tag> {
    index: T,

    /// Ensures that the Tag is used by the struct
    marker: PhantomData<fn() -> Tag>,
}

impl<T: Default, Tag> Default for TagIndex<T, Tag> {
    fn default() -> Self {
        Self {
            index: T::default(),
            marker: PhantomData,
        }
    }
}

impl<T: PartialEq, Tag> Eq for TagIndex<T, Tag> {}

impl<T: PartialEq, Tag> PartialEq for TagIndex<T, Tag> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}

impl<T: Ord, Tag> Ord for TagIndex<T, Tag> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.index.cmp(&other.index)
    }
}

impl<T: PartialOrd, Tag> PartialOrd for TagIndex<T, Tag> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.index.partial_cmp(&other.index)
    }
}

impl<T: Hash, Tag> Hash for TagIndex<T, Tag> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.index.hash(state);
    }
}

impl<T: Clone, Tag> Clone for TagIndex<T, Tag> {
    fn clone(&self) -> Self {
        Self {
            index: self.index.clone(),
            marker: self.marker,
        }
    }
}

impl<T: PartialEq, Tag> PartialEq<T> for TagIndex<T, Tag> {
    fn eq(&self, other: &T) -> bool {
        self.index.eq(other)
    }
}

impl<T: PartialOrd, Tag> PartialOrd<T> for TagIndex<T, Tag> {
    fn partial_cmp(&self, other: &T) -> Option<std::cmp::Ordering> {
        self.index.partial_cmp(other)
    }
}

impl<T: Copy, Tag> Copy for TagIndex<T, Tag> {}

impl<T, Tag> TagIndex<T, Tag> {
    pub fn new(index: T) -> Self {
        Self {
            index,
            marker: PhantomData,
        }
    }
}

impl<T: Copy, Tag> TagIndex<T, Tag> {
    /// Returns the underlying value of the safe index, mostly used for indexing.
    pub fn value(&self) -> T {
        self.index
    }
}

impl<T: fmt::Debug, Tag> fmt::Debug for TagIndex<T, Tag> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.index.fmt(f)
    }
}

impl<T: fmt::Display, Tag> fmt::Display for TagIndex<T, Tag> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.index.fmt(f)
    }
}

// Convenient traits for using the `SafeIndex`.
impl<T: Copy + SliceIndex<[U], Output = U>, U, Tag> Index<TagIndex<T, Tag>> for Vec<U> {
    type Output = U;

    fn index(&self, index: TagIndex<T, Tag>) -> &Self::Output {
        &self[index.value()]
    }
}

impl<T: Copy + SliceIndex<[U], Output = U>, U, Tag> Index<TagIndex<T, Tag>> for [U] {
    type Output = U;

    fn index(&self, index: TagIndex<T, Tag>) -> &Self::Output {
        &self[index.value()]
    }
}

impl<T: Copy + SliceIndex<[U], Output = U>, U, Tag> IndexMut<TagIndex<T, Tag>> for Vec<U> {
    fn index_mut(&mut self, index: TagIndex<T, Tag>) -> &mut Self::Output {
        &mut self[index.value()]
    }
}

impl<T: Copy + SliceIndex<[U], Output = U>, U, Tag> IndexMut<TagIndex<T, Tag>> for [U] {
    fn index_mut(&mut self, index: TagIndex<T, Tag>) -> &mut Self::Output {
        &mut self[index.value()]
    }
}

impl<T, Tag> Deref for TagIndex<T, Tag> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.index
    }
}
