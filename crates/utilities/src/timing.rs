use std::cell::RefCell;
use std::collections::HashMap;
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

/// Aggregated timing summary for a named timer.
struct Aggregate {
    name: String,
    min: f32,
    max: f32,
    total: f32,
    avg: f32,
    count: usize,
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

    /// Aggregate results by name and compute (min, max, avg, count, total) for each.
    fn aggregate_results(&self) -> Vec<Aggregate> {
        let mut map: HashMap<String, Aggregate> = HashMap::new();
        for (name, time) in self.results.borrow().iter() {
            map.entry(name.clone())
                .and_modify(|ag| {
                    ag.count += 1;
                    ag.total += *time;
                    ag.min = ag.min.min(*time);
                    ag.max = ag.max.max(*time);
                })
                .or_insert(Aggregate {
                    name: name.clone(),
                    min: *time,
                    max: *time,
                    total: *time,
                    avg: 0.0,
                    count: 1,
                });
        }

        // Compute the averages and sort by name.
        let mut out: Vec<Aggregate> = map
            .into_iter()
            .map(|(_, mut ag)| {
                ag.avg = if ag.count > 0 { ag.total / (ag.count as f32) } else { 0.0 };
                ag
            })
            .collect();

        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Prints all the finished timers aggregated by name (total first; omit metrics when n == 1).
    pub fn print(&self) {
        for ag in self.aggregate_results() {
            if ag.count == 1 {
                eprintln!("Time {}: {:.3}s", ag.name, ag.total);
            } else {
                eprintln!(
                    "Time {}: {:.3}s, min: {:.3}s, max: {:.3}s, avg: {:.3}s, n: {}",
                    ag.name, ag.total, ag.min, ag.max, ag.avg, ag.count
                );
            }
        }
    }

    /// Writes a YAML report of the finished timers to the given writer.
    pub fn print_yaml(&self, tool_name: &str, writer: &mut impl Write) -> io::Result<()> {
        writeln!(writer, "- tool: {tool_name}")?;
        writeln!(writer, "  timing:")?;

        for ag in self.aggregate_results() {
            writeln!(writer, "    {}:", ag.name)?;
            writeln!(writer, "      total: {total:.3}s", total = ag.total)?;
            if ag.count > 1 {
                writeln!(writer, "      count: {}", ag.count)?;
                writeln!(writer, "      min: {min:.3}s", min = ag.min)?;
                writeln!(writer, "      max: {max:.3}s", max = ag.max)?;
                writeln!(writer, "      avg: {avg:.3}s", avg = ag.avg)?;
            }
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
