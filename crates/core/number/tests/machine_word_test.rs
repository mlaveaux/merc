use num::BigUint;
use rand::RngExt;

use merc_number::machine_word::add_overflow_word;
use merc_number::machine_word::add_with_carry_overflow_word;
use merc_number::machine_word::add_with_carry_word;
use merc_number::machine_word::add_word;
use merc_number::machine_word::div_double_doubleword;
use merc_number::machine_word::div_doubleword;
use merc_number::machine_word::div_triple_doubleword;
use merc_number::machine_word::div_word;
use merc_number::machine_word::equal_word;
use merc_number::machine_word::equals_max_word;
use merc_number::machine_word::equals_one_word;
use merc_number::machine_word::equals_zero_word;
use merc_number::machine_word::four_word;
use merc_number::machine_word::greater_equal_word;
use merc_number::machine_word::greater_word;
use merc_number::machine_word::less_equal_word;
use merc_number::machine_word::less_word;
use merc_number::machine_word::max_word;
use merc_number::machine_word::minus_word;
use merc_number::machine_word::mod_double_doubleword;
use merc_number::machine_word::mod_doubleword;
use merc_number::machine_word::mod_word;
use merc_number::machine_word::monus_word;
use merc_number::machine_word::not_equal_word;
use merc_number::machine_word::not_equals_zero_word;
use merc_number::machine_word::one_word;
use merc_number::machine_word::pred_word;
use merc_number::machine_word::rightmost_bit;
use merc_number::machine_word::shift_right;
use merc_number::machine_word::sqrt_doubleword;
use merc_number::machine_word::sqrt_quadrupleword;
use merc_number::machine_word::sqrt_quadrupleword_overflow;
use merc_number::machine_word::sqrt_tripleword;
use merc_number::machine_word::sqrt_tripleword_overflow;
use merc_number::machine_word::sqrt_word;
use merc_number::machine_word::succ_word;
use merc_number::machine_word::three_word;
use merc_number::machine_word::times_overflow_word;
use merc_number::machine_word::times_with_carry_overflow_word;
use merc_number::machine_word::times_with_carry_word;
use merc_number::machine_word::times_word;
use merc_number::machine_word::two_word;
use merc_number::machine_word::zero_word;
use merc_utilities::random_test;

const MAX: u64 = u64::MAX;

/// Builds `d0 * 2^(64*(n-1)) + ... + d_{n-1}` from most-significant-first digits.
fn big_from_digits(digits: &[u64]) -> BigUint {
    digits
        .iter()
        .fold(BigUint::ZERO, |result, &digit| (result << 64u32) + digit)
}

/// The least-significant 64 bits of `value`.
fn truncate_u64(value: &BigUint) -> u64 {
    value.iter_u64_digits().next().unwrap_or(0)
}

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
    assert!(rightmost_bit(3) && !rightmost_bit(4));
}

#[test]
fn test_add_and_overflow_wraps() {
    assert_eq!(add_word(MAX, 1), 0);
    assert_eq!(add_with_carry_word(MAX, 0), 0);
    assert!(add_overflow_word(MAX, 1) && !add_overflow_word(1, 1));
    assert!(add_with_carry_overflow_word(MAX, 0) && !add_with_carry_overflow_word(MAX - 1, 0));
    assert_eq!(succ_word(MAX), 0);
    assert_eq!(pred_word(0), MAX);
    assert_eq!(minus_word(0, 1), MAX);
    assert_eq!(monus_word(0, 1), 0);
    assert_eq!(monus_word(5, 2), 3);
}

#[test]
fn test_times_and_overflow_edges() {
    // (2^64 - 1)^2 = 2^128 - 2^65 + 1, so the high word is 2^64 - 2 and the low word is 1.
    assert_eq!(times_word(MAX, MAX), 1);
    assert_eq!(times_overflow_word(MAX, MAX), MAX - 1);
    // Adding 2^64 - 1 to that gives 2^128 - 2^64: high word 2^64 - 1, low word 0.
    assert_eq!(times_with_carry_word(MAX, MAX, MAX), 0);
    assert_eq!(times_with_carry_overflow_word(MAX, MAX, MAX), MAX);
    assert_eq!(times_with_carry_word(3, 5, 7), 22);
    assert_eq!(times_with_carry_overflow_word(3, 5, 7), 0);
    assert_eq!(times_word(1 << 40, 1 << 40), 0);
    assert_eq!(times_overflow_word(1 << 40, 1 << 40), 1 << 16);
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
    // 2^128 div 2^65 == 2^63 and 100 div 7 == 14.
    assert_eq!(div_triple_doubleword(1, 0, 0, 2, 0), 1u64 << 63);
    assert_eq!(div_triple_doubleword(0, 0, 100, 0, 7), 14);
    // (2^129 + 4) div 2 == 2^128 + 2, of which the low word 2 is returned.
    assert_eq!(div_triple_doubleword(2, 0, 4, 0, 2), 2);
    assert_eq!(shift_right(false, 0b100), 0b10);
    assert_eq!(shift_right(true, 0), 1u64 << 63);
}

#[test]
fn test_square_roots() {
    assert_eq!(sqrt_word(0), 0);
    assert_eq!(sqrt_word(15), 3);
    assert_eq!(sqrt_word(16), 4);
    assert_eq!(sqrt_word(MAX), (1 << 32) - 1);
    // sqrt(2^64) == 2^32 exactly, and sqrt(2^128 - 1) rounds down to 2^64 - 1.
    assert_eq!(sqrt_doubleword(1, 0), 1u64 << 32);
    assert_eq!(sqrt_doubleword(MAX, MAX), MAX);
    // sqrt(2^192 - 1) rounds down to 2^96 - 1.
    assert_eq!(sqrt_tripleword(MAX, MAX, MAX), MAX);
    assert_eq!(sqrt_tripleword_overflow(MAX, MAX, MAX), (1 << 32) - 1);
    // sqrt(2^256 - 1) rounds down to 2^128 - 1.
    assert_eq!(sqrt_quadrupleword(MAX, MAX, MAX, MAX), MAX);
    assert_eq!(sqrt_quadrupleword_overflow(MAX, MAX, MAX, MAX), MAX);
    // sqrt(2^128) == 2^64 exactly: low word 0, overflow word 1.
    assert_eq!(sqrt_tripleword(1, 0, 0), 0);
    assert_eq!(sqrt_tripleword_overflow(1, 0, 0), 1);
}

// Randomised cross-checks against `num::BigUint`, an arbitrary-precision binary integer.

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_arithmetic_matches_binary_encoding() {
    random_test(10_000, |rng| {
        let a: u64 = rng.random();
        let b: u64 = rng.random();
        let c: u64 = rng.random();
        let (ba, bb, bc) = (BigUint::from(a), BigUint::from(b), BigUint::from(c));
        let base = BigUint::from(1u32) << 64u32;

        // Addition: the low word plus the carry-out reconstruct the exact sum.
        let sum = &ba + &bb;
        assert_eq!(BigUint::from(add_word(a, b)), &sum % &base);
        assert_eq!(add_overflow_word(a, b), sum >= base);
        let sum_carry = &ba + &bb + 1u32;
        assert_eq!(BigUint::from(add_with_carry_word(a, b)), &sum_carry % &base);
        assert_eq!(add_with_carry_overflow_word(a, b), sum_carry >= base);

        // Subtraction wraps modulo 2^64; monus saturates at zero.
        let wrapped_diff = &ba + &base - &bb;
        assert_eq!(BigUint::from(minus_word(a, b)), &wrapped_diff % &base);
        assert_eq!(
            BigUint::from(monus_word(a, b)),
            if ba >= bb { &ba - &bb } else { BigUint::ZERO }
        );

        // Multiplication: the overflow word holds the high 64 bits, so together with the low word
        // they reconstruct the full 128-bit product.
        let product = &ba * &bb;
        assert_eq!(
            BigUint::from(times_overflow_word(a, b)) * &base + times_word(a, b),
            product
        );
        // times_with_carry adds a third word before splitting into hi/lo.
        let fused = &ba * &bb + &bc;
        assert_eq!(
            BigUint::from(times_with_carry_overflow_word(a, b, c)) * &base + times_with_carry_word(a, b, c),
            fused
        );

        // Successor / predecessor wrap.
        assert_eq!(BigUint::from(succ_word(a)), (&ba + 1u32) % &base);
        assert_eq!(BigUint::from(pred_word(a)), (&ba + &base - 1u32) % &base);

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
        assert_eq!(BigUint::from(shift_right(bit, a)), (&ba >> 1u32) + inserted);
        assert_eq!(rightmost_bit(a), &ba % 2u32 == BigUint::from(1u32));
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_div_mod_identities() {
    random_test(10_000, |rng| {
        let a: u64 = rng.random();
        let base = BigUint::from(1u32) << 64u32;

        // Single-word Euclidean division: a == q*divisor + r with 0 <= r < divisor.
        let divisor = rng.random::<u64>() | 1; // odd, hence non-zero
        let q = div_word(a, divisor);
        let r = mod_word(a, divisor);
        assert_eq!(BigUint::from(q) * divisor + r, BigUint::from(a));
        assert!(r < divisor);

        // Double-word numerator over a single-word divisor. Keep the high word below the divisor
        // so the quotient still fits in one word (the C++ contract for div_doubleword).
        let c = rng.random::<u64>() | 1;
        let hi = a % c;
        let lo: u64 = rng.random();
        let numerator = BigUint::from(hi) * &base + lo;
        let bc = BigUint::from(c);
        let dq = div_doubleword(hi, lo, c);
        let dr = mod_doubleword(hi, lo, c);
        assert_eq!(BigUint::from(dq), &numerator / &bc);
        assert_eq!(BigUint::from(dr), &numerator % &bc);

        // Wider divisions truncate their word-valued result to the low 64 bits, exactly like the
        // C++ static_cast; verify against the binary reference with the same truncation.
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
    // The integer square root `r` of `n` is defined by `r*r <= n < (r+1)*(r+1)`.
    let check_bounds = |root: &BigUint, n: &BigUint| {
        let next = root + 1u32;
        assert!(root * root <= *n, "root too large: {root}^2 > {n}");
        assert!(*n < &next * &next, "root too small: {n} >= {next}^2");
    };

    random_test(5_000, |rng| {
        let base = BigUint::from(1u32) << 64u32;
        let (n1, n2, n3, n4): (u64, u64, u64, u64) = (rng.random(), rng.random(), rng.random(), rng.random());

        // Single word.
        check_bounds(&BigUint::from(sqrt_word(n1)), &BigUint::from(n1));

        // Double word (< 2^128): the root still fits in a single word.
        check_bounds(&BigUint::from(sqrt_doubleword(n1, n2)), &big_from_digits(&[n1, n2]));

        // Triple word: the root can exceed a word, split into low and overflow.
        let root = BigUint::from(sqrt_tripleword_overflow(n1, n2, n3)) * &base + sqrt_tripleword(n1, n2, n3);
        check_bounds(&root, &big_from_digits(&[n1, n2, n3]));

        // Quadruple word.
        let root =
            BigUint::from(sqrt_quadrupleword_overflow(n1, n2, n3, n4)) * &base + sqrt_quadrupleword(n1, n2, n3, n4);
        check_bounds(&root, &big_from_digits(&[n1, n2, n3, n4]));
    });
}
