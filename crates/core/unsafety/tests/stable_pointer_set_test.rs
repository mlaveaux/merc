use std::hash::BuildHasherDefault;
use std::hash::Hash;
use std::hash::Hasher;
use std::hash::RandomState;

use allocator_api2::alloc::System;
use dashmap::Equivalent;
use rustc_hash::FxHasher;

use merc_unsafety::StablePointerSet;

#[test]
fn test_insert_and_get() {
    let set = StablePointerSet::new();

    // Insert a value and ensure we get it back
    let (ptr1, inserted) = set.insert(42);
    assert!(inserted);
    // SAFETY: the elements live in `set` for the duration of the test.
    assert_eq!(unsafe { *ptr1.deref() }, 42);

    // Insert the same value and ensure we get the same pointer
    let (ptr2, inserted) = set.insert(42);
    assert!(!inserted);
    assert_eq!(unsafe { *ptr2.deref() }, 42);

    // Pointers to the same value should be identical
    assert_eq!(ptr1, ptr2);

    // Verify that we have only one element
    assert_eq!(set.len(), 1);
}

#[test]
fn test_contains() {
    let set = StablePointerSet::new();
    set.insert(42);
    set.insert(100);

    assert!(set.contains(&42));
    assert!(set.contains(&100));
    assert!(!set.contains(&200));
}

#[test]
fn test_get() {
    let set = StablePointerSet::new();
    set.insert(42);
    set.insert(100);

    let ptr = set.get(&42).expect("Value should exist");
    // SAFETY: the elements live in `set` for the duration of the test.
    assert_eq!(unsafe { *ptr.deref() }, 42);

    let ptr = set.get(&100).expect("Value should exist");
    assert_eq!(unsafe { *ptr.deref() }, 100);

    assert!(set.get(&200).is_none(), "Value should not exist");
}

#[test]
fn test_iteration() {
    let set = StablePointerSet::new();
    set.insert(1);
    set.insert(2);
    set.insert(3);

    // SAFETY: no element is removed while the iterator references are live.
    let mut values: Vec<i32> = unsafe { set.iter() }.copied().collect();
    values.sort();

    assert_eq!(values, vec![1, 2, 3]);
}

#[test]
fn test_stable_pointer_set_insert_equiv_ref() {
    #[derive(PartialEq, Eq, Debug)]
    struct TestValue {
        id: i32,
        name: String,
    }

    impl From<&i32> for TestValue {
        fn from(id: &i32) -> Self {
            TestValue {
                id: *id,
                name: format!("Value-{}", id),
            }
        }
    }

    impl Hash for TestValue {
        fn hash<H: Hasher>(&self, state: &mut H) {
            self.id.hash(state);
        }
    }

    impl Equivalent<TestValue> for i32 {
        fn equivalent(&self, key: &TestValue) -> bool {
            *self == key.id
        }
    }

    let set: StablePointerSet<TestValue> = StablePointerSet::new();

    // Insert using equivalent reference (i32 -> TestValue)
    let (ptr1, inserted) = set.insert_equiv(&42);
    assert!(inserted, "Value should be inserted");
    // SAFETY: the elements live in `set` for the duration of the test.
    assert_eq!(unsafe { ptr1.deref() }.id, 42);
    assert_eq!(unsafe { ptr1.deref() }.name, "Value-42");

    // Try inserting the same value again via equivalent
    let (ptr2, inserted) = set.insert_equiv(&42);
    assert!(!inserted, "Value should not be inserted again");
    assert_eq!(ptr1, ptr2, "Should return the same pointer");

    // Insert a different value
    let (ptr3, inserted) = set.insert_equiv(&100);
    assert!(inserted, "New value should be inserted");
    assert_eq!(unsafe { ptr3.deref() }.id, 100);
    assert_eq!(unsafe { ptr3.deref() }.name, "Value-100");

    // Ensure we have exactly two elements
    assert_eq!(set.len(), 2);
}

#[test]
fn test_stable_pointer_set_remove() {
    let set = StablePointerSet::new();

    // Insert values
    let (ptr1, _) = set.insert(42);
    let (ptr2, _) = set.insert(100);
    assert_eq!(set.len(), 2);

    // SAFETY: `ptr1` and `ptr2` are the last pointers to their elements.
    unsafe {
        // Remove one value
        assert!(set.remove(ptr1));
        assert_eq!(set.len(), 1);
        assert!(!set.contains(&42));
        assert!(set.contains(&100));

        // Remove other value
        assert!(set.remove(ptr2));
        assert_eq!(set.len(), 0);
    }
}

#[test]
fn test_stable_pointer_set_retain() {
    let set = StablePointerSet::new();

    // Insert values
    set.insert(1);
    let (ptr2, _) = set.insert(2);
    set.insert(3);
    let (ptr4, _) = set.insert(4);
    assert_eq!(set.len(), 4);

    // SAFETY: No pointers to the removed (odd) elements are used afterwards,
    // and each predicate borrow is only read while the element is present.
    unsafe {
        // Retain only even numbers
        set.retain(|x| *x.deref() % 2 == 0);
    }

    // Verify results
    assert_eq!(set.len(), 2);
    assert!(!set.contains(&1));
    assert!(set.contains(&2));
    assert!(!set.contains(&3));
    assert!(set.contains(&4));

    // SAFETY: `ptr2` and `ptr4` are the last pointers to their elements.
    unsafe {
        // The retained elements are still reachable through their pointers.
        assert_eq!(*ptr2.deref(), 2);
        assert_eq!(*ptr4.deref(), 4);
        assert!(set.remove(ptr2));
        assert!(set.remove(ptr4));
    }
    assert!(set.is_empty());
}

#[test]
fn test_stable_pointer_set_custom_allocator() {
    // Test with System allocator
    let set: StablePointerSet<i32, RandomState, System> = StablePointerSet::new_in(System);

    // Insert some values
    let (ptr1, inserted) = set.insert(42);
    assert!(inserted);
    let (ptr2, inserted) = set.insert(100);
    assert!(inserted);

    // Check that everything works as expected
    assert_eq!(set.len(), 2);
    // SAFETY: the elements live in `set` for the duration of the test.
    assert_eq!(unsafe { *ptr1.deref() }, 42);
    assert_eq!(unsafe { *ptr2.deref() }, 100);

    // Test contains
    assert!(set.contains(&42));
    assert!(set.contains(&100));
    assert!(!set.contains(&200));
}

#[test]
fn test_stable_pointer_set_custom_hasher_and_allocator() {
    // Use both custom hasher and allocator
    let set: StablePointerSet<i32, BuildHasherDefault<FxHasher>, System> =
        StablePointerSet::with_hasher_in(BuildHasherDefault::<FxHasher>::default(), System);

    // Insert some values
    let (ptr1, inserted) = set.insert(42);
    assert!(inserted);
    let (ptr2, inserted) = set.insert(100);
    assert!(inserted);

    // Check that everything works as expected
    assert_eq!(set.len(), 2);
    // SAFETY: the elements live in `set` for the duration of the test.
    assert_eq!(unsafe { *ptr1.deref() }, 42);
    assert_eq!(unsafe { *ptr2.deref() }, 100);

    // Test contains
    assert!(set.contains(&42));
    assert!(set.contains(&100));
    assert!(!set.contains(&200));
}
