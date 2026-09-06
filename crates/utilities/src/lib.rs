#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]

#[macro_use]
mod cast_macro;

mod debug_trace;
mod error;
mod fixed_cache_policy;
mod fixed_size_cache;
mod generational_index;
mod helper;
mod kani_rng;
mod permutation;
mod pest_display_pair;
mod random_test;
mod sharded_counter;
mod snapshot;
mod source_map;
mod span;
mod tagged_index;
mod test_logger;
mod timing;
mod traversal;

pub use error::MercError;
pub use fixed_cache_policy::CachePolicy;
pub use fixed_cache_policy::FifoPolicy;
pub use fixed_cache_policy::LruPolicy;
pub use fixed_cache_policy::NoPolicy;
pub use fixed_size_cache::FixedSizeCache;
pub use generational_index::GenerationCounter;
pub use generational_index::GenerationalIndex;
pub use helper::PhantomUnsend;
pub use helper::PhantomUnsync;
pub use permutation::is_valid_permutation;
pub use pest_display_pair::DisplayPair;
pub use random_test::random_test;
pub use random_test::random_test_threads;
pub use sharded_counter::ShardedCounter;
pub use snapshot::check_snapshot;
pub use snapshot::ensure_snapshot_version;
pub use source_map::SourceId;
pub use source_map::SourceMap;
pub use span::Span;
pub use span::Spanned;
pub use span::respan;
pub use span::with_offset_corrections;
pub use tagged_index::IdAllocator;
pub use tagged_index::MercIndex;
pub use tagged_index::TagIndex;
pub use test_logger::test_logger;
pub use test_logger::test_threads;
pub use timing::Timing;
pub use traversal::Step;
pub use traversal::Visit;

#[cfg(kani)]
pub use kani_rng::*;
