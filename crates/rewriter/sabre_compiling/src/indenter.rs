use std::cell::RefCell;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Write;
use std::rc::Rc;
use std::str::from_utf8;

/// A `Write` adapter that prefixes each line with the current indentation level.
///
/// The level increases with `indent()`, which returns a guard that decreases it
/// again when dropped, so nesting follows scope.
#[derive(Debug)]
pub struct IndentFormatter<'a, W: Write> {
    /// Current indentation level (number of `indent_str` repetitions per line).
    level: Rc<RefCell<usize>>,
    /// Underlying writer that indented content is forwarded to.
    writer: &'a mut W,
    /// String repeated `level` times at the start of each line.
    indent_str: String,
    /// Whether the next byte written starts a new line and needs indentation.
    at_line_start: bool,
}

impl<'a, W: Write> IndentFormatter<'a, W> {
    /// Wraps `writer` with zero indentation, using two spaces per level.
    pub fn new(writer: &'a mut W) -> Self {
        Self::with_indent_str(writer, "  ".to_string())
    }

    /// Wraps `writer` with zero indentation, using `indent_str` per level.
    pub fn with_indent_str(writer: &'a mut W, indent_str: String) -> Self {
        Self {
            writer,
            level: Rc::new(RefCell::new(0)),
            indent_str,
            at_line_start: true, // Start at the beginning of a line
        }
    }

    /// Increases the indentation level and returns a guard that will
    /// decrease it when dropped.
    pub fn indent(&self) -> Indent {
        let mut level_ref = self.level.borrow_mut();
        *level_ref += 1;
        drop(level_ref); // Release the borrow before creating the guard

        Indent {
            level: Rc::clone(&self.level),
        }
    }

    /// Returns the current indentation level.
    pub fn level(&self) -> usize {
        *self.level.borrow()
    }
}

impl<W: Write> Write for IndentFormatter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Convert the byte slice to a string slice
        let s = from_utf8(buf).map_err(|_| Error::new(ErrorKind::InvalidData, "Invalid UTF-8"))?;

        let parts = s.split('\n');
        let mut first = true;

        // Handle each split line
        for part in parts {
            // Write the newline at the end of the previous line.
            if !first {
                self.writer.write_all(b"\n")?;
                self.at_line_start = true;
            }

            if !part.is_empty() {
                // Add indentation if we're at the start of a line
                if self.at_line_start {
                    for _ in 0..self.level() {
                        self.writer.write_all(self.indent_str.as_bytes())?;
                    }
                    self.at_line_start = false;
                }
                self.writer.write_all(part.as_bytes())?;
            }

            first = false;
        }

        // Update line start status based on whether the string ends with a newline
        if s.ends_with('\n') {
            self.at_line_start = true;
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

/// Guard returned by `IndentFormatter::indent()` that decreases the
/// indentation level again on drop.
#[derive(Debug)]
pub struct Indent {
    /// Shared indentation level, decremented on drop.
    level: Rc<RefCell<usize>>,
}

impl Drop for Indent {
    fn drop(&mut self) {
        let mut level_ref = self.level.borrow_mut();
        debug_assert!(*level_ref > 0, "Indentation level cannot go below zero");
        *level_ref -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::IndentFormatter;
    use super::Write;

    /// Tests that the indenter correctly handles multi-line strings
    #[test]
    fn test_multiline_string() {
        let mut buffer = Vec::new();
        {
            let mut formatter = IndentFormatter::new(&mut buffer);

            // First level indent
            let _indent1 = formatter.indent();

            // Write a multi-line string at once
            write!(formatter, "First line\nSecond line\nThird line").unwrap();
        }

        let result = String::from_utf8(buffer).unwrap();
        let expected = "  First line\n  Second line\n  Third line";
        assert_eq!(result, expected, "Multiline indentation incorrect");
    }

    /// Tests that the indenter correctly handles line continuation across multiple write calls
    #[test]
    fn test_line_continuation() {
        let mut buffer = Vec::new();
        {
            let mut formatter = IndentFormatter::new(&mut buffer);

            // First level indent
            let _indent1 = formatter.indent();

            // First part of a line - should be indented
            write!(formatter, "Start of line ").unwrap();

            // Continuation of the same line - should not be indented
            write!(formatter, "continued here").unwrap();

            // New line followed by text - only the new line's content should be indented
            write!(formatter, "\nSecond line").unwrap();

            // Another continuation
            write!(formatter, " continued").unwrap();

            // A line ending with newline
            write!(formatter, "\nThird line\n").unwrap();

            // A new line after previous newline - should be indented
            write!(formatter, "Fourth line").unwrap();
        }

        let result = String::from_utf8(buffer).unwrap();
        let expected = "  Start of line continued here\n  Second line continued\n  Third line\n  Fourth line";

        assert_eq!(result, expected, "Line continuation handling incorrect");
    }

    /// Tests that nested guards add up and that dropping a guard restores the outer level
    #[test]
    fn test_dropping_the_guard_restores_the_indentation() {
        let mut buffer = Vec::new();
        {
            let mut formatter = IndentFormatter::new(&mut buffer);
            writeln!(formatter, "outer").unwrap();
            {
                let _outer = formatter.indent();
                writeln!(formatter, "one").unwrap();
                {
                    let _inner = formatter.indent();
                    writeln!(formatter, "two").unwrap();
                }
                writeln!(formatter, "one again").unwrap();
            }
            write!(formatter, "outer again").unwrap();
        }

        let result = String::from_utf8(buffer).unwrap();
        assert_eq!(result, "outer\n  one\n    two\n  one again\nouter again");
    }
}
