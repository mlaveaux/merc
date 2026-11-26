use num::PrimInt;
use num::Unsigned;

/// The maximum number of bytes needed to encode a value of type T in most
/// significant bit encoding.
///
/// The encoding stores 7 payload bits per byte, so a `T` of `n` bits needs
/// `ceil(n / 7)` bytes. `((size_of::<T>() + 1) * 8) / 7` computes that ceiling
/// (e.g. 10 bytes for `u64`).
pub const fn encoding_size<T>() -> usize {
    ((std::mem::size_of::<T>() + 1) * 8) / 7
}

/// Encodes a given unsigned variable-length integer using the most significant bit (MSB) algorithm,
/// passing every byte to `write_byte`.
///
/// # Details
///
/// Every byte stores seven bits of the value, least significant first, and has its most
/// significant bit set when more bytes follow. Implementation taken from
/// <https://techoverflow.net/2013/01/25/efficiently-encoding-variable-length-integers-in-cc/>
pub fn write_variablelength<T, E>(mut value: T, mut write_byte: impl FnMut(u8) -> Result<(), E>) -> Result<(), E>
where
    T: PrimInt + Unsigned + From<u8>,
{
    let payload = <T as From<u8>>::from(0b01111111);

    // While more than 7 bits of data are left, occupy the last output byte
    // and set the next byte flag.
    while value > payload {
        write_byte(low_byte(value & payload) | 0b10000000)?;

        // Remove the seven bits we just wrote from value.
        value = value >> 7;
    }

    write_byte(low_byte(value))
}

/// Decodes an unsigned variable-length integer using the MSB algorithm, taking every byte from
/// `read_byte`.
pub fn read_variablelength<T, E>(mut read_byte: impl FnMut() -> Result<u8, E>) -> Result<T, E>
where
    T: PrimInt + Unsigned + From<u8>,
{
    let mut value = T::zero();
    for i in 0..encoding_size::<T>() {
        let byte = read_byte()?;

        // Take 7 bits (mask 0b01111111) from byte and shift it before the bits already written to value.
        value = value | (<T as From<u8>>::from(byte & 0b01111111) << (7 * i));

        if byte & 0b10000000 == 0 {
            // If the next-byte flag is not set then we are finished.
            break;
        }
    }

    Ok(value)
}

/// Returns `value`, which must be less than 256, as a byte.
fn low_byte<T: PrimInt>(value: T) -> u8 {
    value.to_u8().expect("The value fits in a byte")
}
