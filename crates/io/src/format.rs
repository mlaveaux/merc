use std::fmt;

/// Formats bytes into human-readable format using decimal units (GB, MB, KB, bytes)
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

        // Add spaces every three digits from the right
        let len = num_str.len();
        for (i, ch) in num_str.chars().enumerate() {
            if i > 0 && (len - i) % 3 == 0 {
                write!(f, ",")?;
            }
            write!(f, "{}", ch)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_large_formatter_small_numbers() {
        assert_eq!(format!("{}", LargeFormatter(0)), "0");
        assert_eq!(format!("{}", LargeFormatter(123)), "123");
    }

    #[test]
    fn test_large_formatter_millions() {
        assert_eq!(format!("{}", LargeFormatter(1234567)), "1,234,567");
        assert_eq!(format!("{}", LargeFormatter(12345678)), "12,345,678");
    }
}
