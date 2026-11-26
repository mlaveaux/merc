use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;
use streaming_iterator::StreamingIterator;

/// A lending iterator over the lines of a type implementing Read.
///
/// # Errors
///
/// Stops on the first I/O error rather than panicking; the error is then
/// available from `error`.
pub struct LineIterator<T: Read> {
    reader: BufReader<T>,
    buffer: String,
    error: Option<io::Error>,
    end: bool,
}

impl<T: Read> LineIterator<T> {
    pub fn new(reader: T) -> LineIterator<T> {
        LineIterator {
            reader: BufReader::new(reader),
            buffer: String::new(),
            error: None,
            end: false,
        }
    }

    /// Returns the I/O error that terminated iteration, if any.
    pub fn error(&self) -> Option<&io::Error> {
        self.error.as_ref()
    }
}

impl<T: Read> StreamingIterator for LineIterator<T> {
    type Item = String;

    fn advance(&mut self) {
        self.buffer.clear();
        match self.reader.read_line(&mut self.buffer) {
            Ok(n) if n > 0 => {
                if self.buffer.ends_with('\n') {
                    self.buffer.pop();
                    if self.buffer.ends_with('\r') {
                        self.buffer.pop();
                    }
                }
            }
            Ok(_) => self.end = true,
            Err(e) => {
                self.error = Some(e);
                self.end = true;
            }
        }
    }

    fn get(&self) -> Option<&Self::Item> {
        if self.end { None } else { Some(&self.buffer) }
    }
}
