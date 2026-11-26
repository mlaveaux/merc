use std::ptr::NonNull;
use std::sync::Arc;
use std::thread;

use merc_utilities::AtomicTaggedPtr;
use merc_utilities::AtomicTaggedUsize;

/// The loaded pointer equals the original one whatever the tag, and is still usable to read the
/// value (under miri this also checks that tagging preserves the provenance of the pointer).
#[test]
fn test_tag_does_not_change_pointer() {
    let value = 42u64;
    let ptr = NonNull::from(&value);
    let tagged = AtomicTaggedPtr::new(ptr);

    assert!(!tagged.is_tagged());
    assert!(!tagged.set_tag(), "the tag was not set before");
    assert!(tagged.is_tagged());
    assert!(tagged.set_tag(), "the tag was already set");
    assert_eq!(tagged.load(), ptr.as_ptr());
    // SAFETY: `load` returns the pointer to `value`, which is alive and only read.
    assert_eq!(unsafe { *tagged.load() }, 42);

    tagged.clear_tag();
    assert!(!tagged.is_tagged());
    assert_eq!(tagged.load(), ptr.as_ptr());
    // SAFETY: as above.
    assert_eq!(unsafe { *tagged.load() }, 42);
}

#[test]
#[cfg_attr(miri, ignore)] // Too slow under miri.
fn test_concurrent_loads_see_untagged_pointer() {
    let value = Box::new(7u32);
    let ptr = NonNull::from(&*value);
    let tagged = Arc::new(AtomicTaggedPtr::new(ptr));

    let toggler = {
        let tagged = tagged.clone();
        thread::spawn(move || {
            for _ in 0..10_000 {
                tagged.set_tag();
                tagged.clear_tag();
            }
        })
    };

    for _ in 0..10_000 {
        assert_eq!(tagged.load(), ptr.as_ptr());
    }
    toggler.join().unwrap();
}

#[test]
fn test_tag_does_not_change_value() {
    // The largest value that leaves the tag bit free.
    for value in [0, 42, usize::MAX >> 1] {
        let tagged = AtomicTaggedUsize::new(value);

        assert!(!tagged.is_tagged());
        assert!(!tagged.set_tag(), "the tag was not set before");
        assert!(tagged.is_tagged());
        assert!(tagged.set_tag(), "the tag was already set");
        assert_eq!(tagged.load(), value);

        tagged.clear_tag();
        assert!(!tagged.is_tagged());
        assert_eq!(tagged.load(), value);
    }
}

#[test]
#[should_panic(expected = "must leave the tag bit free")]
fn test_value_with_tag_bit_is_rejected() {
    AtomicTaggedUsize::new(1 << (usize::BITS - 1));
}

#[test]
#[cfg_attr(miri, ignore)] // Too slow under miri.
fn test_concurrent_loads_see_untagged_value() {
    let tagged = Arc::new(AtomicTaggedUsize::new(7));

    let toggler = {
        let tagged = tagged.clone();
        thread::spawn(move || {
            for _ in 0..10_000 {
                tagged.set_tag();
                tagged.clear_tag();
            }
        })
    };

    for _ in 0..10_000 {
        assert_eq!(tagged.load(), 7);
    }
    toggler.join().unwrap();
}
