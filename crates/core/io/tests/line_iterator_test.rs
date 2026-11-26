use std::io;
use std::io::Cursor;
use std::io::Read;

use streaming_iterator::StreamingIterator;

use merc_io::LineIterator;

/// Collects all lines of the given iterator.
fn collect_lines<T: Read>(mut iterator: LineIterator<T>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Some(line) = iterator.next() {
        lines.push(line.clone());
    }
    assert!(iterator.error().is_none(), "reading from memory cannot fail");
    lines
}

#[test]
fn test_line_iterator() {
    let cases: [(&str, &[&str]); 7] = [
        ("", &[]),
        ("single line", &["single line"]),
        ("line1\nline2\nline3", &["line1", "line2", "line3"]),
        // A trailing newline does not start another line.
        ("line1\nline2\n", &["line1", "line2"]),
        // Empty lines in between are kept.
        ("a\n\nb", &["a", "", "b"]),
        ("line1\r\nline2\r\nline3", &["line1", "line2", "line3"]),
        // A carriage return that does not precede a newline is part of the line.
        ("a\rb\r\n", &["a\rb"]),
    ];

    for (input, expected) in cases {
        assert_eq!(
            collect_lines(LineIterator::new(Cursor::new(input))),
            expected,
            "input {input:?}"
        );
    }
}

#[test]
fn test_line_iterator_stops_on_error() {
    /// Returns one line, after which every read fails.
    struct FailAfter(Option<&'static [u8]>);

    impl Read for FailAfter {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.0.take() {
                Some(data) => {
                    buf[..data.len()].copy_from_slice(data);
                    Ok(data.len())
                }
                None => Err(io::Error::other("broken")),
            }
        }
    }

    let mut iterator = LineIterator::new(FailAfter(Some(b"first\n")));
    assert_eq!(iterator.next().map(String::as_str), Some("first"));
    assert!(iterator.error().is_none());
    assert_eq!(iterator.next(), None);
    assert_eq!(iterator.error().map(io::Error::to_string).as_deref(), Some("broken"));
}
