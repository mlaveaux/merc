#[cfg(feature = "merc_metrics")]
use log::info;

#[cfg(feature = "merc_metrics")]
#[global_allocator]
static GLOBAL_ALLOCATOR: crate::AllocCounter = crate::AllocCounter::new();

#[cfg(not(target_env = "msvc"))]
#[cfg(not(feature = "merc_metrics"))]
#[cfg(feature = "merc_jemalloc")]
#[global_allocator]
static GLOBAL_ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(not(feature = "merc_metrics"))]
#[cfg(feature = "merc_mimalloc")]
#[global_allocator]
static GLOBAL_ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Prints information from the [AllocCounter].
#[cfg(feature = "merc_metrics")]
pub fn print_allocator_metrics() {
    info!("{}", GLOBAL_ALLOCATOR.get_metrics());
}

#[cfg(not(feature = "merc_metrics"))]
pub fn print_allocator_metrics() {}
