use std::fmt;
use std::marker::PhantomData;

use crate::BytesFormatter;

/// A vector data structure that stores objects in a byte compressed format
#[derive(Debug, Default, PartialEq, Eq, Clone)]
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

    /// Adds a new entry to the vector.
    pub fn push(&mut self, entry: T) {
        self.resize_entries(entry.bytes_required());

        // Add the new entry to the end of the vector.
        let old_len = self.data.len();
        self.data.resize(old_len + self.bytes_per_entry, 0);
        entry.to_bytes(&mut self.data[old_len..]);
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
            end: self.len()
        }
    }

    /// Returns an iterator over the elements in the vector for the begin, end range.
    pub fn iter_range(&self, begin: usize, end: usize) -> ByteCompressedVecIterator<'_, T> {
        ByteCompressedVecIterator { vector: self, current: begin, end }
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
        where F: FnMut(&mut T) 
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

    /// Sorts the given range (begin, end) of the vector using an unstable sort.
    pub fn sort_unstable_range(&mut self, begin: usize, end: usize)
    where
        T: Ord,
    {
        let mut elements: Vec<T> = self.iter_range(begin, end).collect();
        elements.sort_unstable();
        for (i, element) in elements.into_iter().enumerate() {
            self.set(begin + i, element);
        }
    }

    /// Swaps the entries at the given indices.
    pub fn swap(&mut self, index1: usize, index2: usize) {
        let entry1 = self.index(index1);
        let entry2 = self.index(index2);
        self.set(index1, entry2);
        self.set(index2, entry1);
    }

    /// Resizes the vector to the given length, filling new entries with the provided value.
    pub fn resize_with<F>(&mut self, new_len: usize, mut f: F)
    where
        F: FnMut() -> T,
    {
        let current_len = self.len();
        if new_len > current_len {
            for _ in current_len..new_len {
                self.push(f());
            }
        } else if new_len < current_len {
            if new_len == 0 {
                self.data.clear();
                self.bytes_per_entry = 0;
            } else {
                // It could be that the bytes per entry is now less.
                self.data.truncate(new_len * self.bytes_per_entry);
            }
        }
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
        let mut vec = ByteCompressedVec::new();
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

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{bytevec, random_test};

    use rand::Rng;
    use rand::distr::Uniform;

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
    fn test_usize_entry() {
        random_test(100, |rng| {
            let value = rng.random_range(0..1024);
            assert!(value.bytes_required() <= 2);

            let mut bytes = [0; 2];
            value.to_bytes(&mut bytes);
            assert_eq!(usize::from_bytes(&bytes), value);
        });
    }
}
