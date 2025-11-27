use std::ffi::OsStr;
use std::path::Path;

use clap::ValueEnum;
use merc_utilities::MercError;
use merc_utilities::Timing;

use crate::LabelledTransitionSystem;
use crate::read_aut;
use crate::read_lts;

/// Explicitly specify the LTS file format.
#[derive(Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum LtsType {
    Aut,
    Lts,
    Sym,
}

/// Guesses the LTS file format from the file extension.
pub fn guess_format_from_extension(path: &Path, format: Option<LtsType>) -> Option<LtsType> {
    if let Some(format) = format {
        return Some(format);
    }

    if path.extension() == Some(OsStr::new("aut")) {
        Some(LtsType::Aut)
    } else if path.extension() == Some(OsStr::new("lts")) {
        Some(LtsType::Lts)
    } else if path.extension() == Some(OsStr::new("sym")) {
        Some(LtsType::Sym)
    } else {
        None
    }
}

/// Reads an explicit labelled transition system from the given path and format.
pub fn read_explicit_lts(
    path: &Path,
    format: LtsType,
    hidden_labels: Vec<String>,
    timing: &mut Timing,
) -> Result<LabelledTransitionSystem, MercError> {
    assert!(format != LtsType::Sym, "Cannot read symbolic LTS as explicit LTS.");

    let file = std::fs::File::open(path)?;
    let mut time_read = timing.start("read_aut");

    let result = match format {
        LtsType::Aut => read_aut(&file, hidden_labels),
        LtsType::Lts => read_lts(&file, hidden_labels),
        LtsType::Sym => {
            panic!("Cannot read symbolic LTS as explicit LTS.")
        }
    };

    time_read.finish();
    result
}
