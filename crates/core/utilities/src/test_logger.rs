/// Silences the `ena` crate's `unify`/`undo_log` debug! calls.
pub fn silence_ena_logging(builder: &mut env_logger::Builder) -> &mut env_logger::Builder {
    builder.filter_module("ena", log::LevelFilter::Off)
}

/// Initializes the standard logger for tests. Output is routed through the test
/// harness so it is captured rather than printed unconditionally.
pub fn test_logger() {
    if !cfg!(miri) {
        // Ignore double initialisations in tests since tests are ran in parallel.
        let mut builder = env_logger::builder();
        builder.is_test(true);
        silence_ena_logging(&mut builder);
        let _ = builder.try_init();
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

    // Propagate a worker panic so a failing assertion fails the test instead of
    // being silently swallowed.
    for thread in threads {
        thread.join().expect("The logger thread panicked");
    }
}
