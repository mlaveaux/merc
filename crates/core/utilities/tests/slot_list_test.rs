use std::collections::HashSet;

use rand::RngExt;

use merc_utilities::SlotList;
use merc_utilities::random_test;

#[test]
fn test_slot_list_order() {
    let mut list = SlotList::new();
    let a = list.push_back('a');
    list.push_back('b');
    let c = list.push_back('c');

    list.move_to_back(a);
    assert_eq!(list.remove(c), 'c');

    // Slot of `c` is reused.
    assert_eq!(list.push_back('d'), c);

    for expected in ['b', 'a', 'd'] {
        assert_eq!(list.pop_front(), Some(expected));
    }
    assert_eq!(list.pop_front(), None);
    assert!(list.is_empty());
}

/// Compares a random sequence of operations against a vector of `(slot, value)` pairs in list
/// order.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_slot_list() {
    random_test(100, |rng| {
        let mut list = SlotList::new();
        let mut model: Vec<(usize, u32)> = Vec::new();
        let mut freed: HashSet<usize> = HashSet::new();

        for _ in 0..200 {
            match rng.random_range(0..10) {
                0..4 => {
                    let value = rng.random();
                    let slot = list.push_back(value);
                    assert!(
                        model.iter().all(|&(live, _)| live != slot),
                        "slot {slot} is still in use"
                    );
                    if !freed.is_empty() {
                        assert!(freed.remove(&slot), "slot {slot} is not a freed slot");
                    }
                    model.push((slot, value));
                }
                4..6 if !model.is_empty() => {
                    let index = rng.random_range(0..model.len());
                    let entry = model.remove(index);
                    list.move_to_back(entry.0);
                    model.push(entry);
                }
                6..8 if !model.is_empty() => {
                    let (slot, value) = model.remove(rng.random_range(0..model.len()));
                    assert_eq!(list.remove(slot), value);
                    freed.insert(slot);
                }
                8 => {
                    let expected = (!model.is_empty()).then(|| model.remove(0));
                    assert_eq!(list.pop_front(), expected.map(|(_, value)| value));
                    if let Some((slot, _)) = expected {
                        freed.insert(slot);
                    }
                }
                9 if rng.random_bool(0.1) => {
                    list.clear();
                    model.clear();
                    freed.clear();
                }
                _ => {}
            }

            assert_eq!(list.is_empty(), model.is_empty());
        }

        for (_, value) in model {
            assert_eq!(list.pop_front(), Some(value));
        }
        assert_eq!(list.pop_front(), None);
    });
}
