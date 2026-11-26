use itertools::Itertools;

use merc_collections::Odometer;

/// Collects every combination of the odometer, with the first changed digit of every step.
fn combinations(radices: &[usize]) -> (Vec<Vec<usize>>, Vec<usize>) {
    let mut odometer = Odometer::new();
    if !odometer.reset(radices.iter().copied()) {
        return (Vec::new(), Vec::new());
    }

    let mut result = vec![odometer.digits().to_vec()];
    let mut changed = Vec::new();
    while let Some(first) = odometer.increment() {
        let previous = result.last().unwrap();
        assert!(
            previous[..first] == odometer.digits()[..first] && previous[first] != odometer.digits()[first],
            "Digit {first} must be the first digit that changed"
        );
        result.push(odometer.digits().to_vec());
        changed.push(first);
    }
    assert!(odometer.digits().iter().all(|&digit| digit == 0));
    (result, changed)
}

#[test]
fn test_odometer_matches_cartesian_product() {
    for radices in [vec![], vec![1], vec![3], vec![2, 3], vec![3, 1, 2], vec![2, 0, 2]] {
        let expected: Vec<Vec<usize>> = radices
            .iter()
            .map(|&radix| 0..radix)
            .multi_cartesian_product()
            .collect();
        let (result, _) = combinations(&radices);
        if radices.is_empty() {
            // multi_cartesian_product yields nothing for no iterators, but there is one empty combination.
            assert_eq!(result, vec![Vec::<usize>::new()]);
        } else {
            assert_eq!(result, expected, "radices {radices:?}");
        }
    }
}

#[test]
fn test_odometer_reports_first_changed_digit() {
    let (_, changed) = combinations(&[2, 2]);
    assert_eq!(changed, vec![1, 0, 1]);
}

/// Resetting reuses the odometer for other radices, starting from all zeros.
#[test]
fn test_odometer_reset() {
    let mut odometer = Odometer::new();
    assert!(odometer.reset([2, 2]));
    assert_eq!(odometer.increment(), Some(1));
    assert_eq!(odometer.increment(), Some(0));
    assert_eq!(odometer.digits(), [1, 0]);
    assert!(!odometer.reset([3, 0]), "a zero radix has no combinations");
    assert!(odometer.reset([3]));
    assert_eq!(odometer.digits(), [0]);
}
