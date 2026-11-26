//! A mixed-radix counter that enumerates a Cartesian product of index ranges.

/// Enumerates every combination of digits `d` with `d[k] < radices[k]`, in which the last digit changes fastest.
///
/// Unlike [`itertools::Itertools::multi_cartesian_product`], the combination is updated in place, and every step reports
/// the first digit that changed, so that a caller can update only what depends on the changed digits.
#[derive(Clone, Debug, Default)]
pub struct Odometer {
    digits: Vec<usize>,
    radices: Vec<usize>,
}

impl Odometer {
    /// Creates an odometer with no digits, which has exactly one (empty) combination.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets the odometer to the all-zero combination for the given radices, reusing the allocated buffers.
    ///
    /// Returns false iff a radix is zero, in which case there are no combinations.
    pub fn reset(&mut self, radices: impl IntoIterator<Item = usize>) -> bool {
        self.radices.clear();
        self.radices.extend(radices);
        self.digits.clear();
        self.digits.resize(self.radices.len(), 0);
        !self.radices.contains(&0)
    }

    /// Returns the current combination.
    pub fn digits(&self) -> &[usize] {
        &self.digits
    }

    /// Advances to the next combination, and returns the first digit that changed. All digits after it are reset to
    /// zero. Returns `None` after the last combination, at which point the odometer is back at all zeros.
    pub fn increment(&mut self) -> Option<usize> {
        for k in (0..self.digits.len()).rev() {
            self.digits[k] += 1;
            if self.digits[k] < self.radices[k] {
                return Some(k);
            }
            self.digits[k] = 0;
        }
        None
    }
}
