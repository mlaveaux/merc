use bnum::Uint;
use bnum::cast::As;
use num::BigUint;
use num::integer::Roots;

/// Number of bits in a machine word; also the shift amount for one digit.
const WORD_BITS: u32 = 64;

/// A stack-allocated 192-bit (three-word) unsigned integer
type TripleWord = Uint<24>;

/// Extracts the least-significant 64 bits of a [`BigUint`].
fn truncate_u64(value: &BigUint) -> u64 {
    value.iter_u64_digits().next().unwrap_or(0)
}

/// Interprets `(hi, lo)` as the 128-bit number `hi * 2^64 + lo`.
fn double_word(hi: u64, lo: u64) -> u128 {
    ((hi as u128) << WORD_BITS) | lo as u128
}

/// Builds `d0 * 2^(64*(n-1)) + ... + d_{n-1}` from most-significant-first
/// digits as a [`BigUint`].
fn big_from_digits(digits: &[u64]) -> BigUint {
    let mut result = BigUint::ZERO;
    for &digit in digits {
        result = (result << WORD_BITS) + digit;
    }
    result
}

/// Builds `d0 * 2^128 + d1 * 2^64 + d2` from most-significant-first digits as
/// a stack-allocated [`TripleWord`].
fn triple_word(d0: u64, d1: u64, d2: u64) -> TripleWord {
    let mut bytes = [0u8; 24];
    bytes[0..8].copy_from_slice(&d0.to_be_bytes());
    bytes[8..16].copy_from_slice(&d1.to_be_bytes());
    bytes[16..24].copy_from_slice(&d2.to_be_bytes());
    TripleWord::from_be_bytes(bytes)
}

// Word constants

pub fn zero_word() -> u64 {
    0
}
pub fn one_word() -> u64 {
    1
}
pub fn two_word() -> u64 {
    2
}
pub fn three_word() -> u64 {
    3
}
pub fn four_word() -> u64 {
    4
}
pub fn max_word() -> u64 {
    u64::MAX
}

// Predicates

pub fn equals_zero_word(n: u64) -> bool {
    n == 0
}
pub fn not_equals_zero_word(n: u64) -> bool {
    n != 0
}
pub fn equals_one_word(n: u64) -> bool {
    n == 1
}
pub fn equals_max_word(n: u64) -> bool {
    n == u64::MAX
}
pub fn equal_word(n1: u64, n2: u64) -> bool {
    n1 == n2
}
pub fn not_equal_word(n1: u64, n2: u64) -> bool {
    n1 != n2
}
pub fn less_word(n1: u64, n2: u64) -> bool {
    n1 < n2
}
pub fn less_equal_word(n1: u64, n2: u64) -> bool {
    n1 <= n2
}
pub fn greater_word(n1: u64, n2: u64) -> bool {
    n1 > n2
}
pub fn greater_equal_word(n1: u64, n2: u64) -> bool {
    n1 >= n2
}

/// True when `n1 + n2` does not fit in a machine word.
pub fn add_overflow_word(n1: u64, n2: u64) -> bool {
    n1.checked_add(n2).is_none()
}

/// True when `n1 + n2 + 1` does not fit in a machine word.
pub fn add_with_carry_overflow_word(n1: u64, n2: u64) -> bool {
    n1.checked_add(n2).and_then(|s| s.checked_add(1)).is_none()
}

/// The least-significant bit of `n`.
pub fn rightmost_bit(n: u64) -> bool {
    (n & 1) == 1
}

// Word-valued arithmetic (wrapping modulo 2^64)

pub fn succ_word(n: u64) -> u64 {
    n.wrapping_add(1)
}
pub fn pred_word(n: u64) -> u64 {
    n.wrapping_sub(1)
}
pub fn add_word(n1: u64, n2: u64) -> u64 {
    n1.wrapping_add(n2)
}
pub fn add_with_carry_word(n1: u64, n2: u64) -> u64 {
    n1.wrapping_add(n2).wrapping_add(1)
}
pub fn times_word(n1: u64, n2: u64) -> u64 {
    n1.wrapping_mul(n2)
}

/// `n1 * n2 + n3` modulo `2^64`.
pub fn times_with_carry_word(n1: u64, n2: u64, n3: u64) -> u64 {
    n1.wrapping_mul(n2).wrapping_add(n3)
}

/// The high 64 bits of `n1 * n2`.
pub fn times_overflow_word(n1: u64, n2: u64) -> u64 {
    ((n1 as u128 * n2 as u128) >> WORD_BITS) as u64
}

/// The high 64 bits of `n1 * n2 + n3`.
pub fn times_with_carry_overflow_word(n1: u64, n2: u64, n3: u64) -> u64 {
    ((n1 as u128 * n2 as u128 + n3 as u128) >> WORD_BITS) as u64
}

/// `n1 - n2` modulo `2^64`.
pub fn minus_word(n1: u64, n2: u64) -> u64 {
    n1.wrapping_sub(n2)
}

/// `max(0, n1 - n2)`.
pub fn monus_word(n1: u64, n2: u64) -> u64 {
    n1.saturating_sub(n2)
}
pub fn div_word(n1: u64, n2: u64) -> u64 {
    n1 / n2
}
pub fn mod_word(n1: u64, n2: u64) -> u64 {
    n1 % n2
}

/// Square root of `n`, rounded down.
pub fn sqrt_word(n: u64) -> u64 {
    n.sqrt()
}

/// The word shifted one position right, with `bit` inserted as the new
/// most-significant bit.
pub fn shift_right(bit: bool, n: u64) -> u64 {
    let shifted = n >> 1;
    if bit {
        shifted | (1u64 << (WORD_BITS - 1))
    } else {
        shifted
    }
}

// Double / triple / quadruple word operations (base = 2^64)

/// `(2^64 * n1 + n2) div n3`.
pub fn div_doubleword(n1: u64, n2: u64, n3: u64) -> u64 {
    (double_word(n1, n2) / n3 as u128) as u64
}

/// `(2^64 * n1 + n2) mod n3`.
pub fn mod_doubleword(n1: u64, n2: u64, n3: u64) -> u64 {
    (double_word(n1, n2) % n3 as u128) as u64
}

/// `(2^64 * n1 + n2) div (2^64 * n3 + n4)`.
pub fn div_double_doubleword(n1: u64, n2: u64, n3: u64, n4: u64) -> u64 {
    (double_word(n1, n2) / double_word(n3, n4)) as u64
}

/// `(2^64 * n1 + n2) mod (2^64 * n3 + n4)`.
pub fn mod_double_doubleword(n1: u64, n2: u64, n3: u64, n4: u64) -> u64 {
    (double_word(n1, n2) % double_word(n3, n4)) as u64
}

/// `(2^128 * n1 + 2^64 * n2 + n3) div (2^64 * n4 + n5)`.
pub fn div_triple_doubleword(n1: u64, n2: u64, n3: u64, n4: u64, n5: u64) -> u64 {
    let numerator = triple_word(n1, n2, n3);
    // Widen the two-word divisor to three words with a leading zero digit.
    let denominator = triple_word(0, n4, n5);
    (numerator / denominator).as_::<u64>()
}

/// Square root of `2^64 * n1 + n2`, rounded down.
pub fn sqrt_doubleword(n1: u64, n2: u64) -> u64 {
    double_word(n1, n2).sqrt() as u64
}

/// Least-significant word of the square root of `2^128 * n1 + 2^64 * n2 + n3`.
pub fn sqrt_tripleword(n1: u64, n2: u64, n3: u64) -> u64 {
    truncate_u64(&big_from_digits(&[n1, n2, n3]).sqrt())
}

/// Most-significant word of the square root of `2^128 * n1 + 2^64 * n2 + n3`.
pub fn sqrt_tripleword_overflow(n1: u64, n2: u64, n3: u64) -> u64 {
    truncate_u64(&(big_from_digits(&[n1, n2, n3]).sqrt() >> WORD_BITS))
}

/// Least-significant word of the square root of
/// `2^192 * n1 + 2^128 * n2 + 2^64 * n3 + n4`.
pub fn sqrt_quadrupleword(n1: u64, n2: u64, n3: u64, n4: u64) -> u64 {
    truncate_u64(&big_from_digits(&[n1, n2, n3, n4]).sqrt())
}

/// Most-significant word of the square root of
/// `2^192 * n1 + 2^128 * n2 + 2^64 * n3 + n4`.
pub fn sqrt_quadrupleword_overflow(n1: u64, n2: u64, n3: u64, n4: u64) -> u64 {
    truncate_u64(&(big_from_digits(&[n1, n2, n3, n4]).sqrt() >> WORD_BITS))
}
