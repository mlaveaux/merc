use std::io;
use std::io::Read;

use rand::RngExt;

use merc_io::BitReader;
use merc_io::BitWriter;
use merc_utilities::random_test;

/// Packs the given bits, most significant first, into bytes padded with zero bits.
fn reference_encoding(fields: &[(u64, u32)]) -> Vec<u8> {
    let bits: Vec<bool> = fields
        .iter()
        .flat_map(|&(value, number_of_bits)| (0..number_of_bits).rev().map(move |i| (value >> i) & 1 == 1))
        .collect();

    bits.chunks(8)
        .map(|byte| {
            byte.iter()
                .chain(std::iter::repeat(&false))
                .take(8)
                .fold(0u8, |acc, &bit| (acc << 1) | u8::from(bit))
        })
        .collect()
}

/// Writes the fields, checks the bytes against the reference encoding, and reads them back.
fn check_fields(fields: &[(u64, u32)]) {
    let mut bytes = Vec::new();
    let mut writer = BitWriter::new(&mut bytes);
    for &(value, number_of_bits) in fields {
        writer.write_bits(value, number_of_bits).unwrap();
    }
    writer.flush().unwrap();
    assert_eq!(bytes, reference_encoding(fields));

    let mut reader = BitReader::new(&bytes[..]);
    for &(value, number_of_bits) in fields {
        assert_eq!(reader.read_bits(number_of_bits).unwrap(), value);
    }
}

/// A small deterministic case that crosses the 64-bit word boundary of both the writer and the
/// reader, including a full 64-bit field that is not aligned.
#[test]
fn test_fields_across_word_boundary() {
    check_fields(&[
        (0b101, 3),
        (u64::MAX, 64),
        (0, 0),
        (0x1234_5678_9abc_def0, 64),
        (1, 1),
        (0x7f, 7),
    ]);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_bits() {
    random_test(100, |rng| {
        let fields: Vec<(u64, u32)> = (0..200)
            .map(|_| {
                let number_of_bits = rng.random_range(0..=64);
                let value = if number_of_bits == 0 {
                    0
                } else {
                    rng.random::<u64>() >> (64 - number_of_bits)
                };
                (value, number_of_bits)
            })
            .collect();
        check_fields(&fields);
    });
}

#[test]
fn test_padding_and_end_of_stream() {
    let mut bytes = Vec::new();
    let mut writer = BitWriter::new(&mut bytes);
    writer.write_bits(0b10110, 5).unwrap();
    writer.flush().unwrap();
    assert_eq!(bytes, vec![0b1011_0000], "The last byte is padded with zero bits");

    let mut reader = BitReader::new(&bytes[..]);
    assert_eq!(reader.read_bits(5).unwrap(), 0b10110);
    assert_eq!(reader.read_bits(3).unwrap(), 0, "The padding reads as zero bits");
    assert_eq!(
        reader.read_bits(1).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof,
        "Reading past the end must fail"
    );
}

#[test]
fn test_read_stops_once_bits_are_available() {
    /// Returns the given bytes, after which reading would block.
    struct Partial<'a>(&'a [u8]);

    impl Read for Partial<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let read = self.0.len().min(buf.len());
            buf[..read].copy_from_slice(&self.0[..read]);
            self.0 = &self.0[read..];
            Ok(read)
        }
    }

    let mut reader = BitReader::new(Partial(&[0xab]));
    assert_eq!(reader.read_bits(8).unwrap(), 0xab);

    // Bytes beyond the requested bits that a single read returns are kept.
    let mut reader = BitReader::new(Partial(&[0xab, 0xcd]));
    assert_eq!(reader.read_bits(4).unwrap(), 0xa);
    assert_eq!(reader.read_bits(12).unwrap(), 0xbcd);
}

#[test]
fn test_invalid_requests_are_rejected() {
    let mut writer = BitWriter::new(Vec::new());
    assert!(writer.write_bits(0, 65).is_err(), "More than 64 bits must be rejected");
    assert!(writer.write_bits(0b100, 2).is_err(), "A value must fit in its bits");

    let mut reader = BitReader::new(&[0u8; 16][..]);
    assert_eq!(
        reader.read_bits(65).unwrap_err().kind(),
        io::ErrorKind::InvalidInput,
        "More than 64 bits must be rejected"
    );
}
