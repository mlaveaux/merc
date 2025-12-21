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

/// A vector data structure that stores objects in a byte compressed format. The basic idea is that elements of type `T` impplement the `CompressedEntry` trait which allows them to be converted to and from a byte representation. The vector dynamically adjusts the number of bytes used per entry based on the maximum size of the entries added so far.
///
/// For numbers this means that we only store the number of bytes required to represent the largest number added so far. Note that the number of bytes used per entry is only increased over time as larger entries are added.
///
/// TODO: The `drop()` function of `T` is never called.
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

    /// This is basically the collect() of `Vec`.
    ///
    /// However, we use it to determine the required bytes per entry in advance.
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
    pub fn index(&self, index: usize) -> T {
        let start = index * self.bytes_per_entry;
        let end = start + self.bytes_per_entry;
        T::from_bytes(&self.data[start..end])
    }

    /// Sets the entry at the given index.
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
            debug_assert!(self.data.len() % self.bytes_per_entry == 0);
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
        let mut result = ByteCompressedVec::with_capacity(self.data.capacity(), self.bytes_per_entry);
        for entry in self.iter().enumerate() {
            result.push(self.index(indices(entry.0)));
        }
        *self = result;
    }

    /// Swaps the entries at the given indices.
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
            // Preallocate the required space.
            self.data.reserve(new_len * self.bytes_per_entry);
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

    /// Reserves capacity for at least additional more entries to be inserted with the given bytes per entry.
    pub fn reserve(&mut self, additional: usize, bytes_per_entry: usize) {
        self.resize_entries(bytes_per_entry);
        self.data.reserve(additional * self.bytes_per_entry);
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
}

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
        ((self + 1).ilog2() / u8::BITS) as usize + 1
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

#[cfg(test)]
mod tests {
    use super::*;

    use rand::Rng;
    use rand::distr::Uniform;
    use rand::seq::SliceRandom;

    use merc_utilities::random_test;

    use crate::bytevec;

    #[test]
    fn test_index_bytevector() {
        let mut vec = ByteCompressedVec::new();
        vec.push(1);
        assert_eq!(vec.len(), 1);

        vec.push(1024);
        assert_eq!(vec.len(), 2);

        assert_eq!(vec.index(0), 1);
        assert_eq!(vec.index(1), 1024);
    }

    #[test]
    fn test_random_bytevector() {
        let rng = rand::rng();

        let range = Uniform::new(0, usize::MAX).unwrap();
        let expected_vector: Vec<usize> = rng.sample_iter(range).take(100).collect();
        let mut vector = ByteCompressedVec::new();

        for element in &expected_vector {
            vector.push(*element);

            for (expected, element) in expected_vector.iter().zip(vector.iter()) {
                assert_eq!(*expected, element);
            }
        }
    }

    #[test]
    fn test_random_setting_bytevector() {
        let rng = rand::rng();

        let range = Uniform::new(0, usize::MAX).unwrap();
        let expected_vector: Vec<usize> = rng.sample_iter(range).take(100).collect();
        let mut vector = bytevec![0; 100];

        for (index, element) in expected_vector.iter().enumerate() {
            vector.set(index, *element);
        }

        for (expected, element) in expected_vector.iter().zip(vector.iter()) {
            assert_eq!(*expected, element);
        }
    }

    #[test]
    fn test_random_usize_entry() {
        random_test(100, |rng| {
            let value = rng.random_range(0..1024);
            assert!(value.bytes_required() <= 2);

            let mut bytes = [0; 2];
            value.to_bytes(&mut bytes);
            assert_eq!(usize::from_bytes(&bytes), value);
        });
    }

    #[test]
    fn test_swap() {
        let mut vec = ByteCompressedVec::new();
        vec.push(1);
        vec.push(256);
        vec.push(65536);

        vec.swap(0, 2);

        assert_eq!(vec.index(0), 65536);
        assert_eq!(vec.index(1), 256);
        assert_eq!(vec.index(2), 1);
    }

    #[test]
    fn test_random_bytevector_permute() {
        random_test(100, |rng| {
            // Generate random vector to permute
            let elements = (0..rng.random_range(1..10))
                .map(|_| rng.random_range(0..100))
                .collect::<Vec<_>>();

            let vec = ByteCompressedVec::with_iter(elements.iter().cloned());

            for is_inverse in [false, true] {
                println!("Inverse: {is_inverse}, Input: {:?}", vec);

                let permutation = {
                    let mut order: Vec<usize> = (0..elements.len()).collect();
                    order.shuffle(rng);
                    order
                };

                let mut permutated = vec.clone();
                if is_inverse {
                    permutated.permute_indices(|i| permutation[i]);
                } else {
                    permutated.permute(|i| permutation[i]);
                }

                println!("Permutation: {:?}", permutation);
                println!("After permutation: {:?}", permutated);

                // Check that the permutation was applied correctly
                for i in 0..elements.len() {
                    let pos = if is_inverse {
                        permutation[i]
                    } else {
                        permutation
                            .iter()
                            .position(|&j| i == j)
                            .expect("Should find inverse mapping")
                    };

                    debug_assert_eq!(
                        permutated.index(i),
                        elements[pos],
                        "Element at index {} should be {}",
                        i,
                        elements[pos]
                    );
                }
            }
        });
    }
}
