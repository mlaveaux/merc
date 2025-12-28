use std::io::Read;
use std::io::Write;
use std::io::{self};

use bitstream_io::BigEndian;
use bitstream_io::BitRead;
use bitstream_io::BitReader;
use bitstream_io::BitWrite;
use bitstream_io::BitWriter;
use log::error;

use merc_number::read_u64_variablelength;
use merc_number::write_u64_variablelength;
use merc_utilities::MercError;

/// Trait for writing bit-level data.
pub trait BitStreamWrite {
    /// Writes the least significant bits from a u64 value.
    ///
    /// # Preconditions
    /// - number_of_bits must be <= 64
    fn write_bits(&mut self, value: u64, number_of_bits: u8) -> Result<(), MercError>;

    /// Writes a string prefixed with its length as a variable-width integer.
    fn write_string(&mut self, s: &str) -> Result<(), MercError>;

    /// Writes a u64 value using variable-width encoding.
    fn write_integer(&mut self, value: u64) -> Result<(), MercError>;

    /// Flushes any remaining bits to the underlying writer.
    fn flush(&mut self) -> Result<(), MercError>;
}

/// Trait for reading bit-level data.
pub trait BitStreamRead {
    /// Reads bits into the least significant bits of a u64.
    ///
    /// # Preconditions
    /// - number_of_bits must be <= 64
    fn read_bits(&mut self, number_of_bits: u8) -> Result<u64, MercError>;

    /// Reads a length-prefixed string.
    fn read_string(&mut self) -> Result<String, MercError>;

    /// Reads a variable-width encoded integer.
    fn read_integer(&mut self) -> Result<u64, MercError>;
}

/// Writer for bit-level output operations using an underlying writer.
pub struct BitStreamWriter<W: Write> {
    writer: BitWriter<W, BigEndian>,
}

impl<W: Write> BitStreamWriter<W> {
    /// Creates a new BitStreamWriter wrapping the provided writer.
    pub fn new(writer: W) -> Self {
        Self {
            writer: BitWriter::new(writer),
        }
    }
}

impl<W: Write> Drop for BitStreamWriter<W> {
    fn drop(&mut self) {
        if self.flush().is_err() {
            error!("Panicked while flushing the stream when dropped!")
        }
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
}

impl<W: Write> BitStreamWrite for BitStreamWriter<W> {
    fn write_bits(&mut self, value: u64, number_of_bits: u8) -> Result<(), MercError> {
        debug_assert!(number_of_bits <= 64);
        Ok(self.writer.write_var(number_of_bits as u32, value)?)
    }

    fn write_string(&mut self, s: &str) -> Result<(), MercError> {
        self.write_integer(s.len() as u64)?;
        for byte in s.as_bytes() {
            self.writer.write::<8, u64>(*byte as u64)?;
        }
        Ok(())
    }

    fn write_integer(&mut self, value: u64) -> Result<(), MercError> {
        write_u64_variablelength(&mut self.writer, value)?;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MercError> {
        self.writer.byte_align()?;
        Ok(self.writer.flush()?)
    }
}

impl<R: Read> BitStreamRead for BitStreamReader<R> {
    fn read_bits(&mut self, number_of_bits: u8) -> Result<u64, MercError> {
        assert!(number_of_bits <= 64);
        Ok(self.reader.read_var(number_of_bits as u32)?)
    }

    fn read_string(&mut self) -> Result<String, MercError> {
        let length = self.read_integer()?;
        self.text_buffer.clear();
        self.text_buffer
            .reserve(length.try_into().expect("String size exceeds usize!"));

        for _ in 0..length {
            let byte = self.reader.read::<8, u8>()?;
            self.text_buffer.push(byte);
        }

        Ok(String::from_utf8(self.text_buffer.clone()).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?)
    }

    fn read_integer(&mut self) -> Result<u64, MercError> {
        read_u64_variablelength(&mut self.reader)
    }
}

#[cfg(test)]
mod tests {
    use log::debug;
    use merc_utilities::random_test;
    use rand::Rng;
    use rand::distr::Alphanumeric;

    use super::*;

    /// Decide (arbitrarily) what to write into the bitstream.
    #[derive(Debug)]
    enum Instruction {
        String(String),
        Integer(u64),
        /// (value, num_of_bits), where num_of_bits must be at most 64.
        Bits(u64, u8),
    }

    /// Calculate minimum bits needed to represent the value
    /// Use 1 bit if value is 0 to ensure at least 1 bit is written
    pub fn required_bits(value: u64) -> u8 {
        if value == 0 {
            1
        } else {
            64 - value.leading_zeros() as u8
        }
    }

    #[test]
    fn test_arbitrary_bitstream() {
        random_test(100, |rng| {
            let instructions: Vec<Instruction> = (0..100)
                .map(|_| match rng.random_range(0..2) {
                    0 => {
                        let string = rng.sample_iter(&Alphanumeric).take(7).map(char::from).collect();
                        Instruction::String(string)
                    }
                    1 => Instruction::Integer(rng.random()),
                    2 => {
                        let value: u64 = rng.random();
                        Instruction::Bits(value, required_bits(value))
                    }
                    _ => unreachable!("The range is from 0 to 2"),
                })
                .collect();

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
        });
    }
}
