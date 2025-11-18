use std::hint::black_box;

use criterion::Criterion;
use merc_sharedmutex::BfVec;

use benchmarks_sharedmutex::NUM_ITERATIONS;
use benchmarks_sharedmutex::READ_RATIOS;
use benchmarks_sharedmutex::THREADS;
use benchmarks_sharedmutex::benchmark;

pub fn benchmark_vector(c: &mut Criterion) {
    for num_threads in THREADS {
        for read_ratio in READ_RATIOS {
            benchmark(
                c,
                "bf-sharedmutex::BfVec",
                VecClone::<usize>::new(),
                |x| {
                    black_box(x.vector.len());
                },
                |x| {
                    x.vector.push(1);
                    black_box(());
                },
                num_threads,
                NUM_ITERATIONS,
                read_ratio,
            );
        }
    }
}

/// A hack where clone does actually call share
struct VecClone<T> {
    vector: BfVec<T>,
}

impl<T> VecClone<T> {
    fn new() -> Self {
        Self { vector: BfVec::new() }
    }
}

impl<T> Clone for VecClone<T> {
    fn clone(&self) -> Self {
        Self {
            vector: self.vector.share(),
        }
    }
}
