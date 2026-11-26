use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

/// A time-based progress tracker that prints a message at most once per interval,
/// so a long-running procedure with many steps does not flood the log with
/// progress indications.
pub struct TimeProgress<T> {
    interval: Duration,
    /// Reference instant against which `last_update` is measured.
    start: Instant,
    /// Nanoseconds since `start` at the last printed update.
    last_update: AtomicU64,
    message: Box<dyn Fn(T) + Send + Sync>,
}

impl<T> TimeProgress<T> {
    /// Create a new time-based progress tracker with a given interval in seconds.
    pub fn new<F>(message: F, interval_seconds: u64) -> TimeProgress<T>
    where
        F: Fn(T) + Send + Sync + 'static,
    {
        TimeProgress {
            message: Box::new(message),
            interval: Duration::from_secs(interval_seconds),
            start: Instant::now(),
            last_update: AtomicU64::new(0),
        }
    }

    /// Passes `object` to the message closure if the interval has elapsed since
    /// the last update, otherwise does nothing.
    ///
    /// When called concurrently from several threads, a compare-exchange ensures
    /// exactly one caller claims the interval and emits the message, so no
    /// duplicate lines are printed.
    pub fn print(&self, object: T) {
        let elapsed = self.elapsed_nanos();
        let last = self.last_update.load(Ordering::Relaxed);
        if elapsed.saturating_sub(last) >= self.interval_nanos()
            && self
                .last_update
                .compare_exchange(last, elapsed, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            (self.message)(object);
        }
    }

    /// Returns true iff the progress tracker is due for an update.
    pub fn is_due(&self) -> bool {
        self.elapsed_nanos()
            .saturating_sub(self.last_update.load(Ordering::Relaxed))
            >= self.interval_nanos()
    }

    /// Resets the progress timer.
    pub fn reset(&self) {
        self.last_update.store(self.elapsed_nanos(), Ordering::Relaxed);
    }

    /// Nanoseconds elapsed since `start`, saturating at `u64::MAX` (~584 years).
    fn elapsed_nanos(&self) -> u64 {
        self.start.elapsed().as_nanos().min(u64::MAX as u128) as u64
    }

    /// The configured interval expressed in nanoseconds.
    fn interval_nanos(&self) -> u64 {
        self.interval.as_nanos().min(u64::MAX as u128) as u64
    }
}
