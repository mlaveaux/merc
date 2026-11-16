#![doc = include_str!("../README.md")]

mod block_allocator;
mod chained_pointer_set;
mod concurrent_append_vec;
mod concurrent_indexed_set;
mod counting_allocator;
mod erasable;
mod freelist;
mod global_allocator;
mod index_edge;
mod protection_set;
mod sharded_hashmap;
mod slice_dst;
mod stable_pointer_set;

pub(crate) use erasable::*;
pub(crate) use freelist::*;

pub use block_allocator::AllocBlock;
pub use block_allocator::BlockAllocator;
pub use block_allocator::BlockAllocatorSafe;
pub use chained_pointer_set::ChainedPointerSet;
pub use concurrent_append_vec::ConcurrentAppendVec;
pub use concurrent_indexed_set::ConcurrentIndexedSet;
pub use counting_allocator::AllocCounter;
pub use counting_allocator::AllocMetrics;
pub use erasable::Erasable;
pub use erasable::ErasedPtr;
pub use global_allocator::print_allocator_metrics;
pub use index_edge::Edge;
pub use index_edge::index_edge;
pub use protection_set::ProtectionIndex;
pub use protection_set::ProtectionSet;
pub use sharded_hashmap::ShardedHashMap;
pub use slice_dst::AllocatorDst;
pub use slice_dst::SliceDst;
pub use slice_dst::repr_c;
pub use stable_pointer_set::StablePointer;
pub use stable_pointer_set::StablePointerSet;
