use std::cell::RefCell;
use std::io;
use std::io::Write;
use std::rc::Rc;
use std::time::Instant;

use log::info;
use log::warn;

/// A timing object to measure the time of different parts of the program. This
/// is useful for debugging and profiling.
#[derive(Default)]
pub struct Timing {
    results: Rc<RefCell<Vec<(String, f32)>>>,
}

/// A timer object that measures the time between its creation and the call to
/// `finish()`. Finish should be called explicitly before the timer is dropped,
/// otherwise we get zero values since the timer object is unused and can be
/// immediately dropped.
pub struct Timer {
    name: String,
    start: Instant,
    results: Rc<RefCell<Vec<(String, f32)>>>,
    registered: bool,
}

impl Timing {
    /// Creates a new timing object to track timers.
    pub fn new() -> Self {
        Self {
            results: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Starts a new timer with the given name.
    pub fn start(&self, name: &str) -> Timer {
        Timer {
            name: name.to_string(),
            start: Instant::now(),
            results: self.results.clone(),
            registered: false,
        }
    }

    /// Prints all the finished timers.
    pub fn print(&self) {
        for (name, time) in self.results.borrow().iter() {
            eprintln!("Time {name}: {time:.3}s");
        }
    }

    /// Writes a YAML report of the finished timers to the given writer.
    pub fn print_yaml(&self, tool_name: &str, writer: &mut impl Write) -> io::Result<()> {
        writeln!(writer, "- tool: {tool_name}")?;
        writeln!(writer, "  timing:")?;

        for (name, time) in self.results.borrow().iter() {
            writeln!(writer, "    {name}: {time:.3}s")?;
        }
        Ok(())
    }
}

impl Timer {
    /// Finishes the timer and registers the result.
    pub fn finish(&mut self) {
        let time = self.start.elapsed().as_secs_f64();
        info!("Time {}: {:.3}s", self.name, time);

        // Register the result.
        self.results.borrow_mut().push((self.name.clone(), time as f32));
        self.registered = true
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if !self.registered {
            warn!("Timer {} was dropped before 'finish()'", self.name);
        }
    }
}
