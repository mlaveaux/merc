use std::io;
use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;

use env_logger::Target;
use env_logger::WriteStyle;
use log::Level;
use log::LevelFilter;
use log::Log;
use log::Record;
use log::kv::Value;

use merc_tools::format_key_values_json;

/// A writer that appends to a shared buffer, so the test can read what the logger wrote.
#[derive(Clone, Default)]
struct SharedBuffer(Arc<Mutex<Vec<u8>>>);

impl Write for SharedBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Logs one record with the given key-values through a logger that formats only the key-values,
/// and returns what it wrote.
fn render<const N: usize>(key_values: [(&str, Value<'_>); N]) -> String {
    let buffer = SharedBuffer::default();
    let logger = env_logger::Builder::new()
        .filter_level(LevelFilter::Trace)
        .write_style(WriteStyle::Never)
        .target(Target::Pipe(Box::new(buffer.clone())))
        .format(|formatter, record| format_key_values_json(formatter, record.key_values()))
        .build();

    logger.log(
        &Record::builder()
            .level(Level::Info)
            .args(format_args!("message"))
            .key_values(&key_values)
            .build(),
    );
    logger.flush();

    String::from_utf8(buffer.0.lock().unwrap().clone()).unwrap()
}

#[test]
fn test_no_key_values_writes_nothing() {
    assert_eq!(render([]), "");
}

#[test]
fn test_separator_between_pairs() {
    assert_eq!(
        render([("a", Value::from(1)), ("b", Value::from(2)), ("c", Value::from(3))]),
        "\n{ \"a\": 1, \"b\": 2, \"c\": 3 }"
    );
}

#[test]
fn test_numbers_and_bools_are_bare() {
    assert_eq!(
        render([
            ("x", Value::from(true)),
            ("y", Value::from(-7i64)),
            ("z", Value::from(u64::MAX)),
            ("f", Value::from(1.5f64)),
        ]),
        format!("\n{{ \"x\": true, \"y\": -7, \"z\": {}, \"f\": 1.5 }}", u64::MAX)
    );
}

#[test]
fn test_strings_and_non_finite_numbers_are_quoted() {
    assert_eq!(
        render([("path", Value::from("c:\\a\"b")), ("nan", Value::from(f64::NAN))]),
        "\n{ \"path\": \"c:\\\\a\\\"b\", \"nan\": \"NaN\" }"
    );
}

#[test]
fn test_control_characters_escaped() {
    assert_eq!(
        render([("k\"ey", Value::from("a\nb\t\r\u{1}"))]),
        "\n{ \"k\\\"ey\": \"a\\nb\\t\\r\\u0001\" }"
    );
}
