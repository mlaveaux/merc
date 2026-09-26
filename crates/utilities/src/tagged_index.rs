use std::fmt;
use std::hash::Hash;
use std::marker::PhantomData;
use std::ops::Deref;
use std::ops::Index;
use std::ops::IndexMut;
use std::slice::SliceIndex;

/// A trait for index types.
pub trait MercIndex: Copy + PartialEq {
    /// The underlying target type.
    type Target;

    /// Returns the underlying index.
    fn index(&self) -> Self::Target;
}

impl MercIndex for () {
    type Target = ();

    fn index(&self) -> Self::Target {}
}

/// A newtype index wrapping an inner value of type `T`, distinguished at compile time by `Tag`.
/// Two `TagIndex` values compare, order, and hash together only when they share both `T` and
/// `Tag`, so indices from different domains (e.g. state, action, priority) cannot be mixed up or
/// compared with each other. `new` accepts any `T` unconditionally, so a `TagIndex` is not by
/// itself a proof that the wrapped value is a valid index into its domain.
///
/// Implements the traits typically needed for indices (`PartialEq`, `Eq`, `PartialOrd`, `Ord`,
/// `Hash`) but not arithmetic, since indices are not naturally added or subtracted. Implements
/// `Index`/`IndexMut` on `Vec<U>`/`[U]` for direct indexing; `value()` returns the underlying `T`.
pub struct TagIndex<T, Tag> {
    index: T,

    /// Ensures that the Tag is used by the struct
    marker: PhantomData<fn() -> Tag>,
}

impl<T: Copy + PartialEq, Tag> MercIndex for TagIndex<T, Tag> {
    type Target = T;

    fn index(&self) -> Self::Target {
        self.index
    }
}

impl<T: Default, Tag> Default for TagIndex<T, Tag> {
    fn default() -> Self {
        Self {
            index: T::default(),
            marker: PhantomData,
        }
    }
}

impl<T: Eq, Tag> Eq for TagIndex<T, Tag> {}

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
    /// Returns the underlying index value.
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

// Convenient traits for using the `TagIndex`.
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

/// Hands out consecutive `TagIndex<usize, Tag>` values, starting at 0.
pub struct IdAllocator<Tag> {
    next: usize,

    marker: PhantomData<fn() -> Tag>,
}

impl<Tag> Default for IdAllocator<Tag> {
    fn default() -> Self {
        Self {
            next: 0,
            marker: PhantomData,
        }
    }
}

impl<Tag> IdAllocator<Tag> {
    /// Returns the next id in the sequence.
    pub fn alloc(&mut self) -> TagIndex<usize, Tag> {
        let id = TagIndex::new(self.next);
        self.next += 1;
        id
    }
}

#[cfg(test)]
mod tests {
    use super::TagIndex;

    struct StateTag;
    struct ActionTag;

    type StateIndex = TagIndex<usize, StateTag>;
    type ActionIndex = TagIndex<usize, ActionTag>;

    /// `TagIndex`'s own doc comment claims "indices from different domains
    /// (e.g. state, action, priority) cannot be mixed up or compared with
    /// each other". This module has no other tests at all (this is the only
    /// `#[cfg(test)]` in `tagged_index.rs`), so nothing currently exercises
    /// that claim — and it does not hold: `impl PartialEq<T> for
    /// TagIndex<T, Tag>` (and the matching `PartialOrd<T>`) compare against
    /// the *raw*, tag-erased `T`, so a `StateIndex` and an `ActionIndex`
    /// that happen to wrap the same number silently compare equal once
    /// either side is unwrapped to its raw form, even though they come from
    /// unrelated domains. The type system only rejects *directly* comparing
    /// a `StateIndex` to an `ActionIndex` (different `Tag`s); it does not
    /// reject bridging through the untagged `T`, which is exactly what
    /// `PartialEq<T>` offers as a first-class, no-`unsafe`-required
    /// operation.
    #[test]
    fn tag_index_partial_eq_with_raw_value_bridges_across_unrelated_domains() {
        let state = StateIndex::new(3);
        let action = ActionIndex::new(3);

        // Directly comparing `state == action` does not compile (different
        // `Tag`s) -- that part of the safety story holds. But both indices
        // compare equal to the *same* raw value via `PartialEq<T>`, which
        // means a caller who accidentally compares a `StateIndex` against a
        // raw `usize` that actually came from an `ActionIndex` gets a
        // silent, semantically meaningless `true` instead of a compile
        // error or a panic.
        assert_eq!(state, 3usize);
        assert_eq!(action, 3usize);

        // Simulates the actual mix-up: something holding an `ActionIndex`
        // exposes it as a raw value (e.g. for serialization, hashing, or a
        // format string), and that raw value later gets compared against an
        // unrelated `StateIndex` -- nothing here signals that the domains
        // differ.
        let action_as_raw: usize = action.value();
        assert_eq!(
            state, action_as_raw,
            "a StateIndex should not be indistinguishable from an unrelated ActionIndex's raw value"
        );
    }
}
