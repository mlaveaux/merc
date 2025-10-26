/// Returns the number of bits needed to represent the given value.
pub fn bits_for_value(value: usize) -> u8 {
    value.ilog2() as u8 + 1
}
