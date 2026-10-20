use std::collections::VecDeque;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;

use log::error;

use merc_collections::IndexedSet;
use merc_io::BitStreamRead;
use merc_io::BitStreamReader;
use merc_io::BitStreamWrite;
use merc_io::BitStreamWriter;
use merc_number::bits_for_value;
use merc_utilities::MercError;
use merc_utilities::debug_trace;

use crate::ATerm;
use crate::ATermInt;
use crate::ATermIntRef;
use crate::ATermRef;
use crate::Protected;
use crate::ProtectedSend;
use crate::Symb;
use crate::Symbol;
use crate::SymbolRef;
use crate::Term;
use crate::is_int_symbol;
use crate::is_int_term;
use crate::storage::THREAD_TERM_POOL;

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
/// - 28 February 2020: version changed to 0x8306 (added ability to stream aterm_int,
///   implemented structured streaming for all objects)
/// - 24 January 2023: version changed to 0x8307 (removed NoIndex from Variables, Boolean variables.
///   Made the .lts format more compact by not storing states with a default probability 1)
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
            // Unreachable in practice: every caller derives `value` from `read_bits(PACKET_BITS)`
            // with `PACKET_BITS == 2`, so it is always in `0..=3`. Kept as a defensive guard.
            _ => unreachable!("Invalid packet type: {value}"),
        }
    }
}

/// Resolves a `(name, arity)` pair read from a stream back into a `Symbol`, returning the pool's
/// reserved integer symbol rather than a fresh one when the pair names it.
fn resolve_read_symbol(name: &str, arity: usize) -> Symbol {
    THREAD_TERM_POOL.with(|tp| {
        let int_symbol = tp.int_symbol();

        if arity == int_symbol.arity() && name == int_symbol.name() {
            int_symbol.protect()
        } else {
            Symbol::new(name, arity)
        }
    })
}

/// Trait for writing ATerms to a stream.
pub trait ATermWrite {
    /// Writes an ATerm to the stream.
    fn write_aterm(&mut self, term: &ATerm) -> Result<(), MercError>;

    /// Writes an integer to the stream, read back by [`ATermRead::read_aterm_int`] or as an
    /// [`ATermInt`] by [`ATermRead::read_aterm`].
    fn write_aterm_int(&mut self, value: usize) -> Result<(), MercError> {
        self.write_aterm(&ATermInt::new(value))
    }

    /// Writes an iterator of ATerms to the stream.
    fn write_aterm_iter<I>(&mut self, iter: I) -> Result<(), MercError>
    where
        I: ExactSizeIterator<Item = ATerm>;

    /// Flushes any remaining data and writes the end-of-stream marker.
    ///
    /// This method should be called when you're done writing terms to ensure
    /// all data is properly written and the stream is correctly terminated.
    fn flush(&mut self) -> Result<(), MercError>;
}

/// Trait for reading ATerms from a stream.
pub trait ATermRead {
    /// Reads the next ATerm from the stream. Returns None when the end of the stream is reached.
    fn read_aterm(&mut self) -> Result<Option<ATerm>, MercError>;

    /// Reads an integer written by [`ATermWrite::write_aterm_int`], or an [`ATermInt`] written
    /// as a term.
    ///
    /// # Errors
    ///
    /// Returns an error if the next term is not an integer, or the stream has ended.
    fn read_aterm_int(&mut self) -> Result<usize, MercError> {
        let term = self
            .read_aterm()?
            .ok_or("Expected an integer, found the end of the stream")?;
        if !is_int_term(&term) {
            return Err(format!("Expected an integer, found term {term}").into());
        }
        Ok(ATermInt::from(term).value())
    }

    /// Reads an iterator of ATerms from the stream.
    fn read_aterm_iter(&mut self) -> Result<impl ExactSizeIterator<Item = Result<ATerm, MercError>>, MercError>;
}

/// Trait for objects that can be written to and read from an ATerm stream.
pub trait ATermStreamable {
    /// Writes the object to the given ATerm writer.
    fn write<W: ATermWrite>(&self, writer: &mut W) -> Result<(), MercError>;

    /// Reads the object from the given ATerm reader.
    fn read<R: ATermRead>(reader: &mut R) -> Result<Self, MercError>
    where
        Self: Sized;
}

/// Writes terms in a streamable binary aterm format to an output stream.
///
/// # The streamable aterm format
///
/// Terms and function symbols are written as packets, each identified by a header, with indices
/// derived from how many terms (resp. symbols) occurred before them in the stream. For each term,
/// its arguments and symbol are written first (skipping ones already written), then its symbol
/// index followed by one index per argument. A packet header also carries a flag marking the term
/// as top-level output rather than only a subterm. The stream opens with a zero byte, the magic
/// value and the version; a term with function symbol index zero marks the end of the stream.
///
/// The term pool stays locked while a term is written, so the underlying writer must not wait for
/// a thread that creates terms.
pub struct BinaryATermWriter<W: Write> {
    stream: BitStreamWriter<W>,

    /// Stores the function symbols and the number of bits needed to encode their indices.
    function_symbols: ProtectedSend<IndexedSet<SymbolRef<'static>>>,
    function_symbol_index_width: u8,

    /// Stores the terms and the number of bits needed to encode their indices.
    terms: ProtectedSend<IndexedSet<ATermRef<'static>>>,
    term_index_width: u8,

    /// Indicates whether the stream has been flushed.
    flushed: bool,

    /// Local stack to avoid recursive function calls when writing terms.
    stack: ProtectedSend<VecDeque<(ATermRef<'static>, bool)>>,
}

impl<W: Write> BinaryATermWriter<W> {
    /// Creates a new binary ATerm output stream with the given writer.
    pub fn new(writer: W) -> Result<Self, MercError> {
        let mut stream = BitStreamWriter::new(writer);

        // Write the header of the binary aterm format
        stream.write_bits(0, 8)?;
        stream.write_bits(BAF_MAGIC as u64, 16)?;
        stream.write_bits(BAF_VERSION as u64, 16)?;

        let mut function_symbols = ProtectedSend::new(IndexedSet::new());
        // The term with function symbol index 0 indicates the end of the stream
        let end_of_stream_symbol = Symbol::new("end_of_stream".to_string(), 0);
        function_symbols.write().insert(end_of_stream_symbol.copy());

        Ok(Self {
            stream,
            function_symbols,
            function_symbol_index_width: 1,
            terms: ProtectedSend::new(IndexedSet::new()),
            term_index_width: 1,
            stack: ProtectedSend::new(VecDeque::new()),
            flushed: false,
        })
    }

    /// Returns the current bit width needed to encode a function symbol index.
    fn function_symbol_index_width(&self) -> u8 {
        let expected = bits_for_value(self.function_symbols.read().len());
        debug_assert_eq!(
            self.function_symbol_index_width, expected,
            "function_symbol_index_width does not match bits_for_value",
        );

        self.function_symbol_index_width
    }
}

impl<W: Write> BinaryATermWriter<W> {
    /// Writes `term` and every subterm that was not written before, children first, using an
    /// explicit stack instead of recursion.
    fn write_aterm_impl(&mut self, term: &ATerm) -> Result<(), MercError> {
        // The guards hold the global term pool for reading, so no garbage collection runs while
        // they exist and the term references taken from them remain valid.
        let mut stack = self.stack.write();
        let mut terms = self.terms.write();
        let mut function_symbols = self.function_symbols.write();

        // A previous call that failed may have left entries behind.
        stack.clear();
        stack.push_back((term.copy(), false));

        // An entry is visited once to queue its unwritten arguments, and written when all its
        // arguments have been written, which is at the first visit when there are none.
        while let Some((current, visited)) = stack.back().map(|(term, visited)| (term.copy(), *visited)) {
            // The term at the bottom of the stack is the output, which is always written.
            let is_output = stack.len() == 1;

            if !visited {
                if !is_output && terms.contains(&current) {
                    // Written before, possibly as an equal sibling earlier in this call.
                    stack.pop_back();
                    continue;
                }

                let position = stack.len() - 1;
                for arg in current.arguments() {
                    if !terms.contains(&arg) {
                        stack.push_back((arg, false));
                    }
                }

                if stack.len() - 1 != position {
                    stack[position].1 = true;
                    continue;
                }
            }

            if !(is_output && is_int_term(&current)) {
                write_function_symbol(
                    &mut self.stream,
                    &mut function_symbols,
                    &mut self.function_symbol_index_width,
                    current.get_head_symbol(),
                )?;
            }
            write_term_body(
                &mut self.stream,
                &function_symbols,
                self.function_symbol_index_width,
                &terms,
                self.term_index_width,
                &current,
                is_output,
            )?;

            if !is_output {
                let (_, inserted) = terms.insert(current);
                debug_assert!(inserted, "A term is only written once.");
                self.term_index_width = bits_for_value(terms.len());
            }
            stack.pop_back();
        }

        Ok(())
    }
}

/// Writes `symbol` as a packet unless it was written before, so that its index can be used.
fn write_function_symbol<'a, W: Write>(
    stream: &mut BitStreamWriter<W>,
    function_symbols: &mut IndexedSet<SymbolRef<'a>>,
    index_width: &mut u8,
    symbol: SymbolRef<'a>,
) -> Result<(), MercError> {
    if function_symbols.contains(&symbol) {
        return Ok(());
    }

    // Write the packet before inserting the symbol, so that a failed write leaves the table
    // consistent with the stream.
    stream.write_bits(PacketType::FunctionSymbol as u64, PACKET_BITS)?;
    stream.write_string(symbol.name())?;
    stream.write_integer(symbol.arity() as u64)?;

    function_symbols.insert(symbol);
    *index_width = bits_for_value(function_symbols.len());
    Ok(())
}

/// Writes the packet of `term`, whose symbol and arguments have already been written. An output
/// integer is written without its symbol.
fn write_term_body<W: Write>(
    stream: &mut BitStreamWriter<W>,
    function_symbols: &IndexedSet<SymbolRef<'_>>,
    function_symbol_index_width: u8,
    terms: &IndexedSet<ATermRef<'_>>,
    term_index_width: u8,
    term: &ATermRef<'_>,
    is_output: bool,
) -> Result<(), MercError> {
    debug_assert_eq!(function_symbol_index_width, bits_for_value(function_symbols.len()));
    debug_assert_eq!(term_index_width, bits_for_value(terms.len()));

    if is_int_term(term) {
        let value = ATermIntRef::from(term.copy()).value() as u64;
        if is_output {
            stream.write_bits(PacketType::ATermIntOutput as u64, PACKET_BITS)?;
        } else {
            let symbol_index = function_symbols
                .index(&term.get_head_symbol())
                .expect("The symbol must already be written");
            stream.write_bits(PacketType::ATerm as u64, PACKET_BITS)?;
            stream.write_bits(*symbol_index as u64, function_symbol_index_width)?;
        }
        return stream.write_integer(value);
    }

    let symbol_index = function_symbols
        .index(&term.get_head_symbol())
        .expect("The symbol must already be written");
    let packet_type = if is_output {
        PacketType::ATermOutput
    } else {
        PacketType::ATerm
    };

    stream.write_bits(packet_type as u64, PACKET_BITS)?;
    stream.write_bits(*symbol_index as u64, function_symbol_index_width)?;
    for arg in term.arguments() {
        let index = terms.index(&arg).expect("Argument must already be written");
        stream.write_bits(*index as u64, term_index_width)?;
    }

    Ok(())
}

impl<W: Write> ATermWrite for BinaryATermWriter<W> {
    fn write_aterm(&mut self, term: &ATerm) -> Result<(), MercError> {
        self.write_aterm_impl(term)
    }

    fn write_aterm_int(&mut self, value: usize) -> Result<(), MercError> {
        // The same packet as an output ATermInt, without creating the term.
        self.stream.write_bits(PacketType::ATermIntOutput as u64, PACKET_BITS)?;
        self.stream.write_integer(value as u64)
    }

    fn write_aterm_iter<I>(&mut self, iter: I) -> Result<(), MercError>
    where
        I: ExactSizeIterator<Item = ATerm>,
    {
        self.write_aterm_int(iter.len())?;
        for ldd in iter {
            self.write_aterm(&ldd)?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), MercError> {
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
            fn write_bits(&mut self, value: u64, number_of_bits: u8) -> Result<(), MercError>;
            fn write_string(&mut self, s: &str) -> Result<(), MercError>;
            fn write_integer(&mut self, value: u64) -> Result<(), MercError>;
            fn flush(&mut self) -> Result<(), MercError>;
        }
    }
}

impl<W: Write> Drop for BinaryATermWriter<W> {
    fn drop(&mut self) {
        // A caller that is already propagating a write error may drop the
        // writer without having flushed.
        if !self.flushed && ATermWrite::flush(self).is_err() {
            error!("Error flushing the stream when dropped!");
        }

        self.function_symbols.write().clear();
        self.terms.write().clear();

        // Perform garbage collection after clearing the terms, since they might become unreachable.
        THREAD_TERM_POOL.with(|tp| tp.collect_garbage());
    }
}

/// Reads ATerms from a binary aterm input stream written by `BinaryATermWriter`.
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
    pub fn new(reader: R) -> Result<Self, MercError> {
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
        let end_of_stream_symbol = Symbol::new("end_of_stream".to_string(), 0);
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

/// A top-level item of a binary aterm stream.
enum Output {
    Term(ATerm),
    Int(usize),
    End,
}

impl<R: Read> BinaryATermReader<R> {
    /// Reads packets until the next output term, output integer or the end of the stream.
    fn read_output(&mut self) -> Result<Output, MercError> {
        if self.ended {
            return Err(Error::new(ErrorKind::UnexpectedEof, "Attempted to read after end of stream").into());
        }

        loop {
            let header = self.stream.read_bits(PACKET_BITS)?;
            let packet = PacketType::from(header as u8);
            debug_trace!("Read packet: {:?}", packet);

            match packet {
                PacketType::FunctionSymbol => {
                    let name = self.stream.read_string()?;
                    let arity = self.stream.read_integer()? as usize;
                    let symbol = resolve_read_symbol(&name, arity);
                    debug_trace!("Read symbol {symbol}");

                    let mut write_symbols = self.function_symbols.write();
                    // Safety: s is pushed into the container on the next line.
                    let s = unsafe { write_symbols.protect_symbol(&symbol) };
                    write_symbols.push(s);
                    self.function_symbol_index_width = bits_for_value(write_symbols.len());
                }
                PacketType::ATermIntOutput => {
                    let value = self.stream.read_integer()?.try_into()?;
                    debug_trace!("Output int: {value}");
                    return Ok(Output::Int(value));
                }
                PacketType::ATerm | PacketType::ATermOutput => {
                    let symbol_index = self.stream.read_bits(self.function_symbol_index_width())? as usize;
                    if symbol_index == 0 {
                        // End of stream marker
                        debug_trace!("End of stream marker reached");
                        self.ended = true;
                        return Ok(Output::End);
                    }

                    let symbols = self.function_symbols.read();
                    let symbol = symbols.get(symbol_index).ok_or_else(|| {
                        format!(
                            "Read invalid function symbol index {symbol_index}, length {}",
                            symbols.len()
                        )
                    })?;

                    if is_int_symbol(symbol) {
                        let value = self.stream.read_integer()?.try_into()?;
                        let term = ATermInt::new(value);
                        debug_trace!("Read int term: {term}");

                        let mut write_terms = self.terms.write();
                        // Safety: t is pushed into the container on the next line.
                        let t = unsafe { write_terms.protect(&term) };
                        write_terms.push(t);
                        self.term_index_width = bits_for_value(write_terms.len());
                    } else {
                        // When the arity is zero, no bits are read for the arguments.
                        let num_of_bits = if symbol.arity() > 0 { self.term_index_width() } else { 0 };
                        let mut write_terms = self.terms.write();

                        let term = ATerm::try_with_iter(
                            symbol,
                            (0..symbol.arity()).map(|_| {
                                let arg_index = self.stream.read_bits(num_of_bits)? as usize;
                                let arg = write_terms.get(arg_index).ok_or_else(|| {
                                    format!("Read invalid aterm index {arg_index}, length {}", write_terms.len())
                                })?;
                                debug_trace!("Read arg: {arg}");
                                Ok(arg)
                            }),
                        )?;

                        if packet == PacketType::ATermOutput {
                            debug_trace!("Output term: {term}");
                            return Ok(Output::Term(term));
                        }
                        debug_trace!("Read term: {term}");

                        // Safety: t is pushed into the container on the next line.
                        let t = unsafe { write_terms.protect(&term) };
                        write_terms.push(t);
                        self.term_index_width = bits_for_value(write_terms.len());
                    }
                }
            }
        }
    }
}

impl<R: Read> ATermRead for BinaryATermReader<R> {
    fn read_aterm(&mut self) -> Result<Option<ATerm>, MercError> {
        Ok(match self.read_output()? {
            Output::Term(term) => Some(term),
            Output::Int(value) => Some(ATermInt::new(value).into()),
            Output::End => None,
        })
    }

    fn read_aterm_int(&mut self) -> Result<usize, MercError> {
        match self.read_output()? {
            Output::Int(value) => Ok(value),
            Output::Term(term) if is_int_term(&term) => Ok(ATermInt::from(term).value()),
            Output::Term(term) => Err(format!("Expected an integer, found term {term}").into()),
            Output::End => Err("Expected an integer, found the end of the stream".into()),
        }
    }

    fn read_aterm_iter(&mut self) -> Result<impl ExactSizeIterator<Item = Result<ATerm, MercError>>, MercError> {
        if self.ended {
            return Err(Error::new(
                ErrorKind::UnexpectedEof,
                "Attempted to read_aterm_iter() after end of stream",
            )
            .into());
        }

        let count_term = self.read_aterm()?.ok_or("Missing number of elements for iterator")?;
        if !is_int_term(&count_term) {
            return Err(Error::new(
                ErrorKind::InvalidData,
                format!("Expected an integer element count for the iterator, found term {count_term}"),
            )
            .into());
        }

        let number_of_elements: ATermInt = count_term.into();
        Ok(ATermReadIter {
            reader: self,
            remaining: number_of_elements.value(),
        })
    }
}

impl<R: Read> Drop for BinaryATermReader<R> {
    fn drop(&mut self) {
        self.function_symbols.write().clear();
        self.terms.write().clear();

        // Perform garbage collection after clearing the terms, since they might become unreachable.
        THREAD_TERM_POOL.with(|tp| tp.collect_garbage());
    }
}

impl<R: Read> BitStreamRead for BinaryATermReader<R> {
    delegate::delegate! {
        to self.stream {
            fn read_bits(&mut self, number_of_bits: u8) -> Result<u64, MercError>;
            fn read_string(&mut self) -> Result<String, MercError>;
            fn read_integer(&mut self) -> Result<u64, MercError>;
        }
    }
}

/// A read iterator for ATerms from a binary aterm input stream.
pub(crate) struct ATermReadIter<'a, R: Read> {
    reader: &'a mut BinaryATermReader<R>,
    remaining: usize,
}

impl<'a, R: Read> Iterator for ATermReadIter<'a, R> {
    type Item = Result<ATerm, MercError>;

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
    use merc_utilities::random_test;

    use crate::ATerm;
    use crate::ATermInt;
    use crate::ATermList;
    use crate::ATermRead;
    use crate::ATermWrite;
    use crate::BinaryATermReader;
    use crate::BinaryATermWriter;
    use crate::Symbol;
    use crate::Term;
    use crate::is_int_term;
    use crate::is_list_term;
    use crate::random_term;

    /// A sink that accepts a fixed number of bytes and then fails, mimicking a
    /// disk that fills up (or a pipe that breaks) in the middle of writing.
    struct FailAfter {
        remaining: usize,
    }

    impl std::io::Write for FailAfter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if buf.len() <= self.remaining {
                self.remaining -= buf.len();
                Ok(buf.len())
            } else {
                Err(std::io::Error::other("sink full"))
            }
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn test_read_aterm_iter_invalid_count_is_error() {
        // A valid stream whose first output term is not an ATermInt. This is what
        // read_aterm_iter() encounters when given a wrong or corrupt (but
        // well-formed) file, so it must report an error instead of panicking.
        let mut stream: Vec<u8> = Vec::new();
        let mut output_stream = BinaryATermWriter::new(&mut stream).unwrap();
        output_stream
            .write_aterm(&ATerm::from_string("f(a,b)").unwrap())
            .unwrap();
        ATermWrite::flush(&mut output_stream).unwrap();
        drop(output_stream);

        let mut input_stream = BinaryATermReader::new(&stream[..]).unwrap();
        assert!(
            input_stream.read_aterm_iter().is_err(),
            "a non-integer element count must be reported as a stream error"
        );
    }

    #[test]
    fn test_aterm_int_interchangeable_with_int_terms() {
        // An integer written with write_aterm_int must read back as an ATermInt and vice versa,
        // since mCRL2 writes state numbers as terms.
        let mut stream: Vec<u8> = Vec::new();
        let mut output_stream = BinaryATermWriter::new(&mut stream).unwrap();
        output_stream.write_aterm_int(7).unwrap();
        output_stream.write_aterm(&ATermInt::new(8)).unwrap();
        output_stream.write_aterm(&ATerm::from_string("f(a)").unwrap()).unwrap();
        ATermWrite::flush(&mut output_stream).unwrap();
        drop(output_stream);

        let mut input_stream = BinaryATermReader::new(&stream[..]).unwrap();
        assert_eq!(input_stream.read_aterm().unwrap(), Some(ATermInt::new(7).into()));
        assert_eq!(input_stream.read_aterm_int().unwrap(), 8);
        assert!(
            input_stream.read_aterm_int().is_err(),
            "a non-integer term must be reported as an error"
        );
        assert!(
            input_stream.read_aterm_int().is_err(),
            "the end of the stream must be reported as an error"
        );
    }

    #[test]
    fn test_binary_stream_roundtrips_int_and_list_subterms() {
        let f = Symbol::new("f_roundtrip_test", 2);
        let int_term: ATerm = ATermInt::new(42).into();
        let list_term: ATerm =
            ATermList::from_double_iter(vec![ATermInt::new(1), ATermInt::new(2)].into_iter()).protect();
        let term = ATerm::with_args(&f, &[int_term, list_term]).protect();

        let mut stream: Vec<u8> = Vec::new();
        let mut output_stream = BinaryATermWriter::new(&mut stream).unwrap();
        output_stream.write_aterm(&term).unwrap();
        ATermWrite::flush(&mut output_stream).unwrap();
        drop(output_stream);

        let mut input_stream = BinaryATermReader::new(&stream[..]).unwrap();
        let read_term = input_stream.read_aterm().unwrap().expect("term must be present");

        assert_eq!(
            read_term, term,
            "int/list subterms must round-trip through the binary stream"
        );
        assert!(
            is_int_term(&read_term.arg(0)),
            "the read int subterm must still be recognized as an integer term"
        );
        assert!(
            is_list_term(&read_term.arg(1)),
            "the read list subterm must still be recognized as a list term"
        );
    }

    #[test]
    fn test_writer_drop_after_io_error_does_not_panic() {
        // Allow the 5-byte BAF header, then fail every write.
        let mut writer = BinaryATermWriter::new(FailAfter { remaining: 5 }).unwrap();

        let result = writer.write_aterm(&ATerm::from_string("f(a,b)").unwrap());
        assert!(result.is_err(), "the sink rejects everything after the header");

        // The caller is already handling the write error; dropping the writer must
        // not panic on the (equally failing) implicit flush. A panic here would
        // abort the process when the drop happens during unwinding.
        drop(writer);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Miri is too slow
    fn test_random_binary_stream() {
        random_test(100, |rng| {
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
    #[cfg_attr(miri, ignore)] // Miri is too slow
    fn test_random_binary_stream_iter() {
        random_test(100, |rng| {
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
