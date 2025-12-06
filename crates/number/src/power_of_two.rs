//! Utilities for working with numbers, particularly powers of two.

/// Returns true when the given value is a power of two.
///
/// A number is a power of two when exactly a single bit is one.
pub fn is_power_of_two<T>(value: T) -> bool
where
    T: num::PrimInt,
{
    !value.is_zero() && (value & (value - T::one())).is_zero()
}

/// Returns the smallest power of two that is larger than or equal to the given value.
///
/// # Examples
/// ```
/// use merc_number::round_up_to_power_of_two;
///
/// assert_eq!(round_up_to_power_of_two(3u32), 4);
/// assert_eq!(round_up_to_power_of_two(4u32), 4);
/// assert_eq!(round_up_to_power_of_two(5u32), 8);
/// ```
pub fn round_up_to_power_of_two<T>(mut value: T) -> T
where
    T: num::PrimInt,
{
    if is_power_of_two(value) {
        return value;
    }

    if value.is_zero() {
        return T::one();
    }

    // Set all bits to the right of the highest 1-bit
    let bits = std::mem::size_of::<T>() * 8;
    for i in 0..bits {
        value = value | (value >> i);
    }

    // Add one to get the next power of two
    debug_assert!(is_power_of_two(value + T::one()));
    value + T::one()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_power_of_two() {
        // Test powers of 2
        assert!(is_power_of_two(1u32));
        assert!(is_power_of_two(2u32));
        assert!(is_power_of_two(4u32));
        assert!(is_power_of_two(8u32));
        assert!(is_power_of_two(16u32));

        // Test non-powers of 2
        assert!(!is_power_of_two(0u32));
        assert!(!is_power_of_two(3u32));
        assert!(!is_power_of_two(5u32));
        assert!(!is_power_of_two(6u32));
        assert!(!is_power_of_two(7u32));
    }

    #[test]
    fn test_round_up_to_power_of_two() {
        // Test exact powers of 2
        assert_eq!(round_up_to_power_of_two(1u32), 1);
        assert_eq!(round_up_to_power_of_two(2u32), 2);
        assert_eq!(round_up_to_power_of_two(4u32), 4);
        assert_eq!(round_up_to_power_of_two(8u32), 8);

        // Test values in between
        assert_eq!(round_up_to_power_of_two(0u32), 1);
        assert_eq!(round_up_to_power_of_two(3u32), 4);
        assert_eq!(round_up_to_power_of_two(5u32), 8);
        assert_eq!(round_up_to_power_of_two(7u32), 8);
        assert_eq!(round_up_to_power_of_two(9u32), 16);
    }

    #[test]
    fn test_different_types() {
        assert!(is_power_of_two(4u8));
        assert!(is_power_of_two(8u16));
        assert!(is_power_of_two(16u32));
        assert!(is_power_of_two(32u64));
        assert!(is_power_of_two(64usize));

        assert_eq!(round_up_to_power_of_two(3u8), 4);
        assert_eq!(round_up_to_power_of_two(5u16), 8);
        assert_eq!(round_up_to_power_of_two(9u32), 16);
        assert_eq!(round_up_to_power_of_two(17u64), 32);
        assert_eq!(round_up_to_power_of_two(33usize), 64);
    }
}
