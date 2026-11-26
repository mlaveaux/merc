/// Returns the number of bits needed to represent the given value.
pub fn bits_for_value(value: usize) -> u8 {
    if value == 0 {
        return 1;
    }

    value.ilog2() as u8 + 1
}

#[cfg(kani)]
mod verification {
    use super::bits_for_value;

    /// `bits_for_value` returns the minimal bit width: `2^(bits-1) <= value`
    /// and `value < 2^bits` (the upper bound where it fits), with zero mapping
    /// to a single bit. Verified for every `usize`.
    #[kani::proof]
    fn bits_for_value_is_minimal_width() {
        let value: usize = kani::any();
        let bits = bits_for_value(value);

        assert!(bits >= 1);

        if value == 0 {
            assert_eq!(bits, 1);
        } else {
            assert!((1usize << (bits - 1)) <= value);
            if (bits as u32) < usize::BITS {
                assert!(value < (1usize << bits));
            }
        }
    }
}
