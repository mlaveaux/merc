//!
//! A utility function to easily print progress information for procedures that
//! take a fixed number of steps. In particular, avoids writing too many
//! progress indications.
//!

use std::cell::RefCell;
use std::marker::PhantomData;
use std::time::Duration;
use std::time::Instant;

/// A time-based progress tracker that prints messages at regular intervals.
pub struct TimeProgress<T> {
    interval: Duration,
    last_update: RefCell<Instant>,
    message: Box<dyn Fn(T)>,
    _marker: PhantomData<T>,
}

impl<T> TimeProgress<T> {
    /// Create a new time-based progress tracker with a given interval in seconds.
    pub fn new(message: impl Fn(T) + 'static, interval_seconds: u64) -> TimeProgress<T> {
        TimeProgress {
            message: Box::new(message),
            interval: Duration::from_secs(interval_seconds),
            last_update: RefCell::new(Instant::now()),
            _marker: PhantomData,
        }
    }

    /// Increase the progress with the given amount, prints periodic progress
    /// messages based on time intervals.
    pub fn print(&self, object: T) {
        let now = Instant::now();
        let should_print = {
            let last = *self.last_update.borrow();
            now.duration_since(last) >= self.interval
        };
        if should_print {
            (self.message)(object);
            *self.last_update.borrow_mut() = now;
        }
    }
}
