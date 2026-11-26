use std::fmt;

/// Displays a byte count with a `bytes`/`KB`/`MB`/`GB` suffix chosen by magnitude.
///
/// Uses decimal units (1 KB = 1000 bytes), not binary (1 KiB = 1024 bytes).
pub struct BytesFormatter(pub usize);

impl fmt::Display for BytesFormatter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const KB: usize = 1_000;
        const MB: usize = 1_000_000;
        const GB: usize = 1_000_000_000;

        // Choose appropriate unit based on size for best readability
        if self.0 >= GB {
            write!(f, "{:.2} GB", self.0 as f64 / GB as f64)
        } else if self.0 >= MB {
            write!(f, "{:.2} MB", self.0 as f64 / MB as f64)
        } else if self.0 >= KB {
            write!(f, "{:.2} KB", self.0 as f64 / KB as f64)
        } else {
            write!(f, "{} bytes", self.0)
        }
    }
}

pub struct LargeFormatter<T: ToString>(pub T);

impl<T: ToString> fmt::Display for LargeFormatter<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let num_str = self.0.to_string();

        // Handle an optional leading '-' sign separately so comma positions are
        // calculated only over the digit characters.
        let (sign, digits) = if let Some(stripped) = num_str.strip_prefix('-') {
            ("-", stripped)
        } else {
            ("", num_str.as_str())
        };

        if !sign.is_empty() {
            write!(f, "{sign}")?;
        }

        let len = digits.len();
        for (i, ch) in digits.chars().enumerate() {
            if i > 0 && (len - i).is_multiple_of(3) {
                write!(f, ",")?;
            }
            write!(f, "{ch}")?;
        }

        Ok(())
    }
}
