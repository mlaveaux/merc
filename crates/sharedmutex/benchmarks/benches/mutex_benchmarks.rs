use criterion::Criterion;

use merc_sharedmutex::BfSharedMutex;

use benchmarks_sharedmutex::NUM_ITERATIONS;
use benchmarks_sharedmutex::READ_RATIOS;
use benchmarks_sharedmutex::THREADS;
use benchmarks_sharedmutex::benchmark;

/// Benchmark the bfsharedmutex implementation
pub fn benchmark_bfsharedmutex(c: &mut Criterion) {
    for num_threads in THREADS {
        for read_ratio in READ_RATIOS {
            // Benchmark various configurations.
            benchmark(
                c,
                "bf-sharedmutex::BfSharedMutex",
                BfSharedMutex::new(()),
                |shared| {
                    let _guard = shared.read().unwrap();
                },
                |shared| {
                    let _guard = shared.write().unwrap();
                },
                num_threads,
                NUM_ITERATIONS,
                read_ratio,
            );
        }
    }
}
