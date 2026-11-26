#![forbid(unsafe_code)]

use merc_data::DataExpression;
use merc_utilities::FixedSizeCache;
use merc_utilities::LruPolicy;

/// The default maximum number of entries a [ConditionCache] holds before it
/// starts evicting the least recently used one.
pub const DEFAULT_MAX_ENTRIES: usize = 1 << 16;

/// A cache of condition-side normal forms, keyed by the substituted
/// (already-matched) term rather than by any rule's pattern, and shared
/// across an entire rewrite computation.
///
/// `merc_aterm`'s maximal sharing means structural equality of a
/// [DataExpression] is pointer equality, so hashing and comparing keys is
/// cheap regardless of term size, and two condition sides that substitute to
/// the same concrete term hit the same entry regardless of which rule or
/// variable name produced either one.
///
/// A disabled cache stores nothing, so every condition side is normalised.
pub struct ConditionCache {
    cache: Option<FixedSizeCache<DataExpression, DataExpression, LruPolicy<DataExpression>>>,
}

impl ConditionCache {
    /// Creates an empty cache holding at most `max_entries` normal forms
    /// (`Some(0)` means unbounded; see [FixedSizeCache::new]), or a disabled
    /// one for `None`.
    pub fn new(max_entries: Option<usize>) -> Self {
        Self {
            cache: max_entries.map(|max_entries| FixedSizeCache::new(max_entries, LruPolicy::default())),
        }
    }

    /// Returns `term`'s cached normal form, if present.
    pub fn get(&mut self, term: &DataExpression) -> Option<DataExpression> {
        self.cache.as_mut()?.get(term).cloned()
    }

    /// Records `normal_form` as `term`'s normal form, evicting the least
    /// recently used entry first if the cache is full.
    pub fn insert(&mut self, term: DataExpression, normal_form: DataExpression) {
        if let Some(cache) = &mut self.cache {
            cache.insert(term, normal_form);
        }
    }

    /// Returns the number of entries currently cached.
    pub fn len(&self) -> usize {
        self.cache.as_ref().map_or(0, FixedSizeCache::len)
    }

    /// Returns true if the cache contains no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for ConditionCache {
    fn default() -> Self {
        Self::new(Some(DEFAULT_MAX_ENTRIES))
    }
}
