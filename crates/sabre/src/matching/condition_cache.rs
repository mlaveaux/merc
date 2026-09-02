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
pub struct ConditionCache {
    cache: FixedSizeCache<DataExpression, DataExpression, LruPolicy<DataExpression>>,
}

impl ConditionCache {
    /// Creates an empty cache holding at most `max_entries` normal forms (0
    /// means unbounded; see [FixedSizeCache::new]).
    pub fn new(max_entries: usize) -> Self {
        Self {
            cache: FixedSizeCache::new(max_entries, LruPolicy::default()),
        }
    }

    /// Returns `term`'s cached normal form, if present.
    pub fn get(&mut self, term: &DataExpression) -> Option<DataExpression> {
        self.cache.get(term).cloned()
    }

    /// Records `normal_form` as `term`'s normal form, evicting the least
    /// recently used entry first if the cache is full.
    pub fn insert(&mut self, term: DataExpression, normal_form: DataExpression) {
        self.cache.insert(term, normal_form);
    }

    /// Returns the number of entries currently cached.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Returns true if the cache contains no entries.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

impl Default for ConditionCache {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_ENTRIES)
    }
}

#[cfg(test)]
mod tests {
    use merc_data::DataExpression;

    use super::ConditionCache;

    #[test]
    fn test_miss_then_hit() {
        let mut cache = ConditionCache::new(0);
        let key = DataExpression::from_string("even(s(s(d0)))").unwrap();
        let value = DataExpression::from_string("true").unwrap();

        assert!(cache.get(&key).is_none());
        cache.insert(key.clone(), value.clone());
        assert_eq!(cache.get(&key), Some(value));
    }

    #[test]
    fn test_structurally_equal_terms_share_an_entry() {
        // merc_aterm's maximal sharing makes two independently-parsed but
        // structurally identical terms the same interned term, so a lookup
        // built from a different (but equal) DataExpression still hits.
        let mut cache = ConditionCache::new(0);
        let key = DataExpression::from_string("even(s(s(d0)))").unwrap();
        let value = DataExpression::from_string("true").unwrap();
        cache.insert(key, value.clone());

        let other_key = DataExpression::from_string("even(s(s(d0)))").unwrap();
        assert_eq!(cache.get(&other_key), Some(value));
    }

    #[test]
    fn test_eviction_bounds_the_cache_size() {
        let mut cache = ConditionCache::new(2);
        let a = DataExpression::from_string("a").unwrap();
        let b = DataExpression::from_string("b").unwrap();
        let c = DataExpression::from_string("c").unwrap();
        let v = DataExpression::from_string("true").unwrap();

        cache.insert(a.clone(), v.clone());
        cache.insert(b, v.clone());
        cache.insert(c, v);

        assert_eq!(cache.len(), 2, "the cache never holds more than max_entries");
        assert!(
            cache.get(&a).is_none(),
            "the oldest, least recently used entry is evicted first"
        );
    }
}
