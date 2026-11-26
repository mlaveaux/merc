use rand::RngExt;
use rand::distr::Alphanumeric;

use merc_io::BitStreamRead;
use merc_io::BitStreamReader;
use merc_io::BitStreamWrite;
use merc_io::BitStreamWriter;
use merc_utilities::random_test;

/// Decide (arbitrarily) what to write into the bitstream.
#[derive(Debug)]
enum Instruction {
    String(String),
    Integer(u64),
    /// (value, num_of_bits), where num_of_bits must be at most 64.
    Bits(u64, u8),
}

/// Encodes the instructions bit by bit, independently of the writer, as the reference for the
/// format: most significant bit first, integers in groups of seven bits with the least
/// significant group first, and zero bits to pad the last byte.
fn reference_encoding(instructions: &[Instruction]) -> Vec<u8> {
    fn push(bits: &mut Vec<bool>, value: u64, number_of_bits: u8) {
        for i in (0..number_of_bits).rev() {
            bits.push((value >> i) & 1 == 1);
        }
    }

    fn push_integer(bits: &mut Vec<bool>, mut value: u64) {
        while value > 0b0111_1111 {
            push(bits, (value & 0b0111_1111) | 0b1000_0000, 8);
            value >>= 7;
        }
        push(bits, value, 8);
    }

    let mut bits = Vec::new();
    for instruction in instructions {
        match instruction {
            Instruction::String(string) => {
                push_integer(&mut bits, string.len() as u64);
                for byte in string.bytes() {
                    push(&mut bits, u64::from(byte), 8);
                }
            }
            Instruction::Integer(value) => push_integer(&mut bits, *value),
            Instruction::Bits(value, number_of_bits) => push(&mut bits, *value, *number_of_bits),
        }
    }

    bits.chunks(8)
        .map(|byte| {
            byte.iter()
                .chain(std::iter::repeat(&false))
                .take(8)
                .fold(0u8, |acc, &bit| (acc << 1) | u8::from(bit))
        })
        .collect()
}

/// Writes the instructions, checks the bytes against the reference encoding and reads them back.
fn check_instructions(instructions: &[Instruction]) {
    let mut buffer = Vec::new();
    {
        let mut writer = BitStreamWriter::new(&mut buffer);
        for instruction in instructions {
            match instruction {
                Instruction::String(string) => writer.write_string(string).unwrap(),
                Instruction::Integer(value) => writer.write_integer(*value).unwrap(),
                Instruction::Bits(value, number_of_bits) => writer.write_bits(*value, *number_of_bits).unwrap(),
            }
        }
        writer.flush().unwrap();
    }

    assert_eq!(
        buffer,
        reference_encoding(instructions),
        "The encoding must match the reference"
    );

    let mut reader = BitStreamReader::new(&buffer[..]);
    for instruction in instructions {
        match instruction {
            Instruction::String(string) => assert_eq!(reader.read_string().unwrap(), *string),
            Instruction::Integer(value) => assert_eq!(reader.read_integer().unwrap(), *value),
            Instruction::Bits(value, number_of_bits) => {
                assert_eq!(reader.read_bits(*number_of_bits).unwrap(), *value)
            }
        }
    }
}

/// A small deterministic mix in which no integer or string is byte aligned.
#[test]
fn test_mixed_bitstream() {
    check_instructions(&[
        Instruction::Bits(0b101, 3),
        Instruction::Integer(300),
        Instruction::String("merc".to_string()),
        Instruction::Bits(u64::MAX, 64),
        Instruction::Integer(u64::MAX),
        Instruction::String(String::new()),
        Instruction::Bits(1, 1),
    ]);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_bitstream() {
    random_test(100, |rng| {
        let instructions: Vec<Instruction> = (0..100)
            .map(|_| match rng.random_range(0..3) {
                0 => {
                    let string = rng.sample_iter(&Alphanumeric).take(7).map(char::from).collect();
                    Instruction::String(string)
                }
                1 => Instruction::Integer(rng.random()),
                2 => {
                    let value: u64 = rng.random();
                    // The minimal number of bits for the value, at least one.
                    let number_of_bits = (64 - value.leading_zeros()).max(1) as u8;
                    Instruction::Bits(value, number_of_bits)
                }
                _ => unreachable!("The range is from 0 to 3"),
            })
            .collect();
        check_instructions(&instructions);
    });
}

/// Writes the given strings and reads them back, asserting they round-trip.
fn roundtrip_strings(strings: &[&str]) {
    let mut buffer = Vec::new();
    {
        let mut writer = BitStreamWriter::new(&mut buffer);
        for s in strings {
            writer.write_string(s).unwrap();
        }
        writer.flush().unwrap();
    }

    let mut reader = BitStreamReader::new(&buffer[..]);
    for s in strings {
        assert_eq!(&reader.read_string().unwrap(), s);
    }
}

#[test]
fn test_string_edge_cases() {
    // Empty strings, multi-byte UTF-8, and a long string all in one stream, so that strings of
    // differing length are read one after the other.
    roundtrip_strings(&["", "a", "", "héllo wörld", "🦀∑≈ç", &"x".repeat(10_000), ""]);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_string_roundtrip_random_unicode() {
    random_test(100, |rng| {
        let strings: Vec<String> = (0..50)
            .map(|_| {
                let len = rng.random_range(0..32);
                (0..len).map(|_| rng.random::<char>()).collect()
            })
            .collect();
        let refs: Vec<&str> = strings.iter().map(String::as_str).collect();
        roundtrip_strings(&refs);
    });
}

#[test]
fn test_read_string_rejects_invalid_utf8() {
    // Length 2 followed by an invalid UTF-8 sequence.
    let mut buffer = Vec::new();
    {
        let mut writer = BitStreamWriter::new(&mut buffer);
        writer.write_integer(2).unwrap();
        writer.write_bits(0xFF, 8).unwrap();
        writer.write_bits(0xFE, 8).unwrap();
        writer.flush().unwrap();
    }

    let mut reader = BitStreamReader::new(&buffer[..]);
    assert!(reader.read_string().is_err(), "Invalid UTF-8 should produce an error");
}

#[test]
fn test_integer_boundaries() {
    // Edge values and the boundaries at which the encoding grows by a byte, written after three
    // bits so that no integer is byte aligned.
    let mut values = vec![0, u64::MAX];
    for shift in (7..64).step_by(7) {
        values.extend([(1u64 << shift) - 1, 1u64 << shift]);
    }

    let mut instructions = vec![Instruction::Bits(0b101, 3)];
    instructions.extend(values.into_iter().map(Instruction::Integer));
    check_instructions(&instructions);
}

/// Dropping the writer flushes the pending bits.
#[test]
fn test_drop_flushes() {
    let mut buffer = Vec::new();
    {
        let mut writer = BitStreamWriter::new(&mut buffer);
        writer.write_bits(0b10110, 5).unwrap();
    }
    assert_eq!(buffer, vec![0b1011_0000]);
}
