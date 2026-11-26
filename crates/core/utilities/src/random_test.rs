use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::test_logger;

/// Returns the seed for a random test, honoring the `MERC_SEED` environment
/// variable when set so that failures can be reproduced, and otherwise picking a
/// fresh random seed. The chosen seed is printed for reproducibility.
fn random_test_seed() -> u64 {
    if let Ok(seed_str) = std::env::var("MERC_SEED") {
        let seed = seed_str.parse::<u64>().expect("MERC_SEED must be a valid u64");
        println!("seed: {seed} (set by MERC_SEED)");
        seed
    } else {
        let seed: u64 = rand::random();
        println!("random seed: {seed} (use MERC_SEED=<seed> to set fixed seed)");
        seed
    }
}

/// Constructs a random number generator that should be used in random tests. Prints its seed to the console for reproducibility.
pub fn random_test<F>(iterations: usize, mut test_function: F)
where
    F: FnMut(&mut StdRng),
{
    test_logger();

    let mut rng = StdRng::seed_from_u64(random_test_seed());
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

    let mut rng = StdRng::seed_from_u64(random_test_seed());

    for _ in 0..num_threads {
        let mut rng = StdRng::seed_from_u64(rng.next_u64());
        let mut init = init_function();
        threads.push(std::thread::spawn(move || {
            for _ in 0..iterations {
                test_function(&mut rng, &mut init);
            }
        }));
    }

    // Propagate a worker panic so a failing assertion fails the test instead of
    // being silently swallowed.
    for thread in threads {
        thread.join().expect("A worker thread panicked");
    }
}
