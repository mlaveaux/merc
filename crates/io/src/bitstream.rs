use std::io::Read;
use std::io::Write;
use std::io::{self};

use bitstream_io::BigEndian;
use bitstream_io::BitRead;
use bitstream_io::BitReader;
use bitstream_io::BitWrite;
use bitstream_io::BitWriter;

use mcrl3_number::encoding_size;

/// Calculate minimum bits needed to represent the value
/// Use 1 bit if value is 0 to ensure at least 1 bit is written
pub fn required_bits(value: u64) -> u8 {
    if value == 0 {
        1
    } else {
        64 - value.leading_zeros() as u8
    }
}

/// Writer for bit-level output operations using an underlying writer.
pub struct BitStreamWriter<W: Write> {
    writer: BitWriter<W, BigEndian>,

    /// Buffer For variable-width integers
    integer_buffer: [u8; encoding_size::<u64>()],
}

impl<W: Write> BitStreamWriter<W> {
    /// Creates a new BitStreamWriter wrapping the provided writer.
    pub fn new(writer: W) -> Self {
        Self {
            writer: BitWriter::new(writer),
            integer_buffer: [0; encoding_size::<u64>()],
        }
    }

    /// Writes the least significant bits from a u64 value.
    ///
    /// # Preconditions
    /// - number_of_bits must be <= 64
    pub fn write_bits(&mut self, value: u64, number_of_bits: u8) -> io::Result<()> {
        assert!(number_of_bits <= 64);
        self.writer.write_var(number_of_bits as u32, value)
    }

    /// Writes a string prefixed with its length as a variable-width integer.
    pub fn write_string(&mut self, s: &str) -> io::Result<()> {
        self.write_integer(s.len() as u64)?;
        for byte in s.as_bytes() {
            self.writer.write::<8, u64>(*byte as u64)?;
        }
        Ok(())
    }

    /// Writes a usize value using variable-width encoding.
    pub fn write_integer(&mut self, value: u64) -> io::Result<()> {
        let nr_bytes = encode_variablesize_int(value, &mut self.integer_buffer);
        for i in 0..nr_bytes {
            self.writer.write::<8, u64>(self.integer_buffer[i] as u64)?;
        }
        Ok(())
    }

    /// Flushes any remaining bits to the underlying writer.
    pub fn flush(&mut self) -> io::Result<()> {
        self.writer.byte_align()?;
        self.writer.flush()
    }
}

impl<W: Write> Drop for BitStreamWriter<W> {
    fn drop(&mut self) {
        self.flush().expect("Panicked while flushing the stream when dropped");
    }
}

/// Reader for bit-level input operations from an underlying reader.
pub struct BitStreamReader<R: Read> {
    reader: BitReader<R, BigEndian>,
    text_buffer: Vec<u8>,
}

impl<R: Read> BitStreamReader<R> {
    /// Creates a new BitStreamReader wrapping the provided reader.
    pub fn new(reader: R) -> Self {
        Self {
            reader: BitReader::new(reader),
            text_buffer: Vec::with_capacity(128),
        }
    }

    /// Reads bits into the least significant bits of a u64.
    ///
    /// # Preconditions
    /// - number_of_bits must be <= 64
    pub fn read_bits(&mut self, number_of_bits: u8) -> io::Result<u64> {
        assert!(number_of_bits <= 64);
        self.reader.read_var(number_of_bits as u32)
    }

    /// Reads a length-prefixed string.
    pub fn read_string(&mut self) -> io::Result<String> {
        let length = self.read_integer()?;
        self.text_buffer.clear();
        self.text_buffer
            .reserve((length + 1).try_into().expect("String size exceeds usize!"));

        for _ in 0..length {
            let byte = self.reader.read::<8, u64>()? as u8;
            self.text_buffer.push(byte);
        }

        String::from_utf8(self.text_buffer.clone()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Reads a variable-width encoded integer.
    pub fn read_integer(&mut self) -> io::Result<u64> {
        decode_variablesize_int(self)
    }
}

/// Encodes a usize value using 7 bits per byte for the value and 1 bit to indicate
/// if more bytes follow. Writes the encoded bytes to the provided output buffer.
///
/// # Preconditions
/// - output buffer must have sufficient capacity (10 bytes for 64-bit integers)
///
/// # Returns
/// The number of bytes written to the output buffer
fn encode_variablesize_int(mut value: u64, output: &mut [u8]) -> usize {
    let mut output_size = 0;

    while value > 127 {
        output[output_size] = ((value & 127) as u8) | 128;
        value >>= 7;
        output_size += 1;
    }

    output[output_size] = value as u8;
    output_size + 1
}

/// Decodes a variable-width encoded integer from a BitStreamReader.
///
/// # Errors
/// - Reading from the underlying reader fails
/// - The encoded integer uses too many bytes
fn decode_variablesize_int<R: Read>(reader: &mut BitStreamReader<R>) -> io::Result<u64> {
    let mut value = 0u64;
    let max_bytes = (std::mem::size_of::<usize>() * 8).div_ceil(7);

    for i in 0..max_bytes {
        let byte = reader.read_bits(8)?;
        value |= (byte & 127) << (7 * i);

        if byte & 128 == 0 {
            return Ok(value);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "Failed to read integer: too many bytes",
    ))
}

#[cfg(test)]
mod tests {
    use arbitrary::Unstructured;
    use arbtest::arbitrary::Arbitrary;
    use log::debug;
    use mcrl3_utilities::test_logger;

    use super::*;

    /// Decide (arbitrarily) what to write into the bitstream.
    #[derive(Debug)]
    enum Instruction {
        String(String),
        Integer(u64),
        /// (value, num_of_bits), where num_of_bits must be at most 64.
        Bits(u64, u8),
    }

    impl Arbitrary<'_> for Instruction {
        fn arbitrary(u: &mut Unstructured<'_>) -> arbitrary::Result<Self> {
            match u.int_in_range(0..=2)? {
                0 => Ok(Instruction::String(u.arbitrary()?)),
                1 => Ok(Instruction::Integer(u.arbitrary()?)),
                2 => {
                    let value: u64 = u.arbitrary()?;
                    Ok(Instruction::Bits(value, required_bits(value)))
                }
                _ => unreachable!("The range is from 0 to 2"),
            }
        }
    }

    #[test]
    fn test_arbitrary_bitstream() {
        let _ = test_logger();

        arbtest::arbtest(|u| {
            let instructions: Vec<Instruction> = u.arbitrary()?;

            let mut buffer = Vec::new();
            {
                let mut writer = BitStreamWriter::new(&mut buffer);

                for inst in &instructions {
                    debug!("Writing {inst:?}");
                    match inst {
                        Instruction::String(string) => {
                            writer.write_string(string).expect("Failed to write into stream")
                        }
                        Instruction::Integer(value) => {
                            writer.write_integer(*value).expect("Failed to write into stream")
                        }
                        Instruction::Bits(value, number_of_bits) => writer
                            .write_bits(*value, *number_of_bits)
                            .expect("Failed to write into stream"),
                    }
                }

                writer.flush().expect("Failed to write into stream");
            }

            let mut reader = BitStreamReader::new(&buffer[..]);

            for inst in &instructions {
                debug!("Checking {inst:?}");
                match inst {
                    Instruction::String(string) => {
                        debug_assert_eq!(
                            reader.read_string().expect("Failed to read from stream"),
                            *string,
                            "Failed to read back the string"
                        )
                    }
                    Instruction::Integer(value) => {
                        debug_assert_eq!(
                            reader.read_integer().expect("Failed to read from stream"),
                            *value,
                            "Failed to read back the integer"
                        )
                    }
                    Instruction::Bits(value, number_of_bits) => {
                        debug_assert_eq!(
                            reader.read_bits(*number_of_bits).expect("Failed to read from stream"),
                            *value,
                            "Failed to read back the bits"
                        )
                    }
                }
            }

            Ok(())
        });
    }
}
