use std::io::Read;
use std::io::Write;
use std::io::{self};

use log::error;

use merc_number::read_variablelength;
use merc_number::write_variablelength;
use merc_utilities::MercError;

use crate::BitReader;
use crate::BitWriter;

/// Trait for writing bit-level data.
pub trait BitStreamWrite {
    /// Writes the low `number_of_bits` bits of `value`.
    ///
    /// # Errors
    ///
    /// Returns an error if `number_of_bits` exceeds 64, or `value` does not fit in it.
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
    /// Reads `number_of_bits` bits into the low bits of a u64.
    ///
    /// # Errors
    ///
    /// Returns an error if `number_of_bits` exceeds 64.
    fn read_bits(&mut self, number_of_bits: u8) -> Result<u64, MercError>;

    /// Reads a length-prefixed string.
    ///
    /// # Errors
    ///
    /// Returns an error if the encoded length does not fit in a `usize`, the
    /// bytes cannot be allocated, or the bytes are not valid UTF-8.
    fn read_string(&mut self) -> Result<String, MercError>;

    /// Reads a variable-width encoded integer.
    fn read_integer(&mut self) -> Result<u64, MercError>;
}

/// Bit-level writer wrapping an underlying byte writer.
pub struct BitStreamWriter<W: Write> {
    writer: BitWriter<W>,
}

impl<W: Write> BitStreamWriter<W> {
    /// Wraps `writer` for bit-level output.
    pub fn new(writer: W) -> Self {
        Self {
            writer: BitWriter::new(writer),
        }
    }
}

impl<W: Write> Drop for BitStreamWriter<W> {
    fn drop(&mut self) {
        if self.flush().is_err() {
            error!("Error flushing the stream when dropped!")
        }
    }
}

/// Bit-level reader wrapping an underlying byte reader.
pub struct BitStreamReader<R: Read> {
    reader: BitReader<R>,
    text_buffer: Vec<u8>,
}

impl<R: Read> BitStreamReader<R> {
    /// Wraps `reader` for bit-level input.
    pub fn new(reader: R) -> Self {
        Self {
            reader: BitReader::new(reader),
            text_buffer: Vec::with_capacity(128),
        }
    }
}

impl<W: Write> BitStreamWrite for BitStreamWriter<W> {
    #[inline]
    fn write_bits(&mut self, value: u64, number_of_bits: u8) -> Result<(), MercError> {
        Ok(self.writer.write_bits(value, u32::from(number_of_bits))?)
    }

    fn write_string(&mut self, s: &str) -> Result<(), MercError> {
        self.write_integer(s.len() as u64)?;
        for byte in s.bytes() {
            self.writer.write_bits(u64::from(byte), 8)?;
        }
        Ok(())
    }

    fn write_integer(&mut self, value: u64) -> Result<(), MercError> {
        Ok(write_variablelength(value, |byte| {
            self.writer.write_bits(u64::from(byte), 8)
        })?)
    }

    fn flush(&mut self) -> Result<(), MercError> {
        Ok(self.writer.flush()?)
    }
}

impl<R: Read> BitStreamRead for BitStreamReader<R> {
    #[inline]
    fn read_bits(&mut self, number_of_bits: u8) -> Result<u64, MercError> {
        Ok(self.reader.read_bits(u32::from(number_of_bits))?)
    }

    fn read_string(&mut self) -> Result<String, MercError> {
        let length = self.read_integer()?;
        let length_usize: usize = length
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "string length exceeds usize"))?;

        self.text_buffer.clear();
        self.text_buffer
            .try_reserve(length_usize)
            .map_err(|_| io::Error::new(io::ErrorKind::OutOfMemory, "string too large to allocate"))?;
        for _ in 0..length_usize {
            let byte = self.reader.read_bits(8)? as u8;
            self.text_buffer.push(byte);
        }

        // Validate in place and copy the result out, so `text_buffer` keeps its
        // allocation for reuse on the next call (taking it would reset it to empty).
        std::str::from_utf8(&self.text_buffer)
            .map(str::to_owned)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e).into())
    }

    fn read_integer(&mut self) -> Result<u64, MercError> {
        Ok(read_variablelength(|| self.reader.read_bits(8).map(|byte| byte as u8))?)
    }
}
