use merc_number::bits_for_value;

#[test]
fn test_bits_for_value() {
    // Zero is a special case that still needs a single bit.
    assert_eq!(bits_for_value(0), 1);

    assert_eq!(bits_for_value(1), 1);
    assert_eq!(bits_for_value(2), 2);
    assert_eq!(bits_for_value(3), 2);
    assert_eq!(bits_for_value(4), 3);
    assert_eq!(bits_for_value(255), 8);
    assert_eq!(bits_for_value(256), 9);
    assert_eq!(bits_for_value(usize::MAX), usize::BITS as u8);
}
