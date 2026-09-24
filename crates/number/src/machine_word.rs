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

#[cfg(test)]
mod tests {
    use rand::RngExt;

    use merc_utilities::random_test;

    use super::*;

    const MAX: u64 = u64::MAX;

    #[test]
    fn test_constants_and_predicates() {
        assert_eq!(
            (zero_word(), one_word(), two_word(), three_word(), four_word()),
            (0, 1, 2, 3, 4)
        );
        assert_eq!(max_word(), MAX);
        assert!(equals_zero_word(0) && !equals_zero_word(1));
        assert!(not_equals_zero_word(1) && !not_equals_zero_word(0));
        assert!(equals_one_word(1) && !equals_one_word(2));
        assert!(equals_max_word(MAX) && !equals_max_word(0));
        assert!(equal_word(7, 7) && not_equal_word(7, 8));
        assert!(less_word(1, 2) && less_equal_word(2, 2));
        assert!(greater_word(2, 1) && greater_equal_word(2, 2));
        assert!(rightmost_bit(3) && !rightmost_bit(4));
    }

    #[test]
    fn test_add_and_overflow_wraps() {
        assert_eq!(add_word(MAX, 1), 0);
        assert_eq!(add_with_carry_word(MAX, 0), 0);
        assert!(add_overflow_word(MAX, 1) && !add_overflow_word(1, 1));
        assert!(add_with_carry_overflow_word(MAX, 0) && !add_with_carry_overflow_word(1, 1));
        assert_eq!(succ_word(MAX), 0);
        assert_eq!(pred_word(0), MAX);
        assert_eq!(minus_word(0, 1), MAX);
        assert_eq!(monus_word(0, 1), 0);
        assert_eq!(monus_word(5, 2), 3);
    }

    #[test]
    fn test_times_and_overflow_match_u128() {
        for &(a, b, c) in &[(MAX, MAX, MAX), (3, 5, 7), (1u64 << 40, 1u64 << 40, 0)] {
            let full = a as u128 * b as u128;
            assert_eq!(times_word(a, b), full as u64);
            assert_eq!(times_overflow_word(a, b), (full >> 64) as u64);
            let full_c = a as u128 * b as u128 + c as u128;
            assert_eq!(times_with_carry_word(a, b, c), full_c as u64);
            assert_eq!(times_with_carry_overflow_word(a, b, c), (full_c >> 64) as u64);
        }
    }

    #[test]
    fn test_div_mod_and_shift() {
        assert_eq!(div_word(17, 5), 3);
        assert_eq!(mod_word(17, 5), 2);
        // (2^64 * 1 + 0) div 2 == 2^63.
        assert_eq!(div_doubleword(1, 0, 2), 1u64 << 63);
        assert_eq!(mod_doubleword(1, 1, 2), 1);
        assert_eq!(div_double_doubleword(1, 0, 0, 2), 1u64 << 63);
        assert_eq!(mod_double_doubleword(1, 0, 0, 2), 0);
        assert_eq!(shift_right(false, 0b100), 0b10);
        assert_eq!(shift_right(true, 0), 1u64 << 63);
    }

    #[test]
    fn test_triple_doubleword_div_matches_biguint() {
        let (n1, n2, n3, n4, n5) = (7u64, 11, 13, 17, 19);
        let numerator = big_from_digits(&[n1, n2, n3]);
        let denominator = big_from_digits(&[n4, n5]);
        let expected = truncate_u64(&(numerator / denominator));
        assert_eq!(div_triple_doubleword(n1, n2, n3, n4, n5), expected);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // miri is too slow to run this test
    fn test_square_roots() {
        assert_eq!(sqrt_word(0), 0);
        assert_eq!(sqrt_word(15), 3);
        assert_eq!(sqrt_word(16), 4);
        assert_eq!(sqrt_word(MAX), MAX.sqrt());
        // sqrt(2^64) == 2^32 exactly.
        assert_eq!(sqrt_doubleword(1, 0), 1u64 << 32);
        // Cross-check the wide roots against BigUint for a full-width value.
        let value = big_from_digits(&[MAX, MAX, MAX, MAX]);
        let root = value.sqrt();
        assert_eq!(sqrt_quadrupleword(MAX, MAX, MAX, MAX), truncate_u64(&root));
        assert_eq!(
            sqrt_quadrupleword_overflow(MAX, MAX, MAX, MAX),
            truncate_u64(&(root >> 64))
        );
        let triple = big_from_digits(&[MAX, MAX, MAX]);
        let triple_root = triple.sqrt();
        assert_eq!(sqrt_tripleword(MAX, MAX, MAX), truncate_u64(&triple_root));
        assert_eq!(
            sqrt_tripleword_overflow(MAX, MAX, MAX),
            truncate_u64(&(triple_root >> 64))
        );
    }

    // Randomised cross-checks against the binary number encoding
    //
    // Every machine-word operation is checked against the same computation on
    // [`num::BigUint`], an arbitrary-precision *binary* big-integer.

    /// The single machine word `value` as a binary big-integer.
    fn big(value: u64) -> BigUint {
        BigUint::from(value)
    }

    /// One as a binary big-integer.
    fn one() -> BigUint {
        BigUint::from(1u64)
    }

    /// The digit base `2^64` as a binary big-integer.
    fn base() -> BigUint {
        one() << WORD_BITS
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_arithmetic_matches_binary_encoding() {
        random_test(10_000, |rng| {
            let a: u64 = rng.random();
            let b: u64 = rng.random();
            let c: u64 = rng.random();
            let (ba, bb, bc, base) = (big(a), big(b), big(c), base());

            // Addition: the low word plus the carry-out reconstruct the exact sum.
            let sum = &ba + &bb;
            assert_eq!(big(add_word(a, b)), &sum % &base);
            assert_eq!(add_overflow_word(a, b), sum >= base);
            let sum_carry = &ba + &bb + &one();
            assert_eq!(big(add_with_carry_word(a, b)), &sum_carry % &base);
            assert_eq!(add_with_carry_overflow_word(a, b), sum_carry >= base);

            // Subtraction wraps modulo 2^64; monus saturates at zero.
            let wrapped_diff = &ba + &base - &bb;
            assert_eq!(big(minus_word(a, b)), &wrapped_diff % &base);
            assert_eq!(big(monus_word(a, b)), if ba >= bb { &ba - &bb } else { BigUint::ZERO });

            // Multiplication: the overflow word holds the high 64 bits, so together
            // with the low word they reconstruct the full 128-bit product.
            let product = &ba * &bb;
            assert_eq!(&big(times_overflow_word(a, b)) * &base + big(times_word(a, b)), product);
            // times_with_carry adds a third word before splitting into hi/lo.
            let fused = &ba * &bb + &bc;
            assert_eq!(
                &big(times_with_carry_overflow_word(a, b, c)) * &base + big(times_with_carry_word(a, b, c)),
                fused
            );

            // Successor / predecessor wrap.
            assert_eq!(big(succ_word(a)), &(&ba + &one()) % &base);
            assert_eq!(big(pred_word(a)), &(&ba + &base - &one()) % &base);

            // Comparisons agree with the arbitrary-precision ordering.
            assert_eq!(equal_word(a, b), ba == bb);
            assert_eq!(not_equal_word(a, b), ba != bb);
            assert_eq!(less_word(a, b), ba < bb);
            assert_eq!(less_equal_word(a, b), ba <= bb);
            assert_eq!(greater_word(a, b), ba > bb);
            assert_eq!(greater_equal_word(a, b), ba >= bb);

            // shift_right divides by two, inserting `bit` as the new top bit.
            let bit: bool = rng.random();
            let inserted = if bit { &base >> 1u32 } else { BigUint::ZERO };
            assert_eq!(big(shift_right(bit, a)), (&ba >> 1u32) + inserted);
            assert_eq!(rightmost_bit(a), &ba % 2u32 == one());
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_div_mod_identities() {
        random_test(10_000, |rng| {
            let a: u64 = rng.random();
            let base = base();

            // Single-word Euclidean division: a == q*divisor + r with 0 <= r < divisor.
            let divisor = rng.random::<u64>() | 1; // odd, hence non-zero
            let q = div_word(a, divisor);
            let r = mod_word(a, divisor);
            assert_eq!(&big(q) * &big(divisor) + big(r), big(a));
            assert!(r < divisor);

            // Double-word numerator over a single-word divisor. Keep the high word
            // below the divisor so the quotient still fits in one word (the C++
            // contract for div_doubleword).
            let c = rng.random::<u64>() | 1;
            let hi = a % c;
            let lo: u64 = rng.random();
            let numerator = &big(hi) * &base + big(lo);
            let bc = big(c);
            let dq = div_doubleword(hi, lo, c);
            let dr = mod_doubleword(hi, lo, c);
            assert_eq!(big(dq), &numerator / &bc);
            assert_eq!(big(dr), &numerator % &bc);
            assert_eq!(&big(dq) * &bc + big(dr), numerator);

            // Wider divisions truncate their word-valued result to the low 64 bits,
            // exactly like the C++ static_cast; verify against the binary reference
            // with the same truncation.
            let (n1, n2, n3): (u64, u64, u64) = (rng.random(), rng.random(), rng.random());
            let denom_hi: u64 = rng.random();
            let denom_lo = rng.random::<u64>() | 1; // makes the two-word divisor non-zero

            let num_dd = big_from_digits(&[n1, n2]);
            let den_dd = big_from_digits(&[n3, denom_hi.max(1)]);
            assert_eq!(
                div_double_doubleword(n1, n2, n3, denom_hi.max(1)),
                truncate_u64(&(&num_dd / &den_dd))
            );
            assert_eq!(
                mod_double_doubleword(n1, n2, n3, denom_hi.max(1)),
                truncate_u64(&(&num_dd % &den_dd))
            );

            let num_td = big_from_digits(&[n1, n2, n3]);
            let den_td = big_from_digits(&[denom_hi, denom_lo]);
            assert_eq!(
                div_triple_doubleword(n1, n2, n3, denom_hi, denom_lo),
                truncate_u64(&(&num_td / &den_td))
            );
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_square_root_bounds() {
        // For the integer square root `r` of `n`, the defining property is
        // `r*r <= n < (r+1)*(r+1)`.
        let check_bounds = |root: &BigUint, n: &BigUint| {
            let next = root + &one();
            assert!(root * root <= *n, "root too large: {root}^2 > {n}");
            assert!(*n < &next * &next, "root too small: {n} >= {next}^2");
        };

        random_test(5_000, |rng| {
            let base = base();
            let (n1, n2, n3, n4): (u64, u64, u64, u64) = (rng.random(), rng.random(), rng.random(), rng.random());

            // Single word.
            let root = big(sqrt_word(n1));
            let n = big(n1);
            assert_eq!(root, n.sqrt());
            check_bounds(&root, &n);

            // Double word (< 2^128): the root still fits in a single word.
            let n = big_from_digits(&[n1, n2]);
            let root = big(sqrt_doubleword(n1, n2));
            assert_eq!(root, n.sqrt());
            check_bounds(&root, &n);

            // Triple word: the root can exceed a word, split into low and overflow.
            let n = big_from_digits(&[n1, n2, n3]);
            let root = &big(sqrt_tripleword_overflow(n1, n2, n3)) * &base + big(sqrt_tripleword(n1, n2, n3));
            assert_eq!(root, n.sqrt());
            check_bounds(&root, &n);

            // Quadruple word.
            let n = big_from_digits(&[n1, n2, n3, n4]);
            let root =
                &big(sqrt_quadrupleword_overflow(n1, n2, n3, n4)) * &base + big(sqrt_quadrupleword(n1, n2, n3, n4));
            assert_eq!(root, n.sqrt());
            check_bounds(&root, &n);
        });
    }
}
