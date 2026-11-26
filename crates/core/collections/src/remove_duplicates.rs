use std::hash::Hash;

use rustc_hash::FxHashMap;

use crate::ByteCompressedVec;
use crate::CompressedEntry;

/// Below this many entries in a single bucket, duplicates are detected with a
/// linear scan (cheap, no allocation). Above it, a reused hash map scratch
/// buffer is used instead, so a handful of very high-multiplicity buckets
/// cannot make [`dedup_grouped`] quadratic in the bucket size.
pub const HASH_DEDUP_THRESHOLD: usize = 16;

/// What [`dedup_grouped`]/[`dedup_by_bucket`] found for a single entry; passed
/// to their callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DedupOutcome {
    /// This entry is the first with its `(bucket, key)` in the input, so it
    /// survives. `position` is its 0-based rank among kept entries so far -
    /// i.e. the index it would have in a compacted, bucket-grouped output.
    Keep { position: usize },
    /// This entry duplicates an earlier one in the same bucket. `position` is
    /// the one a caller received in that earlier entry's `Keep` outcome, so a
    /// caller merging payloads knows exactly where to merge this one into.
    Duplicate { position: usize },
}

/// Counts `num_entries` entries (numbered `0..num_entries`, in original
/// insertion order) per bucket via `bucket_of`, then scatters them into
/// bucket-grouped order - preserving insertion order within a bucket - via a
/// counting sort, calling `write(position, original_index)` once per entry in
/// that grouped order.
///
/// Returns `bucket_ends`: `bucket_ends[bucket]` is the number of grouped
/// positions occupied by `bucket` and every bucket before it - so `bucket`'s
/// own entries occupy `bucket_ends[bucket - 1]..bucket_ends[bucket]` (reading
/// `bucket_ends[bucket - 1]` as `0` when `bucket == 0`); this is the same
/// array [`dedup_grouped`] takes.
pub fn scatter_into_buckets(
    num_buckets: usize,
    num_entries: usize,
    bucket_of: impl Fn(usize) -> usize,
    mut write: impl FnMut(usize, usize),
) -> Vec<usize> {
    // Count entries per bucket, then turn the counts into prefix-sum start
    // offsets.
    let mut offsets = vec![0usize; num_buckets];
    for i in 0..num_entries {
        offsets[bucket_of(i)] += 1;
    }

    let mut running = 0usize;
    for count in &mut offsets {
        let bucket_len = *count;
        *count = running;
        running += bucket_len;
    }

    // Scatter, preserving insertion order within a bucket. `offsets` doubles
    // as the running cursor here, so afterwards `offsets[bucket]` holds the
    // *end* of that bucket (and thus the start of the next one, since buckets
    // are contiguous) rather than its start.
    for i in 0..num_entries {
        let pos = &mut offsets[bucket_of(i)];
        write(*pos, i);
        *pos += 1;
    }

    offsets
}

/// Compacts entries that are already in bucket-grouped order (see
/// [`scatter_into_buckets`]) - numbered `0..bucket_ends.last()` by their
/// position in that grouped order, *not* by any original index - dropping or
/// merging entries that share the same `key_of` within a bucket.
///
/// Two entries can only be duplicates of each other if they're in the same
/// bucket, so once entries are bucket-grouped this never needs a global sort -
/// just a single walk over each bucket, calling `on_entry(position, bucket,
/// outcome)` once per entry with a [`DedupOutcome`]. Small buckets
/// (`<= HASH_DEDUP_THRESHOLD`) use a linear scan to find a duplicate; larger
/// ones a reused hash map, so a handful of very high out-degree buckets can't
/// make this quadratic in the bucket size.
///
/// A caller that only wants to drop duplicates copies the surviving entry's
/// columns into fresh storage on `Keep` and does nothing on `Duplicate`; one
/// that wants to merge duplicate payloads (e.g. combining two edges' BDD
/// configurations with a boolean `or`) instead merges into whatever it wrote
/// for `Duplicate`'s `position` on the matching `Keep`.
pub fn dedup_grouped<K, FKey, FEntry>(bucket_ends: &[usize], key_of: FKey, mut on_entry: FEntry)
where
    K: Eq + Hash + Copy,
    FKey: Fn(usize) -> K,
    FEntry: FnMut(usize, usize, DedupOutcome),
{
    // That cache only ever needs to cover the *current* bucket: a later bucket
    // never looks back at an earlier one's keys.
    let mut bucket_keys: Vec<K> = Vec::new();
    let mut scratch: FxHashMap<K, usize> = FxHashMap::default();
    let mut start = 0usize;
    let mut total_survivors = 0usize;

    for (bucket, &end) in bucket_ends.iter().enumerate() {
        let bucket_start_position = total_survivors;

        if end - start <= HASH_DEDUP_THRESHOLD {
            // Small bucket: a linear scan avoids any allocation.
            bucket_keys.clear();
            for read in start..end {
                let key = key_of(read);
                let seen = bucket_keys.iter().position(|&seen_key| seen_key == key);

                if let Some(local_position) = seen {
                    on_entry(
                        read,
                        bucket,
                        DedupOutcome::Duplicate {
                            position: bucket_start_position + local_position,
                        },
                    );
                } else {
                    on_entry(
                        read,
                        bucket,
                        DedupOutcome::Keep {
                            position: total_survivors,
                        },
                    );
                    bucket_keys.push(key);
                    total_survivors += 1;
                }
            }
        } else {
            // Large ("hub") bucket: a hash map keeps this from degrading to
            // O(bucket_len^2).
            scratch.clear();
            for read in start..end {
                let key = key_of(read);

                if let Some(&position) = scratch.get(&key) {
                    on_entry(read, bucket, DedupOutcome::Duplicate { position });
                } else {
                    scratch.insert(key, total_survivors);
                    on_entry(
                        read,
                        bucket,
                        DedupOutcome::Keep {
                            position: total_survivors,
                        },
                    );
                    total_survivors += 1;
                }
            }
        }

        start = end;
    }
}

/// Deduplicates `num_entries` entries - numbered `0..num_entries` in original
/// insertion order - into `num_buckets` buckets, without ever globally sorting
/// or permuting the caller's own storage: [`scatter_into_buckets`] builds a
/// `position -> original_index` permutation, then [`dedup_grouped`] runs
/// against it, translating grouped positions back to original indices so
/// `on_entry` never has to deal with the grouped order itself.
///
/// `on_entry` receives the entry's *original* index (and its bucket - the
/// same value `bucket_of` would return for it), not a grouped or compacted
/// one, so a caller can index its own per-entry columns directly by that
/// original index from inside the callback, rather than physically
/// scattering them to match the bucket-grouped order.
pub fn dedup_by_bucket<K, FBucket, FKey, FEntry>(
    num_buckets: usize,
    num_entries: usize,
    bucket_of: FBucket,
    key_of: FKey,
    mut on_entry: FEntry,
) where
    K: Eq + Hash + Copy,
    FBucket: Fn(usize) -> usize,
    FKey: Fn(usize) -> K,
    FEntry: FnMut(usize, usize, DedupOutcome),
{
    if num_entries == 0 {
        return;
    }

    // `grouped` holds `original_index` values up to `num_entries - 1`, written
    // in original-index order (0, 1, 2, ...) at scattered positions.
    let mut grouped: ByteCompressedVec<usize> =
        ByteCompressedVec::with_capacity(num_entries, num_entries.bytes_required());
    grouped.resize_zeroed(num_entries, num_entries.bytes_required());
    let bucket_ends = scatter_into_buckets(num_buckets, num_entries, bucket_of, |position, original_index| {
        grouped.set(position, original_index)
    });
    dedup_grouped(
        &bucket_ends,
        |position| key_of(grouped.index(position)),
        |position, bucket, outcome| on_entry(grouped.index(position), bucket, outcome),
    );
}
