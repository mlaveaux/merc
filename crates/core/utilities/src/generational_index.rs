use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::ops::Deref;

/// A generational index that stores both an index and a generation counter.
/// The generation is only tracked in debug builds to avoid overhead in release.
///
/// This allows detecting use-after-free scenarios in debug mode while
/// maintaining zero overhead in release mode.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct GenerationalIndex<I: Copy + Into<usize> = usize> {
    /// The raw index value
    index: I,

    #[cfg(debug_assertions)]
    /// Generation counter, only available in debug builds
    generation: usize,
}

impl Default for GenerationalIndex<usize> {
    fn default() -> Self {
        GenerationalIndex {
            index: 0,
            #[cfg(debug_assertions)]
            generation: usize::MAX,
        }
    }
}

impl<I: Copy + Into<usize>> Deref for GenerationalIndex<I> {
    type Target = I;

    /// Deref implementation to access the underlying index value.
    fn deref(&self) -> &Self::Target {
        &self.index
    }
}

impl<I: Copy + Into<usize>> GenerationalIndex<I> {
    /// Creates a new generational index with the specified index and generation.
    #[cfg(debug_assertions)]
    fn new(index: I, generation: usize) -> Self {
        Self { index, generation }
    }

    /// Creates a new generational index with the specified index.
    #[cfg(not(debug_assertions))]
    fn new(index: I) -> Self {
        Self { index }
    }
}

/// A counter that keeps track of generational indices.
/// This helps manage generations of indices to detect use-after-free and similar issues.
#[derive(Clone, Debug, Default)]
pub struct GenerationCounter {
    /// Current generation count, only stored in debug builds
    #[cfg(debug_assertions)]
    current_generation: Vec<usize>,
}

impl GenerationCounter {
    /// Creates a new generation counter.
    pub fn new() -> Self {
        #[cfg(debug_assertions)]
        {
            Self {
                current_generation: Vec::new(),
            }
        }

        #[cfg(not(debug_assertions))]
        Self {}
    }
}

impl GenerationCounter {
    /// Creates a new generational index with the given index and the next generation.
    pub fn create_index<I>(&mut self, index: I) -> GenerationalIndex<I>
    where
        I: Copy + Into<usize>,
    {
        #[cfg(debug_assertions)]
        {
            let generation = if self.current_generation.len() <= index.into() {
                self.current_generation.resize(index.into() + 1, 0);
                0
            } else {
                let generation = &mut self.current_generation[index.into()];
                *generation = generation.wrapping_add(1);
                *generation
            };

            GenerationalIndex::new(index, generation)
        }

        #[cfg(not(debug_assertions))]
        {
            GenerationalIndex::new(index)
        }
    }

    /// Returns a generational index with the given index and the current generation.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `index` was never created by [`create_index`](Self::create_index) on this counter.
    pub fn recall_index<I>(&self, index: I) -> GenerationalIndex<I>
    where
        I: Copy + Into<usize>,
    {
        #[cfg(debug_assertions)]
        {
            let idx = index.into();
            assert!(
                idx < self.current_generation.len(),
                "recall_index called with index {idx} that was never created"
            );
            GenerationalIndex::new(index, self.current_generation[idx])
        }
        #[cfg(not(debug_assertions))]
        {
            GenerationalIndex::new(index)
        }
    }

    /// Returns whether the given generational index is still valid, i.e. its
    /// generation matches the current generation of the underlying slot.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `index` was never created by [`create_index`](Self::create_index) on this counter.
    pub fn is_valid<I>(&self, index: GenerationalIndex<I>) -> bool
    where
        I: Copy + Into<usize>,
    {
        #[cfg(debug_assertions)]
        {
            let idx = index.index.into();
            assert!(
                idx < self.current_generation.len(),
                "is_valid called with index {idx} that was never created"
            );
            self.current_generation[idx] == index.generation
        }
        #[cfg(not(debug_assertions))]
        {
            let _ = index;
            true
        }
    }

    /// Returns the underlying index.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `index` was never created by [`create_index`](Self::create_index) on this
    /// counter, or if its generation is stale (a use-after-free of a recycled index).
    pub fn get_index<I>(&self, index: GenerationalIndex<I>) -> I
    where
        I: Copy + Into<usize> + fmt::Debug,
    {
        #[cfg(debug_assertions)]
        {
            let idx = index.index.into();
            assert!(
                idx < self.current_generation.len(),
                "get_index called with index {idx} that was never created"
            );
            if self.current_generation[idx] != index.generation {
                panic!("Attempting to access an invalid index: {index:?}");
            }
        }

        index.index
    }
}

// Standard trait implementations for GenerationalIndex

impl<I> PartialEq for GenerationalIndex<I>
where
    I: Copy + Into<usize> + Eq,
{
    fn eq(&self, other: &Self) -> bool {
        #[cfg(debug_assertions)]
        {
            // A sentinel (default) index has the `usize::MAX` generation. It is
            // only equal to another sentinel index with the same raw index, and
            // never aliases a live generation. Handling it here keeps `eq`
            // reflexive (`default == default`), which is required for `Eq` and
            // for use as a hash-map key.
            let self_sentinel = self.generation == usize::MAX;
            let other_sentinel = other.generation == usize::MAX;
            if self_sentinel || other_sentinel {
                return self_sentinel && other_sentinel && self.index == other.index;
            }

            debug_assert_eq!(
                self.generation, other.generation,
                "Comparing indices of different generations"
            );
        }

        self.index == other.index
    }
}

impl<I> Eq for GenerationalIndex<I> where I: Copy + Into<usize> + Eq {}

impl<I> PartialOrd for GenerationalIndex<I>
where
    I: Copy + Into<usize> + PartialOrd + Eq,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        // Skip the generation check for the sentinel (default) generation, to
        // stay consistent with `PartialEq`.
        #[cfg(debug_assertions)]
        if self.generation != usize::MAX && other.generation != usize::MAX {
            debug_assert_eq!(
                self.generation, other.generation,
                "Comparing indices of different generations"
            );
        }

        self.index.partial_cmp(&other.index)
    }
}

impl<I> Ord for GenerationalIndex<I>
where
    I: Copy + Into<usize> + Eq + Ord,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Skip the generation check for the sentinel (default) generation, to
        // stay consistent with `PartialEq`.
        #[cfg(debug_assertions)]
        if self.generation != usize::MAX && other.generation != usize::MAX {
            debug_assert_eq!(
                self.generation, other.generation,
                "Comparing indices of different generations"
            );
        }
        self.index.cmp(&other.index)
    }
}

impl<I> Hash for GenerationalIndex<I>
where
    I: Copy + Into<usize> + Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
    }
}

impl<I> fmt::Debug for GenerationalIndex<I>
where
    I: Copy + Into<usize> + fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[cfg(debug_assertions)]
        {
            write!(
                f,
                "GenerationalIndex(index: {:?}, generation: {})",
                self.index, self.generation
            )
        }
        #[cfg(not(debug_assertions))]
        {
            write!(f, "GenerationalIndex(index: {:?})", self.index)
        }
    }
}

impl fmt::Display for GenerationalIndex<usize> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.index)
    }
}
