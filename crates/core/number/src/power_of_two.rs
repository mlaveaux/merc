//! Utilities for working with numbers, particularly powers of two.

/// Returns true when the given value is a power of two.
///
/// A number is a power of two when exactly a single bit is one.
pub fn is_power_of_two<T>(value: T) -> bool
where
    T: num::PrimInt + num::Unsigned,
{
    !value.is_zero() && (value & (value - T::one())).is_zero()
}

/// Returns the smallest power of two that is larger than or equal to the given
/// value.
///
/// # Panics
///
/// This function will panic if the value is larger than the largest power of
/// two that can be represented by the type `T`.
///
/// # Examples
/// ```
/// use merc_number::round_up_to_power_of_two;
///
/// assert_eq!(round_up_to_power_of_two(3u32), 4);
/// assert_eq!(round_up_to_power_of_two(4u32), 4);
/// assert_eq!(round_up_to_power_of_two(5u32), 8);
/// ```
pub fn round_up_to_power_of_two<T>(value: T) -> T
where
    T: num::PrimInt + num::Unsigned,
{
    if value.is_zero() {
        return T::one();
    }

    if is_power_of_two(value) {
        return value;
    }

    let bits = std::mem::size_of::<T>() * 8;
    let shift = bits - value.leading_zeros() as usize;

    // When `shift == bits` the ceiling is `2^bits`, which is not representable
    // in `T`.
    assert!(
        shift < bits,
        "round_up_to_power_of_two: value has no representable power-of-two ceiling",
    );

    T::one() << shift
}

#[cfg(kani)]
mod verification {
    use super::is_power_of_two;
    use super::round_up_to_power_of_two;

    /// `is_power_of_two` holds exactly when a single bit is set, for every
    /// `u16`. Kani also checks the implementation is free of overflow.
    #[kani::proof]
    fn is_power_of_two_matches_count_ones() {
        let value: u16 = kani::any();
        assert_eq!(is_power_of_two(value), value.count_ones() == 1);
    }

    /// For every representable input, `round_up_to_power_of_two` returns the
    /// smallest power of two that is `>=` the input (and never overflows).
    #[kani::proof]
    fn round_up_is_smallest_power_of_two() {
        let value: u16 = kani::any();
        // Restrict to inputs whose ceiling is representable in u16.
        kani::assume(value <= 1u16 << 15);

        let result = round_up_to_power_of_two(value);

        assert!(is_power_of_two(result));
        assert!(result >= value.max(1));

        // Halving drops strictly below the input, so no smaller power qualifies.
        if result > 1 {
            assert!(result / 2 < value.max(1));
        }
    }
}
