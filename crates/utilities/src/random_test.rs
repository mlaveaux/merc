use rand::RngCore;
use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::test_logger;

/// Constructs a random number generator that should be used in random tests. Prints its seed to the console for reproducibility.
pub fn random_test<F>(iterations: usize, mut test_function: F)
where
    F: FnMut(&mut StdRng),
{
    test_logger();

    if let Ok(seed_str) = std::env::var("MERC_SEED") {
        let seed = seed_str.parse::<u64>().expect("MERC_SEED must be a valid u64");
        println!("seed: {seed} (fixed by MERC_SEED)");
        let mut rng = StdRng::seed_from_u64(seed);
        for _ in 0..iterations {
            test_function(&mut rng);
        }
        return;
    }

    let seed: u64 = rand::random();
    println!("random seed: {seed} (use MERC_SEED=<seed> to set fixed seed)");
    let mut rng = StdRng::seed_from_u64(seed);

    for _ in 0..iterations {
        test_function(&mut rng);
    }
}

/// Can be used to run a random test with a specific seed for reproducibility.
pub fn random_test_seeded<F>(seed: u64, iterations: usize, mut test_function: F)
where
    F: FnMut(&mut StdRng),
{
    test_logger();

    println!("seed: {seed}");
    let mut rng = StdRng::seed_from_u64(seed);

    for _ in 0..iterations {
        test_function(&mut rng);
    }
}

pub fn random_test_threads<C, F, G>(iterations: usize, num_threads: usize, init_function: G, test_function: F)
where
    C: Send + 'static,
    F: Fn(&mut StdRng, &mut C) + Copy + Send + Sync + 'static,
    G: Fn() -> C,
{
    test_logger();

    let mut threads = vec![];

    let seed: u64 = rand::random();
    println!("seed: {seed}");
    let mut rng = StdRng::seed_from_u64(seed);

    for _ in 0..num_threads {
        let mut rng = StdRng::seed_from_u64(rng.next_u64());
        let mut init = init_function();
        threads.push(std::thread::spawn(move || {
            for _ in 0..iterations {
                test_function(&mut rng, &mut init);
            }
        }));
    }

    for thread in threads {
        let _ = thread.join();
    }
}
