//!
//! A utility function to easily print progress information for procedures that
//! take a fixed number of steps. In particular, avoids writing too many
//! progress indications.
//!

use std::marker::PhantomData;
use std::time::Duration;
use std::time::Instant;

/// The struct that can be initialised to keep track of progress counters.
pub struct Progress<F: Fn(usize, usize)> {
    maximum: usize,
    counter: usize,

    message: F,
}

impl<F: Fn(usize, usize)> Progress<F> {
    /// Create a new progress tracker with a given maximum.
    pub fn new(message: F, maximum: usize) -> Progress<F> {
        Progress {
            message,
            maximum,
            counter: 0,
        }
    }

    /// Increase the progress with the given amount, prints the message when 1% progress has been made.
    pub fn add(&mut self, amount: usize) {
        let increment = (self.maximum / 100usize).max(1);

        if (self.counter + amount) / increment > self.counter / increment {
            // Print a progress message when the increment increased.
            (self.message)(self.counter, increment);
        }

        self.counter += amount;
    }
}

/// A time-based progress tracker that prints messages at regular intervals.
pub struct TimeProgress<F: Fn(T), T> {
    interval: Duration,
    last_update: Instant,
    message: F,
    _marker: PhantomData<T>,
}

impl<F: Fn(T), T> TimeProgress<F, T> {
    /// Create a new time-based progress tracker with a given interval in seconds.
    pub fn new(message: F, interval_seconds: u64) -> TimeProgress<F, T> {
        TimeProgress {
            message,
            interval: Duration::from_secs(interval_seconds),
            last_update: Instant::now(),
            _marker: PhantomData,
        }
    }

    /// Increase the progress with the given amount, prints periodic progress
    /// messages based on time intervals.
    pub fn print(&mut self, object: T) {
        let now = Instant::now();
        if now.duration_since(self.last_update) >= self.interval {
            (self.message)(object);
            self.last_update = now;
        }
    }
}
