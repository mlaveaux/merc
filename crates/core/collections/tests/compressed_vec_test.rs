use rand::Rng;
use rand::RngExt;
use rand::seq::SliceRandom;

use merc_collections::ByteCompressedVec;
use merc_collections::CompressedEntry;
use merc_collections::bytevec;
use merc_utilities::random_test;

/// A value that needs exactly `bytes` bytes, for `bytes` in `1..=8`.
fn value_of_width(rng: &mut impl Rng, bytes: u32) -> usize {
    let low = if bytes == 1 { 0 } else { 1usize << (8 * (bytes - 1)) };
    let high = if bytes == 8 {
        usize::MAX
    } else {
        (1usize << (8 * bytes)) - 1
    };
    rng.random_range(low..=high)
}

/// Asserts that the vector has the same contents as the model.
fn assert_matches(vec: &ByteCompressedVec<usize>, model: &[usize]) {
    assert_eq!(vec.len(), model.len());
    assert_eq!(vec.is_empty(), model.is_empty());
    assert_eq!(vec.iter().collect::<Vec<_>>(), model);
    for (index, &expected) in model.iter().enumerate() {
        assert_eq!(vec.index(index), expected, "index {index}");
    }
}

#[test]
fn test_index_bytevector() {
    let mut vec = ByteCompressedVec::new();
    vec.push(1);
    assert_eq!(vec.len(), 1);

    // A wider entry widens the existing entries without changing them.
    vec.push(1024);
    assert_eq!(vec.len(), 2);

    assert_eq!(vec.index(0), 1);
    assert_eq!(vec.index(1), 1024);
}

/// Applies random operations to the vector and to a `Vec` model, with entries of every width so
/// that the entry width grows during the test.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_operations_match_vec() {
    random_test(100, |rng| {
        let mut vec = ByteCompressedVec::new();
        let mut model: Vec<usize> = Vec::new();

        for _ in 0..100 {
            let width = rng.random_range(1..=8);
            let value = value_of_width(rng, width);
            match rng.random_range(0..6) {
                0 | 1 => {
                    vec.push(value);
                    model.push(value);
                }
                2 => assert_eq!(vec.pop(), model.pop()),
                3 if !model.is_empty() => {
                    let index = rng.random_range(0..model.len());
                    vec.set(index, value);
                    model[index] = value;
                }
                4 if !model.is_empty() => {
                    let (first, second) = (rng.random_range(0..model.len()), rng.random_range(0..model.len()));
                    vec.swap(first, second);
                    model.swap(first, second);
                }
                5 if !model.is_empty() => {
                    let index = rng.random_range(0..model.len());
                    vec.update(index, |entry| *entry = value);
                    model[index] = value;
                }
                _ => {}
            }
            assert_matches(&vec, &model);
        }

        let mut collected = ByteCompressedVec::with_iter(model.iter().copied());
        assert_matches(&collected, &model);

        let sum = collected.fold(0usize, |sum, entry| {
            let old = *entry;
            *entry = old / 2;
            sum.wrapping_add(old)
        });
        assert_eq!(sum, model.iter().fold(0usize, |sum, &entry| sum.wrapping_add(entry)));
        let halved: Vec<usize> = model.iter().map(|entry| entry / 2).collect();
        assert_matches(&collected, &halved);

        collected.map(|entry| *entry = entry.wrapping_mul(2));
        let doubled: Vec<usize> = halved.iter().map(|entry| entry.wrapping_mul(2)).collect();
        assert_matches(&collected, &doubled);
    });
}

#[test]
fn test_from_elem_and_set() {
    let mut vector = bytevec![0; 10];
    assert_matches(&vector, &[0; 10]);

    vector.set(3, usize::MAX);
    vector.set(7, 300);
    assert_matches(&vector, &[0, 0, 0, usize::MAX, 0, 0, 0, 300, 0, 0]);
    assert_eq!(
        vector.iter_range(2, 8).collect::<Vec<_>>(),
        [0, usize::MAX, 0, 0, 0, 300]
    );
}

#[test]
fn test_bytes_required_boundaries() {
    // Exact byte boundaries must not be overestimated, and usize::MAX must not overflow.
    assert_eq!(0usize.bytes_required(), 1);
    assert_eq!(255usize.bytes_required(), 1);
    assert_eq!(256usize.bytes_required(), 2);
    assert_eq!(65535usize.bytes_required(), 2);
    assert_eq!(65536usize.bytes_required(), 3);
    assert_eq!(usize::MAX.bytes_required(), (usize::BITS / u8::BITS) as usize);

    // A round-trip through a vector with the maximum value must work.
    let mut vec = ByteCompressedVec::new();
    vec.push(usize::MAX);
    assert_eq!(vec.index(0), usize::MAX);
}

/// A value is stored in exactly the bytes it requires, least significant byte first.
#[test]
fn test_random_usize_entry() {
    random_test(100, |rng| {
        let width = rng.random_range(1..=8);
        let value = value_of_width(rng, width);
        assert_eq!(value.bytes_required(), width as usize, "value {value:#x}");

        let mut bytes = vec![0; width as usize];
        value.to_bytes(&mut bytes);
        assert_eq!(bytes, value.to_le_bytes()[..width as usize]);
        assert_eq!(usize::from_bytes(&bytes), value);
    });
}

#[test]
fn test_resize_with_and_shrink_to_fit() {
    let mut vec = ByteCompressedVec::with_capacity(1000, 1);
    for i in 0..1000 {
        vec.push(i);
    }

    vec.resize_with(10, || 0);
    vec.shrink_to_fit();
    assert_matches(&vec, &(0..10).collect::<Vec<_>>());

    vec.resize_with(12, || 7);
    assert_matches(&vec, &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 7, 7]);

    vec.resize_with(0, || 0);
    assert_matches(&vec, &[]);
}

#[test]
fn test_resize_zeroed() {
    let mut vec: ByteCompressedVec<usize> = ByteCompressedVec::with_capacity(10, 4);
    vec.resize_zeroed(10, 4);
    assert_matches(&vec, &[0; 10]);

    // Entries can then be filled in out of order via `set`.
    vec.set(5, 12345);
    assert_eq!(vec.index(5), 12345);
    assert_eq!(vec.index(4), 0);
}

#[test]
fn test_swap() {
    let mut vec = ByteCompressedVec::new();
    vec.push(1);
    vec.push(256);
    vec.push(65536);

    vec.swap(0, 2);
    assert_matches(&vec, &[65536, 256, 1]);

    vec.swap(1, 1);
    assert_matches(&vec, &[65536, 256, 1]);
}

/// A small deterministic permutation with a fixed point and a cycle of length three.
#[test]
#[cfg_attr(miri, ignore)] // bitvec violates Stacked Borrows.
fn test_permute_small() {
    let elements = [10, 20, 30, 40];
    let permutation = [2, 1, 3, 0];

    // `permute` moves the element at position `i` to position `permutation[i]`.
    let mut permuted = ByteCompressedVec::with_iter(elements.iter().copied());
    permuted.permute(|i| permutation[i]);
    assert_matches(&permuted, &[40, 20, 10, 30]);

    // `permute_indices` puts the element at position `permutation[i]` at position `i`.
    let mut indexed = ByteCompressedVec::with_iter(elements.iter().copied());
    indexed.permute_indices(|i| permutation[i]);
    assert_matches(&indexed, &[30, 20, 40, 10]);

    let mut fast = ByteCompressedVec::with_iter(elements.iter().copied());
    fast.permute_indices_fast(|i| permutation[i]);
    assert_matches(&fast, &[30, 20, 40, 10]);
}

#[test]
#[cfg_attr(miri, ignore)] // bitvec violates Stacked Borrows.
fn test_random_permute() {
    random_test(100, |rng| {
        let elements: Vec<usize> = (0..rng.random_range(1..10)).map(|_| rng.random_range(0..100)).collect();
        let vec = ByteCompressedVec::with_iter(elements.iter().copied());

        let mut permutation: Vec<usize> = (0..elements.len()).collect();
        permutation.shuffle(rng);

        // `permute` moves the element at position `i` to position `permutation[i]`.
        let mut expected = vec![0; elements.len()];
        for (i, &target) in permutation.iter().enumerate() {
            expected[target] = elements[i];
        }
        let mut permuted = vec.clone();
        permuted.permute(|i| permutation[i]);
        assert_matches(&permuted, &expected);

        // `permute_indices` and `permute_indices_fast` put the element at position
        // `permutation[i]` at position `i`.
        let expected: Vec<usize> = permutation.iter().map(|&source| elements[source]).collect();
        let mut indexed = vec.clone();
        indexed.permute_indices(|i| permutation[i]);
        assert_matches(&indexed, &expected);

        let mut fast = vec.clone();
        fast.permute_indices_fast(|i| permutation[i]);
        assert_matches(&fast, &expected);
    });
}
