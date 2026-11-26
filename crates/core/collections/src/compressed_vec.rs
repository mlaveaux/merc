use std::fmt;
use std::marker::PhantomData;

use bitvec::bitvec;
use bitvec::order::Lsb0;
use delegate::delegate;
use log::trace;

use merc_io::BytesFormatter;
use merc_utilities::TagIndex;
use merc_utilities::debug_trace;
use merc_utilities::is_valid_permutation;

/// A copy of `vec![]` that can be used for the [`crate::ByteCompressedVec`].
#[macro_export]
macro_rules! bytevec {
    () => {
        $crate::ByteCompressedVec::new()
    };
    ($elem:expr; $n:expr) => {
        $crate::ByteCompressedVec::from_elem($elem, $n)
    };
}

/// A vector that stores `CompressedEntry` elements in a fixed, per-vector byte width sized to the
/// largest entry added so far; the width only ever grows, never shrinks, as larger entries arrive.
///
/// `T::drop` is never called for stored entries.
#[derive(Default, PartialEq, Eq, Clone)]
pub struct ByteCompressedVec<T> {
    data: Vec<u8>,
    bytes_per_entry: usize,
    _marker: PhantomData<T>,
}

impl<T: CompressedEntry> ByteCompressedVec<T> {
    pub fn new() -> ByteCompressedVec<T> {
        ByteCompressedVec {
            data: Vec::new(),
            bytes_per_entry: 0,
            _marker: PhantomData,
        }
    }

    /// Initializes a ByteCompressedVec with the given capacity and (minimal) bytes per entry.
    pub fn with_capacity(capacity: usize, bytes_per_entry: usize) -> ByteCompressedVec<T> {
        ByteCompressedVec {
            data: Vec::with_capacity(capacity * bytes_per_entry),
            bytes_per_entry,
            _marker: PhantomData,
        }
    }

    /// Equivalent to `collect()`, but scans the iterator up front to size the entries once
    /// instead of growing the byte width as larger entries are pushed.
    pub fn with_iter<I>(iter: I) -> ByteCompressedVec<T>
    where
        I: ExactSizeIterator<Item = T> + Clone,
    {
        let bytes_per_entry = iter
            .clone()
            .fold(0, |max_bytes, entry| max_bytes.max(entry.bytes_required()));

        let mut vec = ByteCompressedVec::with_capacity(iter.len(), bytes_per_entry);
        for entry in iter {
            vec.push(entry);
        }
        vec
    }

    /// Adds a new entry to the vector.
    pub fn push(&mut self, entry: T) {
        self.resize_entries(entry.bytes_required());

        // Add the new entry to the end of the vector.
        let old_len = self.data.len();
        self.data.resize(old_len + self.bytes_per_entry, 0);
        entry.to_bytes(&mut self.data[old_len..]);
    }

    /// Removes the last element from the vector and returns it, or None if it is empty.
    pub fn pop(&mut self) -> Option<T> {
        if self.is_empty() {
            None
        } else {
            let index = self.len() - 1;
            let entry = self.index(index);
            self.data.truncate(index * self.bytes_per_entry);
            Some(entry)
        }
    }

    /// Returns the entry at the given index.
    ///
    /// # Panics
    ///
    /// Panics if `index >= len()`.
    pub fn index(&self, index: usize) -> T {
        let start = index * self.bytes_per_entry;
        let end = start + self.bytes_per_entry;
        T::from_bytes(&self.data[start..end])
    }

    /// Sets the entry at the given index.
    ///
    /// # Panics
    ///
    /// Panics if `index >= len()`; unlike `push`, this does not grow the vector.
    pub fn set(&mut self, index: usize, entry: T) {
        self.resize_entries(entry.bytes_required());

        let start = index * self.bytes_per_entry;
        let end = start + self.bytes_per_entry;
        entry.to_bytes(&mut self.data[start..end]);
    }

    /// Returns the number of elements in the vector.
    pub fn len(&self) -> usize {
        if self.bytes_per_entry == 0 {
            0
        } else {
            debug_assert!(self.data.len().is_multiple_of(self.bytes_per_entry));
            self.data.len() / self.bytes_per_entry
        }
    }

    /// Returns true if the vector is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns metrics about memory usage of this compressed vector
    pub fn metrics(&self) -> CompressedVecMetrics {
        let element_count = self.len();
        let actual_memory =
            self.data.len() + std::mem::size_of_val(&self.bytes_per_entry) + std::mem::size_of::<PhantomData<T>>();
        let worst_case_memory = element_count * std::mem::size_of::<T>();

        CompressedVecMetrics {
            actual_memory,
            worst_case_memory,
        }
    }

    /// Returns an iterator over the elements in the vector.
    pub fn iter(&self) -> ByteCompressedVecIterator<'_, T> {
        ByteCompressedVecIterator {
            vector: self,
            current: 0,
            end: self.len(),
        }
    }

    /// Returns an iterator over the elements in the vector for the begin, end range.
    pub fn iter_range(&self, begin: usize, end: usize) -> ByteCompressedVecIterator<'_, T> {
        ByteCompressedVecIterator {
            vector: self,
            current: begin,
            end,
        }
    }

    /// Updates the given entry using a closure.
    ///
    /// # Panics
    ///
    /// Panics if `index >= len()`.
    pub fn update<F>(&mut self, index: usize, mut update: F)
    where
        F: FnMut(&mut T),
    {
        let mut entry = self.index(index);
        update(&mut entry);
        self.set(index, entry);
    }

    /// Iterate over all elements and adapt the elements using a closure.
    pub fn map<F>(&mut self, mut f: F)
    where
        F: FnMut(&mut T),
    {
        for index in 0..self.len() {
            let mut entry = self.index(index);
            f(&mut entry);
            self.set(index, entry);
        }
    }

    /// Folds over the elements in the vector using the provided closure.
    pub fn fold<B, F>(&mut self, init: B, mut f: F) -> B
    where
        F: FnMut(B, &mut T) -> B,
    {
        let mut accumulator = init;
        for index in 0..self.len() {
            let mut element = self.index(index);
            accumulator = f(accumulator, &mut element);
            self.set(index, element);
        }
        accumulator
    }

    /// Permutes a vector in place according to the given permutation function.
    ///
    /// The resulting vector will be [v_p^-1(0), v_p^-1(1), ..., v_p^-1(n-1)] where p is the permutation function.
    pub fn permute<P>(&mut self, permutation: P)
    where
        P: Fn(usize) -> usize,
    {
        debug_assert!(
            is_valid_permutation(&permutation, self.len()),
            "The given permutation must be a bijective mapping"
        );

        let mut visited = bitvec![usize, Lsb0; 0; self.len()];
        for start in 0..self.len() {
            if visited[start] {
                continue;
            }

            // Perform the cycle starting at 'start'
            let mut current = start;

            // Keeps track of the last displaced element
            let mut old = self.index(start);

            debug_trace!("Starting new cycle at position {}", start);
            while !visited[current] {
                visited.set(current, true);
                let next = permutation(current);
                if next != current {
                    debug_trace!("Moving element from position {} to position {}", current, next);
                    let temp = self.index(next);
                    self.set(next, old);
                    old = temp;
                }

                current = next;
            }
        }
    }

    /// Applies a permutation to a vector in place using an index function.
    ///
    /// The resulting vector will be [v_p(0), v_p(1), ..., v_p(n-1)] where p is the index function.
    pub fn permute_indices<P>(&mut self, indices: P)
    where
        P: Fn(usize) -> usize,
    {
        debug_assert!(
            is_valid_permutation(&indices, self.len()),
            "The given permutation must be a bijective mapping"
        );

        let mut visited = bitvec![usize, Lsb0; 0; self.len()];
        for start in 0..self.len() {
            if visited[start] {
                continue;
            }

            // Follow the cycle starting at 'start'
            debug_trace!("Starting new cycle at position {}", start);
            let mut current = start;
            let original = self.index(start);

            while !visited[current] {
                visited.set(current, true);
                let next = indices(current);

                if next != current {
                    if next != start {
                        debug_trace!("Moving element from position {} to position {}", current, next);
                        self.set(current, self.index(next));
                    } else {
                        break;
                    }
                }

                current = next;
            }

            trace!("Writing original to {}", current);
            self.set(current, original);
        }
    }

    /// Applies a permutation to a vector in place using an index function.
    ///
    /// This variant is faster but requires additional memory for the intermediate result vector.
    pub fn permute_indices_fast<P>(&mut self, indices: P)
    where
        P: Fn(usize) -> usize,
    {
        // `with_capacity` is in entries, not bytes, so pass the element count.
        let mut result = ByteCompressedVec::with_capacity(self.len(), self.bytes_per_entry);
        for index in 0..self.len() {
            result.push(self.index(indices(index)));
        }
        *self = result;
    }

    /// Swaps the entries at the given indices.
    ///
    /// # Panics
    ///
    /// Panics if either index is `>= len()`.
    pub fn swap(&mut self, index1: usize, index2: usize) {
        if index1 != index2 {
            let start1 = index1 * self.bytes_per_entry;
            let start2 = index2 * self.bytes_per_entry;

            // Create a temporary buffer for one entry
            let temp = T::from_bytes(&self.data[start1..start1 + self.bytes_per_entry]);

            // Copy entry2 to entry1's position
            self.data.copy_within(start2..start2 + self.bytes_per_entry, start1);

            // Copy temp to entry2's position
            temp.to_bytes(&mut self.data[start2..start2 + self.bytes_per_entry]);
        }
    }

    /// Resizes the vector to the given length, filling new entries with the provided value.
    pub fn resize_with<F>(&mut self, new_len: usize, mut f: F)
    where
        F: FnMut() -> T,
    {
        let current_len = self.len();
        if new_len > current_len {
            // Preallocate space for the additional entries.
            self.data.reserve((new_len - current_len) * self.bytes_per_entry);
            for _ in current_len..new_len {
                self.push(f());
            }
        } else if new_len < current_len {
            if new_len == 0 {
                self.data.clear();
                self.bytes_per_entry = 0;
            } else {
                // It could be that the bytes per entry is now less, but that we never reduce.
                self.data.truncate(new_len * self.bytes_per_entry);
            }
        }
    }

    /// Reserves capacity for at least `additional` more entries, sized for entries up to
    /// `bytes_per_entry` bytes wide.
    pub fn reserve(&mut self, additional: usize, bytes_per_entry: usize) {
        self.resize_entries(bytes_per_entry);
        self.data.reserve(additional * self.bytes_per_entry);
    }

    /// Grows the vector to `new_len` entries, filling the new ones with an
    /// all-zero byte pattern. Only meaningful for entry types where an all-zero
    /// byte pattern decodes to a valid "zero" value.
    ///
    /// Meant for pre-allocating a vector whose entries are filled in out of
    /// order afterward via [`Self::set`].
    pub fn resize_zeroed(&mut self, new_len: usize, bytes_per_entry: usize) {
        self.resize_entries(bytes_per_entry);
        let target_len = new_len * self.bytes_per_entry;
        if target_len > self.data.len() {
            self.data.resize(target_len, 0);
        }
    }

    /// Releases any excess allocated capacity back to the allocator.
    ///
    /// This is a hint: the allocator is not required to actually shrink the allocation.
    /// Useful after `resize_with` truncates the vector, since truncation only reduces the
    /// logical length and never releases the underlying allocation on its own.
    pub fn shrink_to_fit(&mut self) {
        self.data.shrink_to_fit();
    }

    /// Resizes all entries in the vector to the given length.
    fn resize_entries(&mut self, new_bytes_required: usize) {
        if new_bytes_required > self.bytes_per_entry {
            let mut new_data: Vec<u8> = vec![0; self.len() * new_bytes_required];

            if self.bytes_per_entry > 0 {
                // Resize all the existing elements because the new entry requires more bytes.
                for (index, entry) in self.iter().enumerate() {
                    let start = index * new_bytes_required;
                    let end = start + new_bytes_required;
                    entry.to_bytes(&mut new_data[start..end]);
                }
            }

            self.bytes_per_entry = new_bytes_required;
            self.data = new_data;
        }
    }
}

impl<T: CompressedEntry + Clone> ByteCompressedVec<T> {
    pub fn from_elem(entry: T, n: usize) -> ByteCompressedVec<T> {
        let mut vec = ByteCompressedVec::with_capacity(n, entry.bytes_required());
        for _ in 0..n {
            vec.push(entry.clone());
        }
        vec
    }
}

/// Metrics for tracking memory usage of a ByteCompressedVec
#[derive(Debug, Clone)]
pub struct CompressedVecMetrics {
    /// Actual memory used by the compressed vector (in bytes)
    pub actual_memory: usize,
    /// Worst-case memory that would be used by an uncompressed vector (len * sizeof(T))
    pub worst_case_memory: usize,
}

impl CompressedVecMetrics {
    /// Calculate memory savings in bytes
    pub fn memory_savings(&self) -> usize {
        self.worst_case_memory.saturating_sub(self.actual_memory)
    }

    /// Calculate memory savings as a percentage
    pub fn used_percentage(&self) -> f64 {
        if self.worst_case_memory == 0 {
            0.0
        } else {
            (self.actual_memory as f64 / self.worst_case_memory as f64) * 100.0
        }
    }
}

impl fmt::Display for CompressedVecMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "memory: {} ({:.1}%), saving: {} ",
            BytesFormatter(self.actual_memory),
            self.used_percentage(),
            BytesFormatter(self.memory_savings()),
        )
    }
}
pub struct ByteCompressedVecIterator<'a, T> {
    vector: &'a ByteCompressedVec<T>,
    current: usize,
    end: usize,
}

impl<T: CompressedEntry> Iterator for ByteCompressedVecIterator<'_, T> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current < self.end {
            let result = self.vector.index(self.current);
            self.current += 1;
            Some(result)
        } else {
            None
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.end - self.current;
        (remaining, Some(remaining))
    }
}

impl<T: CompressedEntry> ExactSizeIterator for ByteCompressedVecIterator<'_, T> {}

pub trait CompressedEntry {
    // Returns the entry as a byte vector
    fn to_bytes(&self, bytes: &mut [u8]);

    // Creates an entry from a byte vector
    fn from_bytes(bytes: &[u8]) -> Self;

    // Returns the number of bytes required to store the current entry
    fn bytes_required(&self) -> usize;
}

impl CompressedEntry for usize {
    fn to_bytes(&self, bytes: &mut [u8]) {
        let array = &self.to_le_bytes();
        for (i, byte) in bytes.iter_mut().enumerate().take(usize::BITS as usize / 8) {
            *byte = array[i];
        }
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        let mut array = [0; 8];
        for (i, byte) in bytes.iter().enumerate().take(usize::BITS as usize / 8) {
            array[i] = *byte;
        }
        usize::from_le_bytes(array)
    }

    fn bytes_required(&self) -> usize {
        // Compute the bit width of the value directly. Using `self + 1` would
        // overflow at usize::MAX and overestimate by one byte at exact byte
        // boundaries (e.g. 255 would report 2 bytes instead of 1).
        ((*self).max(1).ilog2() / u8::BITS) as usize + 1
    }
}

impl<T: CompressedEntry + fmt::Debug> fmt::Debug for ByteCompressedVec<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

/// Implement it for the TagIndex for convenience.
impl<T: CompressedEntry + Copy, Tag> CompressedEntry for TagIndex<T, Tag> {
    delegate! {
        to self.value() {
            fn to_bytes(&self, bytes: &mut [u8]);
            fn bytes_required(&self) -> usize;
        }
    }

    fn from_bytes(bytes: &[u8]) -> Self {
        TagIndex::new(T::from_bytes(bytes))
    }
}
