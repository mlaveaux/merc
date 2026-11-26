use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use merc_io::TimeProgress;

/// A `TimeProgress` whose closure counts how often it fired.
fn counting_progress(interval_seconds: u64) -> (TimeProgress<()>, Arc<AtomicUsize>) {
    let count = Arc::new(AtomicUsize::new(0));
    let counter = count.clone();
    let progress = TimeProgress::new(
        move |()| {
            counter.fetch_add(1, Ordering::Relaxed);
        },
        interval_seconds,
    );
    (progress, count)
}

#[test]
fn test_zero_interval_fires_every_call() {
    let (progress, count) = counting_progress(0);

    assert!(progress.is_due(), "a zero interval is always due");
    for _ in 0..50 {
        progress.print(());
    }
    progress.reset();
    assert!(progress.is_due(), "a zero interval is due right after a reset");
    progress.print(());
    assert_eq!(
        count.load(Ordering::Relaxed),
        51,
        "every print must fire at a zero interval"
    );
}

#[test]
fn test_large_interval_does_not_fire() {
    let (progress, count) = counting_progress(3600);

    assert!(!progress.is_due(), "a one-hour interval cannot be due immediately");
    for _ in 0..100 {
        progress.print(());
    }
    assert_eq!(
        count.load(Ordering::Relaxed),
        0,
        "no print should fire within the interval"
    );
}

#[test]
fn test_message_receives_object() {
    let last = Arc::new(AtomicUsize::new(0));
    let seen = last.clone();
    let progress = TimeProgress::new(move |value: usize| seen.store(value, Ordering::Relaxed), 0);

    progress.print(42);
    assert_eq!(last.load(Ordering::Relaxed), 42);
}
