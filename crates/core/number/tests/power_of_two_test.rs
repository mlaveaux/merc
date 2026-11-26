use merc_number::is_power_of_two;
use merc_number::round_up_to_power_of_two;

#[test]
fn test_is_power_of_two() {
    for value in [1u32, 2, 4, 8, 16, 1 << 31] {
        assert!(is_power_of_two(value), "{value} is a power of two");
    }
    for value in [0u32, 3, 5, 6, 7, 12, u32::MAX] {
        assert!(!is_power_of_two(value), "{value} is not a power of two");
    }
}

#[test]
fn test_round_up_to_power_of_two() {
    assert_eq!(round_up_to_power_of_two(0u32), 1);
    assert_eq!(round_up_to_power_of_two(1u32), 1);
    assert_eq!(round_up_to_power_of_two(2u32), 2);
    assert_eq!(round_up_to_power_of_two(3u32), 4);
    assert_eq!(round_up_to_power_of_two(5u32), 8);
    assert_eq!(round_up_to_power_of_two(8u32), 8);
    assert_eq!(round_up_to_power_of_two(9u32), 16);
}

#[test]
fn test_different_types() {
    assert!(is_power_of_two(4u8));
    assert!(is_power_of_two(8u16));
    assert!(is_power_of_two(32u64));
    assert!(is_power_of_two(64usize));

    assert_eq!(round_up_to_power_of_two(3u8), 4);
    assert_eq!(round_up_to_power_of_two(5u16), 8);
    assert_eq!(round_up_to_power_of_two(17u64), 32);
    assert_eq!(round_up_to_power_of_two(33usize), 64);
}

/// Checks both functions on every `u8` against a definition that does not use bit tricks.
#[test]
fn test_exhaustive_u8() {
    let powers: Vec<u8> = (0..8).map(|shift| 1 << shift).collect();
    for value in 0..=u8::MAX {
        assert_eq!(is_power_of_two(value), powers.contains(&value), "value {value}");
        if value <= 128 {
            let expected = *powers
                .iter()
                .find(|&&power| power >= value)
                .expect("128 is the largest u8 power of two");
            assert_eq!(round_up_to_power_of_two(value), expected, "value {value}");
        }
    }
}

#[test]
#[should_panic(expected = "no representable power-of-two ceiling")]
fn test_round_up_unrepresentable_panics() {
    // 200 rounds up to 256, which does not fit in a u8. This must panic in every build profile,
    // not silently return a wrong value.
    let _ = round_up_to_power_of_two(200u8);
}
