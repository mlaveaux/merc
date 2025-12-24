use std::fmt;

use itertools::Itertools;
use oxidd::bdd::BDDFunction;
use oxidd::util::OptBool;

use crate::CubeIter;

/// A helper structure to format configuration sets for output.
pub struct FormatConfigSet<'a>(pub &'a BDDFunction);

impl fmt::Display for FormatConfigSet<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            CubeIter::new(&self.0).format_with("+", |cube, fmt| { fmt(&format_args!("{}", FormatConfig(&cube))) })
        )
    }
}

/// A helper structure to format a configuration for output.
pub struct FormatConfig<'a>(pub &'a Vec<OptBool>);

impl fmt::Display for FormatConfig<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for value in self.0 {
            match value {
                OptBool::True => write!(f, "1")?,
                OptBool::False => write!(f, "0")?,
                OptBool::None => write!(f, "-")?,
            }
        }

        Ok(())
    }
}
