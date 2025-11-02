//!
//! # The binary aterm stream format
//!
//! 8:0 16:BAF_MAGIC 16:BAF_VERSION
//!
//! After that every packet started with a header
//! 2:Header
//!

#![forbid(unsafe_code)]

use std::collections::VecDeque;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;

use mcrl3_io::BitStreamRead;
use mcrl3_io::BitStreamReader;
use mcrl3_io::BitStreamWrite;
use mcrl3_io::BitStreamWriter;
use mcrl3_number::bits_for_value;
use mcrl3_utilities::IndexedSet;
use mcrl3_utilities::MCRL3Error;
use mcrl3_utilities::debug_trace;

use crate::ATerm;
use crate::ATermInt;
use crate::ATermIntRef;
use crate::ATermRef;
use crate::Protected;
use crate::Symb;
use crate::Symbol;
use crate::SymbolRef;
use crate::Term;
use crate::is_int_symbol;
use crate::is_int_term;

/// The magic value for a binary aterm format stream.
/// As of version 0x8305 the magic and version are written as 2 bytes not encoded as variable-width integers.
/// To ensure compatibility with older formats the previously variable-width encoding is mimicked by prefixing them with 1000 (0x8).
const BAF_MAGIC: u16 = 0x8baf;

/// The BAF_VERSION constant is the version number of the ATerms written in BAF format.
/// History:
/// - before 2013: version 0x0300
/// - 29 August 2013: version changed to 0x0301
/// - 23 November 2013: version changed to 0x0302 (introduction of index for variable types)
/// - 24 September 2014: version changed to 0x0303 (introduction of stochastic distribution)
/// - 2 April 2017: version changed to 0x0304 (removed a few superfluous fields in the format)
/// - 19 July 2019: version changed to 0x8305 (introduction of the streamable aterm format)
/// - 28 February 2020: version changed to 0x8306 (added ability to stream aterm_int, implemented structured streaming for all objects)
/// - 24 January 2023: version changed to 0x8307 (removed NoIndex from Variables, Boolean variables. Made the .lts format more compact by not storing states with a default probability 1)
/// - 6 August 2024: version changed to 0x8308 (introduced machine numbers)
const BAF_VERSION: u16 = 0x8308;

/// Each packet has a header consisting of a type.
/// Either indicates a function symbol, a term (either shared or output) or an arbitrary integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum PacketType {
    FunctionSymbol = 0,
    ATerm = 1,
    ATermOutput = 2,
    ATermIntOutput = 3,
}

/// The number of bits needed to store an element of PacketType.
const PACKET_BITS: u8 = 2;

impl From<u8> for PacketType {
    fn from(value: u8) -> Self {
        match value {
            0 => PacketType::FunctionSymbol,
            1 => PacketType::ATerm,
            2 => PacketType::ATermOutput,
            3 => PacketType::ATermIntOutput,
            _ => panic!("Invalid packet type: {value}"),
        }
    }
}

/// Trait for writing ATerms to a stream.
pub trait ATermWrite {
    /// Writes an ATerm to the stream.
    fn write_aterm(&mut self, term: &ATerm) -> Result<(), MCRL3Error>;

    /// Writes an iterator of ATerms to the stream.
    fn write_aterm_iter<I>(&mut self, iter: I) -> Result<(), MCRL3Error>
    where
        I: ExactSizeIterator<Item = ATerm>;

    /// Flushes any remaining data and writes the end-of-stream marker.
    ///
    /// This method should be called when you're done writing terms to ensure
    /// all data is properly written and the stream is correctly terminated.
    fn flush(&mut self) -> Result<(), MCRL3Error>;
}

/// Trait for reading ATerms from a stream.
pub trait ATermRead {
    /// Reads the next ATerm from the stream. Returns None when the end of the stream is reached.
    fn read_aterm(&mut self) -> Result<Option<ATerm>, MCRL3Error>;

    /// Reads an iterator of ATerms from the stream.
    fn read_aterm_iter(&mut self) -> Result<impl ExactSizeIterator<Item = Result<ATerm, MCRL3Error>>, MCRL3Error>;
}

/// Trait for objects that can be written to and read from an ATerm stream.
pub trait ATermStreamable {
    /// Writes the object to the given ATerm writer.
    fn write<W: ATermWrite>(&self, writer: &mut W) -> Result<(), MCRL3Error>;

    /// Reads the object from the given ATerm reader.
    fn read<R: ATermRead>(reader: &mut R) -> Result<Self, MCRL3Error>
    where
        Self: Sized;
}

/// Writes terms in a streamable binary aterm format to an output stream.
///
/// # The streamable aterm format:
///
/// Aterms (and function symbols) are written as packets (with an identifier in the header) and their
/// indices are derived from the number of aterms, resp. symbols, that occur before them in this stream. For each term
/// we first ensure that its arguments and symbol are written to the stream (avoiding duplicates). Then its
/// symbol index followed by a number of indices (depending on the arity) for its argments are written as integers.
/// Packet headers also contain a special value to indicate that the read term should be visible as output as opposed to
/// being only a subterm.
/// The start of the stream is a zero followed by a header and a version and a term with function symbol index zero
/// indicates the end of the stream.
///
pub struct BinaryATermWriter<W: Write> {
    stream: BitStreamWriter<W>,

    /// Stores the function symbols and the number of bits needed to encode their indices.
    function_symbols: IndexedSet<Symbol>,
    function_symbol_index_width: u8,

    /// Stores the terms and the number of bits needed to encode their indices.
    terms: IndexedSet<ATerm>,
    term_index_width: u8,

    /// Indicates whether the stream has been flushed.
    flushed: bool,

    /// Local stack to avoid recursive function calls when writing terms.
    stack: VecDeque<(ATerm, bool)>,
}

impl<W: Write> BinaryATermWriter<W> {
    /// Creates a new binary ATerm output stream with the given writer.
    ///
    /// # Arguments
    /// * `writer` - The underlying writer to write binary data to
    ///
    /// # Returns
    /// A new `BinaryATermOutputStream` instance or an error if header writing fails
    pub fn new(writer: W) -> Result<Self, MCRL3Error> {
        let mut stream = BitStreamWriter::new(writer);

        // Write the header of the binary aterm format
        stream.write_bits(0, 8)?;
        stream.write_bits(BAF_MAGIC as u64, 16)?;
        stream.write_bits(BAF_VERSION as u64, 16)?;

        let mut function_symbols = IndexedSet::new();
        // The term with function symbol index 0 indicates the end of the stream
        function_symbols.insert(Symbol::new("end_of_stream".to_string(), 0));

        Ok(Self {
            stream,
            function_symbols,
            function_symbol_index_width: 1,
            terms: IndexedSet::new(),
            term_index_width: 1,
            stack: VecDeque::new(),
            flushed: false,
        })
    }

    /// \brief Write a function symbol to the output stream.
    fn write_function_symbol(&mut self, symbol: &SymbolRef<'_>) -> Result<usize, MCRL3Error> {
        let (index, inserted) = self.function_symbols.insert(symbol.protect());

        if inserted {
            // Write the function symbol to the stream
            self.stream.write_bits(PacketType::FunctionSymbol as u64, PACKET_BITS)?;
            self.stream.write_string(symbol.name())?;
            self.stream.write_integer(symbol.arity() as u64)?;
            self.function_symbol_index_width = bits_for_value(self.function_symbols.len());
        }

        Ok(*index)
    }

    /// Returns the current bit width needed to encode a function symbol index.
    ///
    /// In debug builds, this asserts that the cached width equals the
    /// computed width based on the current number of function symbols.
    fn function_symbol_index_width(&self) -> u8 {
        let expected = bits_for_value(self.function_symbols.len());
        debug_assert_eq!(
            self.function_symbol_index_width, expected,
            "function_symbol_index_width does not match bits_for_value",
        );

        self.function_symbol_index_width
    }

    /// Returns the current bit width needed to encode a term index.
    ///
    /// In debug builds, this asserts that the cached width equals the
    /// computed width based on the current number of terms.
    fn term_index_width(&self) -> u8 {
        let expected = bits_for_value(self.terms.len());
        debug_assert_eq!(
            self.term_index_width, expected,
            "term_index_width does not match bits_for_value",
        );
        self.term_index_width
    }
}

impl<W: Write> ATermWrite for BinaryATermWriter<W> {
    fn write_aterm(&mut self, term: &ATerm) -> Result<(), MCRL3Error> {
        self.stack.push_back((term.clone(), false));

        while let Some((current_term, write_ready)) = self.stack.pop_back() {
            // Indicates that this term is output and not a subterm, these should always be written.
            let is_output = self.stack.is_empty();

            if !self.terms.contains(&current_term) || is_output {
                if write_ready {
                    if is_int_term(&current_term) {
                        let int_term = ATermIntRef::from(current_term.copy());
                        if is_output {
                            // If the integer is output, write the header and just an integer
                            self.stream.write_bits(PacketType::ATermIntOutput as u64, PACKET_BITS)?;
                            self.stream.write_integer(int_term.value() as u64)?;
                        } else {
                            let symbol_index = self.write_function_symbol(&int_term.get_head_symbol())?;

                            self.stream.write_bits(PacketType::ATerm as u64, PACKET_BITS)?;
                            self.stream
                                .write_bits(symbol_index as u64, self.function_symbol_index_width())?;
                            self.stream.write_integer(int_term.value() as u64)?;
                        }
                    } else {
                        let symbol_index = self.write_function_symbol(&current_term.get_head_symbol())?;
                        let packet_type = if is_output {
                            PacketType::ATermOutput
                        } else {
                            PacketType::ATerm
                        };

                        self.stream.write_bits(packet_type as u64, PACKET_BITS)?;
                        self.stream
                            .write_bits(symbol_index as u64, self.function_symbol_index_width())?;

                        for arg in current_term.arguments() {
                            let index = self.terms.index(&arg).expect("Argument must already be written");
                            self.stream.write_bits(*index as u64, self.term_index_width())?;
                        }
                    }

                    if !is_output {
                        let (_, inserted) = self.terms.insert(current_term);
                        assert!(inserted, "This term should have a new index assigned.");
                        self.term_index_width = bits_for_value(self.terms.len());
                    }
                } else {
                    // Add current term back to stack for writing after processing arguments
                    self.stack.push_back((current_term.clone(), true));

                    // Add arguments to stack for processing first
                    for arg in current_term.arguments() {
                        if !self.terms.contains(&arg) {
                            self.stack.push_back((arg.protect(), false));
                        }
                    }
                }
            }

            // This term was already written and as such should be skipped. This can happen if
            // one term has two equal subterms.
        }

        Ok(())
    }

    fn write_aterm_iter<I>(&mut self, iter: I) -> Result<(), MCRL3Error>
    where
        I: ExactSizeIterator<Item = ATerm>,
    {
        self.write_aterm(&ATermInt::new(iter.len()))?;
        for ldd in iter {
            self.write_aterm(&ldd)?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MCRL3Error> {
        // Write the end of stream marker
        self.stream.write_bits(PacketType::ATerm as u64, PACKET_BITS)?;
        self.stream.write_bits(0, self.function_symbol_index_width())?;
        self.stream.flush()?;
        self.flushed = true;
        Ok(())
    }
}

impl<W: Write> BitStreamWrite for BinaryATermWriter<W> {
    delegate::delegate! {
        to self.stream {
            fn write_bits(&mut self, value: u64, number_of_bits: u8) -> Result<(), MCRL3Error>;
            fn write_string(&mut self, s: &str) -> Result<(), MCRL3Error>;
            fn write_integer(&mut self, value: u64) -> Result<(), MCRL3Error>;
            fn flush(&mut self) -> Result<(), MCRL3Error>;
        }
    }
}

impl<W: Write> Drop for BinaryATermWriter<W> {
    fn drop(&mut self) {
        if !self.flushed {
            ATermWrite::flush(self).expect("Panicked while flushing the stream when dropped");
        }
    }
}

/// The reader counterpart of [`BinaryATermWriter`], which reads ATerms from a binary aterm input stream.
pub struct BinaryATermReader<R: Read> {
    stream: BitStreamReader<R>,

    /// Stores the function symbols read so far, and the width needed to encode their indices.
    function_symbols: Protected<Vec<SymbolRef<'static>>>,
    function_symbol_index_width: u8,

    /// Stores the terms read so far, and the width needed to encode their indices.
    terms: Protected<Vec<ATermRef<'static>>>,
    term_index_width: u8,

    /// Indicates whether the end of stream marker has already been encountered.
    ended: bool,
}

impl<R: Read> BinaryATermReader<R> {
    /// Checks for the header and initializes the binary aterm input stream.
    pub fn new(reader: R) -> Result<Self, MCRL3Error> {
        let mut stream = BitStreamReader::new(reader);

        // Read the binary aterm format header
        if stream.read_bits(8)? != 0 || stream.read_bits(16)? != BAF_MAGIC as u64 {
            return Err(Error::new(ErrorKind::InvalidData, "Missing BAF_MAGIC control sequence").into());
        }

        let version = stream.read_bits(16)?;
        if version != BAF_VERSION as u64 {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("BAF version ({version}) incompatible with expected version ({BAF_VERSION})"),
            )
            .into());
        }

        // The term with function symbol index 0 indicates the end of the stream
        let mut function_symbols = Protected::new(Vec::new());
        let end_of_stream_symbol = Symbol::new(String::new(), 0);
        function_symbols.write().push(end_of_stream_symbol.copy());

        Ok(Self {
            stream,
            function_symbols,
            function_symbol_index_width: 1,
            terms: Protected::new(Vec::new()),
            term_index_width: 1,
            ended: false,
        })
    }

    /// Returns the current bit width needed to encode a function symbol index.
    ///
    /// In debug builds, this asserts that the cached width equals the
    /// computed width based on the current number of function symbols.
    fn function_symbol_index_width(&self) -> u8 {
        let expected = bits_for_value(self.function_symbols.read().len());
        debug_assert_eq!(
            self.function_symbol_index_width, expected,
            "function_symbol_index_width does not match bits_for_value",
        );

        self.function_symbol_index_width
    }

    /// Returns a mutable reference to the underlying bit stream reader.
    pub fn stream(&mut self) -> &mut BitStreamReader<R> {
        &mut self.stream
    }

    /// Returns the current bit width needed to encode a term index.
    ///
    /// In debug builds, this asserts that the cached width equals the
    /// computed width based on the current number of terms.
    fn term_index_width(&self) -> u8 {
        let expected = bits_for_value(self.terms.read().len());
        debug_assert_eq!(
            self.term_index_width, expected,
            "term_index_width does not match bits_for_value",
        );
        self.term_index_width
    }
}

impl<R: Read> ATermRead for BinaryATermReader<R> {
    fn read_aterm(&mut self) -> Result<Option<ATerm>, MCRL3Error> {
        if self.ended {
            return Err(Error::new(
                ErrorKind::UnexpectedEof,
                "Attempted to read_aterm() after end of stream",
            )
            .into());
        }

        loop {
            let header = self.stream.read_bits(PACKET_BITS)?;
            let packet = PacketType::from(header as u8);
            debug_trace!("Read packet: {:?}", packet);

            match packet {
                PacketType::FunctionSymbol => {
                    let name = self.stream.read_string()?;
                    let arity = self.stream.read_integer()? as usize;
                    let symbol = Symbol::new(name, arity);
                    debug_trace!("Read symbol {symbol}");

                    let mut write_symbols = self.function_symbols.write();
                    let s = write_symbols.protect_symbol(&symbol);
                    write_symbols.push(s);
                    self.function_symbol_index_width = bits_for_value(write_symbols.len());
                }
                PacketType::ATermIntOutput => {
                    let value = self.stream.read_integer()?.try_into()?;
                    debug_trace!("Output int term: {}", ATermInt::new(value));
                    return Ok(Some(ATermInt::new(value).into()));
                }
                PacketType::ATerm | PacketType::ATermOutput => {
                    let symbol_index = self.stream.read_bits(self.function_symbol_index_width())? as usize;
                    if symbol_index == 0 {
                        // End of stream marker
                        debug_trace!("End of stream marker reached");
                        self.ended = true;
                        return Ok(None);
                    }

                    let symbols = self.function_symbols.read();
                    let symbol = symbols.get(symbol_index).ok_or(format!(
                        "Read invalid function symbol index {symbol_index}, length {}",
                        symbols.len()
                    ))?;

                    if is_int_symbol(symbol) {
                        let value = self.stream.read_integer()?.try_into()?;
                        let term = ATermInt::new(value);
                        debug_trace!("Read int term: {term}");

                        let mut write_terms = self.terms.write();
                        let t = write_terms.protect(&term);
                        write_terms.push(t);
                        self.term_index_width = bits_for_value(write_terms.len());
                    } else {
                        let mut arguments = Vec::with_capacity(symbol.arity());

                        // When the arity is zero, no bits are read for the arguments.
                        let num_of_bits = if symbol.arity() > 0 { self.term_index_width() } else { 0 };
                        let mut write_terms = self.terms.write();
                        for _ in 0..symbol.arity() {
                            let arg_index = self.stream.read_bits(num_of_bits)? as usize;
                            let arg = write_terms.get(arg_index).ok_or(format!(
                                "Read invalid aterm index {arg_index}, length {}",
                                write_terms.len()
                            ))?;
                            debug_trace!("Read arg: {arg}");
                            arguments.push(arg);
                        }

                        let term = ATerm::with_args(symbol, &arguments);

                        if packet == PacketType::ATermOutput {
                            debug_trace!("Output term: {term}");
                            return Ok(Some(term));
                        }
                        debug_trace!("Read term: {term}");

                        let t = write_terms.protect(&term);
                        write_terms.push(t);
                        self.term_index_width = bits_for_value(write_terms.len());
                    }
                }
            }
        }
    }

    fn read_aterm_iter(&mut self) -> Result<impl ExactSizeIterator<Item = Result<ATerm, MCRL3Error>>, MCRL3Error> {
        if self.ended {
            return Err(Error::new(
                ErrorKind::UnexpectedEof,
                "Attempted to read_aterm_iter() after end of stream",
            )
            .into());
        }

        let number_of_elements: ATermInt = self
            .read_aterm()?
            .ok_or("Missing number of elements for iterator")?
            .into();
        Ok(ATermReadIter {
            reader: self,
            remaining: number_of_elements.value(),
        })
    }
}

impl<R: Read> BitStreamRead for BinaryATermReader<R> {
    delegate::delegate! {
        to self.stream {
            fn read_bits(&mut self, number_of_bits: u8) -> Result<u64, MCRL3Error>;
            fn read_string(&mut self) -> Result<String, MCRL3Error>;
            fn read_integer(&mut self) -> Result<u64, MCRL3Error>;
        }
    }
}

/// A read iterator for ATerms from a binary aterm input stream.
pub struct ATermReadIter<'a, R: Read> {
    reader: &'a mut BinaryATermReader<R>,
    remaining: usize,
}

impl<'a, R: Read> Iterator for ATermReadIter<'a, R> {
    type Item = Result<ATerm, MCRL3Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        self.remaining -= 1;
        match self.reader.read_aterm() {
            Ok(Some(term)) => Some(Ok(term)),
            Ok(None) => Some(Err(Error::new(
                ErrorKind::UnexpectedEof,
                "Unexpected end of stream while reading iterator",
            )
            .into())),
            Err(e) => Some(Err(e)),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<'a, R: Read> ExactSizeIterator for ATermReadIter<'a, R> {
    fn len(&self) -> usize {
        self.remaining
    }
}

#[cfg(test)]
mod tests {
    use mcrl3_utilities::random_test;

    use crate::random_term;

    use super::*;

    #[test]
    fn test_random_binary_stream() {
        random_test(1, |rng| {
            let input: Vec<_> = (0..20)
                .map(|_| random_term(rng, &[("f".into(), 2), ("g".into(), 1)], &["a".into(), "b".into()], 1))
                .collect();

            let mut stream: Vec<u8> = Vec::new();

            let mut output_stream = BinaryATermWriter::new(&mut stream).unwrap();
            for term in &input {
                output_stream.write_aterm(term).unwrap();
            }
            ATermWrite::flush(&mut output_stream).expect("Flushing the output to the stream");
            drop(output_stream); // Explicitly drop to release the mutable borrow

            let mut input_stream = BinaryATermReader::new(&stream[..]).unwrap();
            for term in &input {
                println!("Term {}", term);
                debug_assert_eq!(
                    *term,
                    input_stream.read_aterm().unwrap().unwrap(),
                    "The read term must match the term that we have written"
                );
            }
        });
    }

    #[test]
    fn test_random_binary_stream_iter() {
        random_test(1, |rng| {
            let input: Vec<_> = (0..20)
                .map(|_| random_term(rng, &[("f".into(), 2), ("g".into(), 1)], &["a".into(), "b".into()], 1))
                .collect();

            let mut stream: Vec<u8> = Vec::new();

            let mut output_stream = BinaryATermWriter::new(&mut stream).unwrap();
            output_stream.write_aterm_iter(input.iter().cloned()).unwrap();
            ATermWrite::flush(&mut output_stream).expect("Flushing the output to the stream");
            drop(output_stream); // Explicitly drop to release the mutable borrow

            let mut input_stream = BinaryATermReader::new(&stream[..]).unwrap();
            let read_iter = input_stream.read_aterm_iter().unwrap();
            for (term_written, term_read) in input.iter().zip(read_iter) {
                let term_read = term_read.expect("Reading term from stream must succeed");
                println!("Term {}", term_written);
                debug_assert_eq!(
                    *term_written, term_read,
                    "The read term must match the term that we have written"
                );
            }
        });
    }
}
