use std::convert::Infallible;

use num::PrimInt;
use num::Unsigned;
use rand::RngExt;

use merc_number::encoding_size;
use merc_number::read_variablelength;
use merc_number::write_variablelength;
use merc_utilities::random_test;

/// Encodes `value` and returns the bytes.
fn encode<T: PrimInt + Unsigned + From<u8>>(value: T) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_variablelength(value, |byte| {
        bytes.push(byte);
        Ok::<(), Infallible>(())
    })
    .unwrap();
    bytes
}

/// Round-trips `value` through the encoder and decoder, and checks that the encoding uses one
/// byte per started group of seven significant bits.
fn roundtrip<T: PrimInt + Unsigned + From<u8> + std::fmt::Debug>(value: T) {
    let bytes = encode(value);
    let significant_bits = (T::zero().count_zeros() - value.leading_zeros()).max(1) as usize;
    assert_eq!(
        bytes.len(),
        significant_bits.div_ceil(7),
        "length of the encoding of {value:?}"
    );
    assert!(bytes.len() <= encoding_size::<T>());

    let mut bytes = bytes.into_iter();
    assert_eq!(
        read_variablelength::<T, _>(|| bytes.next().ok_or("missing byte")).unwrap(),
        value
    );
    assert!(bytes.next().is_none(), "All bytes must be read");
}

#[test]
fn test_encoding_size() {
    assert_eq!(encoding_size::<u8>(), 2);
    assert_eq!(encoding_size::<u16>(), 3);
    assert_eq!(encoding_size::<u32>(), 5);
    assert_eq!(encoding_size::<u64>(), 10);
}

/// The byte format is fixed (it is shared with the mCRL2 binary formats): seven bits per byte,
/// least significant group first, with the most significant bit set on all but the last byte.
#[test]
fn test_exact_encoding() {
    assert_eq!(encode(0u64), [0x00]);
    assert_eq!(encode(1u64), [0x01]);
    assert_eq!(encode(127u64), [0x7f]);
    assert_eq!(encode(128u64), [0x80, 0x01]);
    assert_eq!(encode(300u64), [0xac, 0x02]);
    assert_eq!(encode(16_384u64), [0x80, 0x80, 0x01]);
    assert_eq!(encode(u8::MAX), [0xff, 0x01]);

    let mut max = vec![0xff; 9];
    max.push(0x01);
    assert_eq!(encode(u64::MAX), max);
}

#[test]
fn test_boundary_encoding() {
    // Edge values and the per-byte continuation boundaries where the encoded length grows by one
    // byte (7 payload bits per byte).
    roundtrip(0u64);
    roundtrip(u64::MAX);
    for shift in [7, 14, 21, 28, 35, 42, 49, 56, 63] {
        roundtrip((1u64 << shift) - 1);
        roundtrip(1u64 << shift);
    }

    // Smaller types use fewer bytes, up to their own maximum.
    roundtrip(u8::MAX);
    roundtrip(u16::MAX);
    roundtrip(u32::MAX);
    roundtrip(1u32 << 28);
}

#[test]
fn test_read_propagates_error() {
    // The continuation bit promises a second byte that never comes.
    let mut bytes = [0x80u8].into_iter();
    assert_eq!(
        read_variablelength::<u64, _>(|| bytes.next().ok_or("missing byte")),
        Err("missing byte")
    );
}

#[test]
fn test_random_integer_encoding() {
    random_test(1000, |rng| {
        roundtrip(rng.random::<u64>());
        roundtrip(rng.random::<u32>());
    });
}
