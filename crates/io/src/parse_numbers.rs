use std::num::ParseIntError;
use thiserror::Error;

/// Errors that can occur during number parsing.
#[derive(Error, Debug)]
pub enum ParseNumberError {
    #[error("could not read an integer from '{text}'")]
    InvalidInteger { text: String },
    #[error("parse error: {0}")]
    ParseError(#[from] ParseIntError),
}

/// Parses a natural number from a string, skipping leading and trailing whitespace.
///
/// # Examples
/// ```
/// use merc_io::parse_natural_number;
///
/// assert_eq!(parse_natural_number("42").unwrap(), 42);
/// assert_eq!(parse_natural_number("  123  ").unwrap(), 123);
/// assert!(parse_natural_number("abc").is_err());
/// ```
pub fn parse_natural_number(text: &str) -> Result<usize, ParseNumberError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(ParseNumberError::InvalidInteger { text: text.to_string() });
    }

    trimmed.parse::<usize>().map_err(|e| {
        if trimmed.chars().any(|c| !c.is_ascii_digit()) {
            ParseNumberError::InvalidInteger { text: text.to_string() }
        } else {
            e.into()
        }
    })
}

/// Parses a sequence of natural numbers separated by whitespace.
///
/// # Examples
/// ```
/// use merc_io::parse_natural_number_sequence;
///
/// assert_eq!(parse_natural_number_sequence("1 2 3").unwrap(), vec![1, 2, 3]);
/// assert_eq!(parse_natural_number_sequence("  42  123  ").unwrap(), vec![42, 123]);
/// assert!(parse_natural_number_sequence("1 a 3").is_err());
/// ```
pub fn parse_natural_number_sequence(text: &str) -> Result<Vec<usize>, ParseNumberError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(vec![]);
    }

    trimmed.split_whitespace().map(parse_natural_number).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_natural_number() {
        assert_eq!(parse_natural_number("42").unwrap(), 42);
        assert_eq!(parse_natural_number("  123  ").unwrap(), 123);
        assert_eq!(parse_natural_number("0").unwrap(), 0);

        // Error cases
        assert!(parse_natural_number("").is_err());
        assert!(parse_natural_number("abc").is_err());
        assert!(parse_natural_number("12.34").is_err());
        assert!(parse_natural_number("-42").is_err());
    }

    #[test]
    fn test_parse_natural_number_sequence() {
        assert_eq!(parse_natural_number_sequence("1 2 3").unwrap(), vec![1, 2, 3]);
        assert_eq!(parse_natural_number_sequence("  42  123  ").unwrap(), vec![42, 123]);
        assert_eq!(parse_natural_number_sequence("").unwrap(), vec![]);

        // Error cases
        assert!(parse_natural_number_sequence("1 a 3").is_err());
        assert!(parse_natural_number_sequence("1 2.3 4").is_err());
        assert!(parse_natural_number_sequence("1 -2 3").is_err());
    }
}
