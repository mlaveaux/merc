use merc_io::BitStreamRead;
use merc_io::BitStreamReader;
use merc_io::BitStreamWrite;
use merc_io::BitStreamWriter;

#[test]
fn read_string_on_truncated_stream_errors_instead_of_panicking() {
    // Claim a string of length 1000 but only supply a handful of bytes.
    let mut buffer = Vec::new();
    {
        let mut writer = BitStreamWriter::new(&mut buffer);
        writer.write_integer(1000).unwrap();
        writer.write_bits(0x41, 8).unwrap();
        writer.flush().unwrap();
    }

    let mut reader = BitStreamReader::new(&buffer[..]);
    let result = reader.read_string();
    assert!(result.is_err(), "a truncated string should error, got {result:?}");
}

#[test]
fn read_integer_on_empty_stream_errors_instead_of_panicking() {
    let buffer: Vec<u8> = Vec::new();
    let mut reader = BitStreamReader::new(&buffer[..]);
    let result = reader.read_integer();
    assert!(
        result.is_err(),
        "reading from an empty stream should error, got {result:?}"
    );
}
