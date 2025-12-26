use std::io::Read;
use std::io::Write;

use bitstream_io::BitRead;
use bitstream_io::BitReader;
use bitstream_io::BitWrite;
use bitstream_io::BitWriter;
use bitstream_io::Endianness;

use merc_utilities::MercError;

/// The number of bits needed to represent a value of type T in most significant bit encoding.
pub const fn encoding_size<T>() -> usize {
    ((std::mem::size_of::<T>() + 1) * 8) / 7
}

/// Encodes a given unsigned variable-length integer using the most significant bit (MSB) algorithm.
///
/// # Details
///
/// Implementation taken from <https://techoverflow.net/2013/01/25/efficiently-encoding-variable-length-integers-in-cc/>
pub fn write_u64_variablelength<W: Write, E: Endianness>(
    stream: &mut BitWriter<W, E>,
    mut value: u64,
) -> Result<(), MercError> {
    // While more than 7 bits of data are left, occupy the last output byte
    // and set the next byte flag.
    while value > 0b01111111 {
        stream.write::<8, u8>((value as u8 & 0b01111111) | 0b10000000)?;

        // Remove the seven bits we just wrote from value.
        value >>= 7;
    }

    stream.write::<8, u8>(value as u8)?;
    Ok(())
}

/// Decodes an unsigned variable-length integer using the MSB algorithm.
pub fn read_u64_variablelength<R: Read, E: Endianness>(stream: &mut BitReader<R, E>) -> Result<u64, MercError> {
    let mut value: u64 = 0;
    for i in 0..encoding_size::<u64>() {
        let byte = stream.read::<8, u8>()?;

        // Take 7 bits (mask 0x01111111) from byte and shift it before the bits already written to value.
        value |= ((byte & 0b01111111) as u64) << (7 * i);

        if byte & 0b10000000 == 0 {
            // If the next-byte flag is not set then we are finished.
            break;
        }
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    use bitstream_io::BigEndian;
    use rand::Rng;

    use merc_utilities::random_test;

    #[test]
    fn test_random_integer_encoding() {
        random_test(1000, |rng| {
            let value = rng.random();

            let mut stream: [u8; encoding_size::<u64>()] = [0; encoding_size::<u64>()];
            let mut writer = BitWriter::<_, BigEndian>::new(&mut stream[0..]);
            write_u64_variablelength(&mut writer, value).unwrap();

            let mut reader = BitReader::<_, BigEndian>::new(&stream[0..]);
            let result = read_u64_variablelength(&mut reader).unwrap();

            assert_eq!(result, value);
        });
    }
}
