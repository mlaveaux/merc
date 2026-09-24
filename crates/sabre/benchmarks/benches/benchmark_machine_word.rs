#![forbid(unsafe_code)]

use std::hint::black_box;

use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use merc_number::machine_word;
use rand::RngExt;
use rand::SeedableRng;
use rand::rngs::StdRng;

/// Number of pre-generated inputs each measured iteration runs over, chosen to
/// amortise the loop overhead against the cost of one machine-word call.
const BATCH: usize = 1_000;

/// These are the only [`machine_word`] operations that go through an
/// arbitrary-precision integer internally (`div_triple_doubleword` and the
/// triple/quadruple-word square roots): everything else fits in a native
/// `u64`/`u128`. Comparing their cost before and after swapping that backend
/// is the point of this benchmark.
pub fn criterion_benchmark_wide_word_ops(c: &mut Criterion) {
    let mut rng = StdRng::seed_from_u64(0x6d6572635f6e756d);

    let div_inputs: Vec<(u64, u64, u64, u64, u64)> = (0..BATCH)
        .map(|_| {
            (
                rng.random(),
                rng.random(),
                rng.random(),
                rng.random(),
                rng.random::<u64>() | 1, // non-zero low divisor digit
            )
        })
        .collect();

    c.bench_function("div_triple_doubleword", |bencher| {
        bencher.iter(|| {
            for &(n1, n2, n3, n4, n5) in &div_inputs {
                black_box(machine_word::div_triple_doubleword(n1, n2, n3, n4, n5));
            }
        });
    });

    let triple_inputs: Vec<(u64, u64, u64)> = (0..BATCH).map(|_| (rng.random(), rng.random(), rng.random())).collect();

    c.bench_function("sqrt_tripleword", |bencher| {
        bencher.iter(|| {
            for &(n1, n2, n3) in &triple_inputs {
                black_box(machine_word::sqrt_tripleword(n1, n2, n3));
            }
        });
    });

    c.bench_function("sqrt_tripleword_overflow", |bencher| {
        bencher.iter(|| {
            for &(n1, n2, n3) in &triple_inputs {
                black_box(machine_word::sqrt_tripleword_overflow(n1, n2, n3));
            }
        });
    });

    let quadruple_inputs: Vec<(u64, u64, u64, u64)> = (0..BATCH)
        .map(|_| (rng.random(), rng.random(), rng.random(), rng.random()))
        .collect();

    c.bench_function("sqrt_quadrupleword", |bencher| {
        bencher.iter(|| {
            for &(n1, n2, n3, n4) in &quadruple_inputs {
                black_box(machine_word::sqrt_quadrupleword(n1, n2, n3, n4));
            }
        });
    });

    c.bench_function("sqrt_quadrupleword_overflow", |bencher| {
        bencher.iter(|| {
            for &(n1, n2, n3, n4) in &quadruple_inputs {
                black_box(machine_word::sqrt_quadrupleword_overflow(n1, n2, n3, n4));
            }
        });
    });
}

criterion_group!(benches, criterion_benchmark_wide_word_ops);
criterion_main!(benches);
