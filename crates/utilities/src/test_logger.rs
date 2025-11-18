/// Constructs a logger for tests. This logger will not print anything to the console, but will instead write to a buffer.
pub fn test_logger() {
    if cfg!(not(feature = "merc_miri")) {
        // Ignore double initialisations in tests since nexttest runs them in parallel.
        let _ = env_logger::builder().is_test(true).try_init();
    }
}

pub fn test_threads<C, F, G>(num_threads: usize, init_function: G, test_function: F)
where
    C: Send + 'static,
    F: Fn(&mut C) + Copy + Send + Sync + 'static,
    G: Fn() -> C,
{
    test_logger();

    let mut threads = vec![];

    for _ in 0..num_threads {
        let mut init = init_function();
        threads.push(std::thread::spawn(move || {
            test_function(&mut init);
        }));
    }

    for thread in threads {
        let _ = thread.join();
    }
}
