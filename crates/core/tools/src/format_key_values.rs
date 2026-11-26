use std::io;
use std::io::Write;

use env_logger::fmt::Formatter;
use log::kv::Key;
use log::kv::Source;
use log::kv::Value;
use log::kv::VisitSource;

/// Formats key-value pairs from the given source as a JSON object.
///
/// # Details
///
/// Can be used to configure `env_logger` to format log key-values as JSON.
///
/// ```rust
/// use merc_tools::format_key_values_json;
///
/// env_logger::Builder::new()
///     .format_key_values(|formatter, source| {
///         Ok(format_key_values_json(formatter, source)?)
///     })
///     .init();
///
/// ```
pub fn format_key_values_json(formatter: &mut Formatter, source: &dyn Source) -> io::Result<()> {
    if source.count() > 0 {
        // If there are key-values, format them as a JSON object.
        formatter.write_all(b"\n{ ")?;
        let mut json_printer = JsonPrinter { formatter, first: true };
        if let Err(e) = source.visit(&mut json_printer) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, e));
        }
        formatter.write_all(b" }")?;
    }

    Ok(())
}

/// A visitor that formats key-value pairs as JSON on a single line.
struct JsonPrinter<'a> {
    formatter: &'a mut Formatter,
    /// Whether the next pair is the first; used to place comma separators.
    first: bool,
}

impl<'a, 'kvs> VisitSource<'kvs> for JsonPrinter<'a> {
    fn visit_pair(&mut self, key: Key<'kvs>, value: Value<'kvs>) -> Result<(), log::kv::Error> {
        write_json_pair(self.formatter, &mut self.first, key, value)?;
        Ok(())
    }
}

/// Writes a single `"key": value` pair as JSON, prefixing a `, ` separator for
/// every pair after the first (tracked through `first`).
///
/// Booleans and finite numbers are emitted as bare JSON tokens; everything else
/// (including strings, which `Value`'s `Display` renders unquoted) is written as
/// a quoted, escaped JSON string so the result is always valid JSON.
fn write_json_pair<W: Write>(writer: &mut W, first: &mut bool, key: Key<'_>, value: Value<'_>) -> io::Result<()> {
    if *first {
        *first = false;
    } else {
        writer.write_all(b", ")?;
    }

    write_json_string(writer, key.as_str())?;
    writer.write_all(b": ")?;

    if let Some(b) = value.to_bool() {
        write!(writer, "{b}")
    } else if let Some(i) = value.to_i64() {
        write!(writer, "{i}")
    } else if let Some(u) = value.to_u64() {
        write!(writer, "{u}")
    } else if let Some(f) = value.to_f64().filter(|f| f.is_finite()) {
        write!(writer, "{f}")
    } else {
        write_json_string(writer, &value.to_string())
    }
}

/// Writes `s` as a double-quoted, escaped JSON string.
fn write_json_string<W: Write>(writer: &mut W, s: &str) -> io::Result<()> {
    writer.write_all(b"\"")?;
    for c in s.chars() {
        match c {
            '"' => writer.write_all(b"\\\"")?,
            '\\' => writer.write_all(b"\\\\")?,
            '\n' => writer.write_all(b"\\n")?,
            '\r' => writer.write_all(b"\\r")?,
            '\t' => writer.write_all(b"\\t")?,
            c if (c as u32) < 0x20 => write!(writer, "\\u{:04x}", c as u32)?,
            c => write!(writer, "{c}")?,
        }
    }
    writer.write_all(b"\"")
}
