use rand::RngExt;

use merc_io::BytesFormatter;
use merc_io::LargeFormatter;
use merc_utilities::random_test;

#[test]
fn test_bytes_formatter_units() {
    assert_eq!(BytesFormatter(0).to_string(), "0 bytes");
    assert_eq!(BytesFormatter(1).to_string(), "1 bytes");
    assert_eq!(BytesFormatter(999).to_string(), "999 bytes");
    // Exact unit boundaries.
    assert_eq!(BytesFormatter(1_000).to_string(), "1.00 KB");
    assert_eq!(BytesFormatter(1_000_000).to_string(), "1.00 MB");
    assert_eq!(BytesFormatter(1_000_000_000).to_string(), "1.00 GB");
    // Fractional values within a unit.
    assert_eq!(BytesFormatter(1_500).to_string(), "1.50 KB");
    assert_eq!(BytesFormatter(2_500_000_000).to_string(), "2.50 GB");
    // Just below the next boundary rounds up to the boundary in the lower unit.
    assert_eq!(BytesFormatter(999_999).to_string(), "1000.00 KB");
}

/// The unit matches the magnitude of the value, and the number shown is the value in that unit,
/// with two decimals.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_bytes_formatter_random() {
    random_test(1000, |rng| {
        let value: usize = rng.random_range(0..10_000_000_000);
        let formatted = BytesFormatter(value).to_string();
        let (number, unit) = formatted.split_once(' ').expect("a number and a unit");

        let (expected_unit, unit_size) = match value {
            1_000_000_000.. => ("GB", 1_000_000_000),
            1_000_000.. => ("MB", 1_000_000),
            1_000.. => ("KB", 1_000),
            _ => ("bytes", 1),
        };
        assert_eq!(unit, expected_unit, "unit of {value}");

        if unit_size == 1 {
            assert_eq!(number, value.to_string());
        } else {
            let (_, decimals) = number.split_once('.').expect("a fractional part");
            assert_eq!(decimals.len(), 2, "{formatted:?} has two decimals");
            // Rounding to two decimals is off by at most half a hundredth of the unit.
            let shown = number.parse::<f64>().unwrap() * unit_size as f64;
            assert!(
                (shown - value as f64).abs() <= unit_size as f64 * 0.005 + 1e-6,
                "{value} formatted as {formatted:?}"
            );
        }
    });
}

#[test]
fn test_large_formatter() {
    assert_eq!(LargeFormatter(0).to_string(), "0");
    assert_eq!(LargeFormatter(123).to_string(), "123");
    assert_eq!(LargeFormatter(1234).to_string(), "1,234");
    assert_eq!(LargeFormatter(123456).to_string(), "123,456");
    assert_eq!(LargeFormatter(1234567).to_string(), "1,234,567");
    assert_eq!(LargeFormatter(12345678).to_string(), "12,345,678");
    assert_eq!(LargeFormatter(-123i64).to_string(), "-123");
    assert_eq!(LargeFormatter(-123456i64).to_string(), "-123,456");
    assert_eq!(LargeFormatter(-1234567i64).to_string(), "-1,234,567");
    assert_eq!(LargeFormatter(-123456789i64).to_string(), "-123,456,789");
}

/// The digits are unchanged and grouped by three from the right: the first group has one to
/// three digits, all others exactly three.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_large_formatter_random() {
    random_test(1000, |rng| {
        // Vary the magnitude, so that every number of digits occurs.
        let value: i64 = rng.random::<i64>() >> rng.random_range(0..63);
        let formatted = LargeFormatter(value).to_string();
        assert_eq!(formatted.replace(',', ""), value.to_string());

        let groups: Vec<&str> = formatted.trim_start_matches('-').split(',').collect();
        assert!((1..=3).contains(&groups[0].len()), "{value} formatted as {formatted:?}");
        assert!(
            groups[1..].iter().all(|group| group.len() == 3),
            "{value} formatted as {formatted:?}"
        );
    });
}
