use std::collections::HashMap;
use std::collections::VecDeque;
use std::hash::Hash;

use crate::SlotList;

/// A trait for implementing cache replacement policies.
///
/// Cache policies determine which elements to remove when the cache is full.
pub trait CachePolicy<K>: Default {
    /// Called when the cache is cleared.
    fn clear(&mut self);

    /// Called when a new element is inserted into the cache.
    fn inserted(&mut self, key: K);

    /// Called when an element is accessed in the cache.
    fn touch(&mut self, key: &K);

    /// Returns the key that should be replaced when the cache is full.
    fn replacement_candidate<V>(&mut self, cache: &HashMap<K, V>) -> Option<K>;
}

/// A policy that replaces an arbitrary element.
#[derive(Debug, Default)]
pub struct NoPolicy;

impl<K: Clone> CachePolicy<K> for NoPolicy {
    fn clear(&mut self) {}
    fn inserted(&mut self, _key: K) {}
    fn touch(&mut self, _key: &K) {}

    fn replacement_candidate<V>(&mut self, cache: &HashMap<K, V>) -> Option<K> {
        cache.keys().next().cloned()
    }
}

/// A First-In-First-Out (FIFO) replacement policy.
#[derive(Debug)]
pub struct FifoPolicy<K> {
    queue: VecDeque<K>,
}

impl<K> FifoPolicy<K> {
    pub fn new() -> Self {
        Self { queue: VecDeque::new() }
    }
}

impl<K> Default for FifoPolicy<K> {
    fn default() -> Self {
        Self { queue: VecDeque::new() }
    }
}

impl<K: Clone + Eq + Hash> CachePolicy<K> for FifoPolicy<K> {
    fn clear(&mut self) {
        self.queue.clear();
    }

    fn inserted(&mut self, key: K) {
        self.queue.push_back(key);
    }

    fn touch(&mut self, _key: &K) {
        // FIFO policy doesn't use access information
    }

    fn replacement_candidate<V>(&mut self, cache: &HashMap<K, V>) -> Option<K> {
        while let Some(key) = self.queue.pop_front() {
            if cache.contains_key(&key) {
                return Some(key);
            }
        }
        None
    }
}

/// A Least-Recently-Used (LRU) replacement policy.
///
/// Keeps the recency order in a [SlotList], indexed by key, so that `touch`
/// runs in constant time on every cache hit.
#[derive(Debug)]
pub struct LruPolicy<K> {
    /// The slot of every tracked key.
    index: HashMap<K, usize>,
    /// Tracked keys, from least to most recently used.
    order: SlotList<K>,
}

impl<K> LruPolicy<K> {
    pub fn new() -> Self {
        Self {
            index: HashMap::new(),
            order: SlotList::new(),
        }
    }
}

impl<K> Default for LruPolicy<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Clone + Eq + Hash> CachePolicy<K> for LruPolicy<K> {
    fn clear(&mut self) {
        self.index.clear();
        self.order.clear();
    }

    fn inserted(&mut self, key: K) {
        if self.index.contains_key(&key) {
            self.touch(&key);
            return;
        }

        let slot = self.order.push_back(key.clone());
        self.index.insert(key, slot);
    }

    fn touch(&mut self, key: &K) {
        if let Some(&slot) = self.index.get(key) {
            self.order.move_to_back(slot);
        }
    }

    fn replacement_candidate<V>(&mut self, cache: &HashMap<K, V>) -> Option<K> {
        while let Some(key) = self.order.pop_front() {
            self.index.remove(&key);
            if cache.contains_key(&key) {
                return Some(key);
            }
        }
        None
    }
}
