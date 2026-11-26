use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;

use rand::Rng;
use rand::RngExt;

use merc_collections::DedupOutcome;
use merc_collections::HASH_DEDUP_THRESHOLD;
use merc_collections::dedup_by_bucket;
use merc_collections::dedup_grouped;
use merc_collections::scatter_into_buckets;
use merc_utilities::random_test;

/// Drops duplicate `(bucket, key)` pairs down to the first occurrence of each pair, using
/// [`dedup_by_bucket`], and returns the survivors in the order they were kept. Checks that the
/// `bucket` handed to the callback matches the entry's own bucket, that `Keep` positions are
/// sequential, and that a `Duplicate` points at the position of the entry it duplicates.
fn drop_duplicates(num_buckets: usize, entries: &[(usize, u32)]) -> Vec<(usize, u32)> {
    let mut kept: Vec<(usize, u32)> = Vec::new();
    dedup_by_bucket(
        num_buckets,
        entries.len(),
        |i| entries[i].0,
        |i| entries[i].1,
        |i, bucket, outcome| {
            assert_eq!(bucket, entries[i].0, "on_entry's bucket should match the entry's own");
            match outcome {
                DedupOutcome::Keep { position } => {
                    assert_eq!(position, kept.len(), "Keep positions should be sequential");
                    kept.push(entries[i]);
                }
                DedupOutcome::Duplicate { position } => {
                    assert_eq!(
                        kept[position], entries[i],
                        "Duplicate should point at its first occurrence"
                    );
                }
            }
        },
    );
    kept
}

/// The first occurrence of every `(bucket, key)` pair, grouped by bucket and in input order
/// within a bucket.
fn expected_survivors(num_buckets: usize, entries: &[(usize, u32)]) -> Vec<(usize, u32)> {
    let mut seen = HashSet::new();
    (0..num_buckets)
        .flat_map(|bucket| entries.iter().copied().filter(move |entry| entry.0 == bucket))
        .filter(|&entry| seen.insert(entry))
        .collect()
}

fn random_entries(rng: &mut impl Rng, num_buckets: usize) -> Vec<(usize, u32)> {
    (0..rng.random_range(0..200))
        .map(|_| (rng.random_range(0..num_buckets), rng.random_range(0..10)))
        .collect()
}

#[test]
fn test_drop_small() {
    let entries = [(1, 5), (0, 3), (1, 5), (0, 4), (1, 2), (0, 3)];
    assert_eq!(drop_duplicates(3, &entries), [(0, 3), (0, 4), (1, 5), (1, 2)]);
    assert_eq!(drop_duplicates(1, &[]), []);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_drop_matches_naive_reference() {
    random_test(200, |rng| {
        let num_buckets = rng.random_range(1..20);
        let entries = random_entries(rng, num_buckets);
        assert_eq!(
            drop_duplicates(num_buckets, &entries),
            expected_survivors(num_buckets, &entries)
        );
    });
}

/// A single bucket with more than `HASH_DEDUP_THRESHOLD` entries, which a large-bucket
/// implementation must handle like a small one.
#[test]
fn test_large_bucket() {
    let num_keys = HASH_DEDUP_THRESHOLD as u32 * 2;
    let entries: Vec<(usize, u32)> = (0..2).flat_map(|_| (0..num_keys).map(|key| (0usize, key))).collect();

    let kept = drop_duplicates(1, &entries);
    assert_eq!(kept, (0..num_keys).map(|key| (0, key)).collect::<Vec<_>>());
}

/// Merges duplicate `(bucket, key)` entries' payloads by summing them on `Duplicate`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_merge_matches_naive_reference() {
    random_test(200, |rng| {
        let num_buckets = rng.random_range(1..20);
        let entries: Vec<(usize, u32, u32)> = (0..rng.random_range(0..200))
            .map(|_| {
                (
                    rng.random_range(0..num_buckets),
                    rng.random_range(0..10),
                    rng.random_range(0..100),
                )
            })
            .collect();

        let mut merged: Vec<(usize, u32, u32)> = Vec::new();
        dedup_by_bucket(
            num_buckets,
            entries.len(),
            |i| entries[i].0,
            |i| entries[i].1,
            |i, _bucket, outcome| match outcome {
                DedupOutcome::Keep { position } => {
                    assert_eq!(position, merged.len(), "Keep positions should be sequential");
                    merged.push(entries[i]);
                }
                DedupOutcome::Duplicate { position } => {
                    merged[position].2 += entries[i].2;
                }
            },
        );

        let mut expected: HashMap<(usize, u32), u32> = HashMap::new();
        for &(bucket, key, payload) in &entries {
            *expected.entry((bucket, key)).or_default() += payload;
        }
        assert_eq!(
            merged.len(),
            expected.len(),
            "Every (bucket, key) is merged into one entry"
        );
        let actual: HashMap<(usize, u32), u32> = merged
            .iter()
            .map(|&(bucket, key, payload)| ((bucket, key), payload))
            .collect();
        assert_eq!(actual, expected, "Merged payloads should match a naive reference");
    });
}

/// Uses `scatter_into_buckets` and `dedup_grouped` directly, compacting the scattered keys in
/// place.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_random_scatter_then_dedup_grouped_in_place() {
    random_test(200, |rng| {
        let num_buckets = rng.random_range(1..20);
        let entries = random_entries(rng, num_buckets);

        // Scatter the keys straight into grouped order via a write callback.
        let scattered = RefCell::new(vec![u32::MAX; entries.len()]);
        let bucket_ends = scatter_into_buckets(
            num_buckets,
            entries.len(),
            |i| entries[i].0,
            |position, i| {
                scattered.borrow_mut()[position] = entries[i].1;
            },
        );

        // Bucket-grouped order, stable within a bucket, and `bucket_ends` are the prefix sums of
        // the bucket sizes.
        let grouped: Vec<u32> = (0..num_buckets)
            .flat_map(|bucket| {
                entries
                    .iter()
                    .filter(move |entry| entry.0 == bucket)
                    .map(|entry| entry.1)
            })
            .collect();
        assert_eq!(*scattered.borrow(), grouped);
        let expected_ends: Vec<usize> = (0..num_buckets)
            .map(|bucket| entries.iter().filter(|entry| entry.0 <= bucket).count())
            .collect();
        assert_eq!(bucket_ends, expected_ends);

        let mut write = 0usize;
        let mut kept_buckets: Vec<usize> = Vec::new();
        dedup_grouped(
            &bucket_ends,
            |position| scattered.borrow()[position],
            |position, bucket, outcome| {
                if let DedupOutcome::Keep {
                    position: kept_position,
                } = outcome
                {
                    assert_eq!(kept_position, write, "Keep positions should be sequential");
                    let value = scattered.borrow()[position];
                    scattered.borrow_mut()[write] = value;
                    kept_buckets.push(bucket);
                    write += 1;
                }
            },
        );

        let scattered = scattered.into_inner();
        let kept: Vec<(usize, u32)> = kept_buckets
            .into_iter()
            .zip(scattered[..write].iter().copied())
            .collect();
        assert_eq!(kept, expected_survivors(num_buckets, &entries));
    });
}
