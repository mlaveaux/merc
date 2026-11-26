use std::fmt;
use std::mem::align_of;
use std::ptr::NonNull;
use std::sync::atomic::AtomicPtr;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

/// An atomic pointer to a `T` that stores a flag in the lowest bit of its address.
///
/// The alignment of `T` keeps that bit zero in every valid pointer, which is checked at compile
/// time, so the flag never changes the pointer itself. Loads mask the flag off, and the address
/// is only ever transformed with its provenance preserved.
///
/// All operations are relaxed: the flag carries no data of its own, so whoever relies on it must
/// order its accesses by other means, such as a lock.
pub struct AtomicTaggedPtr<T> {
    ptr: AtomicPtr<T>,
}

impl<T> AtomicTaggedPtr<T> {
    /// The bit of the address that stores the flag.
    const TAG: usize = {
        assert!(
            align_of::<T>() >= 2,
            "The alignment of T must leave the lowest address bit free"
        );
        1
    };

    /// Creates an untagged pointer.
    pub fn new(ptr: NonNull<T>) -> Self {
        debug_assert!(
            ptr.is_aligned(),
            "The pointer must be aligned to leave the tag bit free"
        );
        Self {
            ptr: AtomicPtr::new(ptr.as_ptr().map_addr(|addr| addr & !Self::TAG)),
        }
    }

    /// Returns the pointer without the tag, which is never null.
    pub fn load(&self) -> *mut T {
        self.ptr.load(Ordering::Relaxed).map_addr(|addr| addr & !Self::TAG)
    }

    /// Returns true iff the tag is set.
    pub fn is_tagged(&self) -> bool {
        self.ptr.load(Ordering::Relaxed).addr() & Self::TAG != 0
    }

    /// Sets the tag and returns whether it was already set.
    pub fn set_tag(&self) -> bool {
        self.ptr.fetch_or(Self::TAG, Ordering::Relaxed).addr() & Self::TAG != 0
    }

    /// Clears the tag.
    pub fn clear_tag(&self) {
        self.ptr.fetch_and(!Self::TAG, Ordering::Relaxed);
    }
}

impl<T> fmt::Debug for AtomicTaggedPtr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AtomicTaggedPtr")
            .field("ptr", &self.load())
            .field("tagged", &self.is_tagged())
            .finish()
    }
}

/// An atomic `usize` that stores a flag in its most significant bit, see [AtomicTaggedPtr].
///
/// Values must leave that bit free, which is checked on construction. All operations are relaxed
/// for the same reason as for [AtomicTaggedPtr].
pub struct AtomicTaggedUsize {
    value: AtomicUsize,
}

impl AtomicTaggedUsize {
    /// The bit of the value that stores the flag.
    const TAG: usize = 1 << (usize::BITS - 1);

    /// Creates an untagged value.
    ///
    /// # Panics
    ///
    /// Panics if the most significant bit of `value` is set.
    pub fn new(value: usize) -> Self {
        assert!(value & Self::TAG == 0, "The value {value} must leave the tag bit free");
        Self {
            value: AtomicUsize::new(value),
        }
    }

    /// Returns the value without the tag.
    pub fn load(&self) -> usize {
        self.value.load(Ordering::Relaxed) & !Self::TAG
    }

    /// Returns true iff the tag is set.
    pub fn is_tagged(&self) -> bool {
        self.value.load(Ordering::Relaxed) & Self::TAG != 0
    }

    /// Sets the tag and returns whether it was already set.
    pub fn set_tag(&self) -> bool {
        self.value.fetch_or(Self::TAG, Ordering::Relaxed) & Self::TAG != 0
    }

    /// Clears the tag.
    pub fn clear_tag(&self) {
        self.value.fetch_and(!Self::TAG, Ordering::Relaxed);
    }
}

impl fmt::Debug for AtomicTaggedUsize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AtomicTaggedUsize")
            .field("value", &self.load())
            .field("tagged", &self.is_tagged())
            .finish()
    }
}
