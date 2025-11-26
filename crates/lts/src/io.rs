use std::ffi::OsStr;
use std::path::Path;

use clap::ValueEnum;
use merc_utilities::MercError;

use crate::LabelledTransitionSystem;
use crate::read_aut;
use crate::read_lts;

/// Explicitly specify the LTS file format.
#[derive(Clone, Debug, ValueEnum)]
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

/// Returns true iff the given filename is an explicit LTS file based on its extension or the provided format.
pub fn is_explicit_lts(format: &LtsType) -> bool {
    match format {
        LtsType::Aut | LtsType::Lts => true,
        LtsType::Sym => false,
    }
}

/// Returns true iff the given filename is a symbolic LTS file based on its extension or the provided format.
pub fn is_symbolic_lts(format: &LtsType) -> bool {
    match format {
        LtsType::Aut | LtsType::Lts => false,
        LtsType::Sym => true,
    }
}

/// Reads an explicit labelled transition system from the given path and format.
pub fn read_explicit_lts(
    path: &Path,
    format: LtsType,
    hidden_labels: Vec<String>,
) -> Result<LabelledTransitionSystem, MercError> {
    let file = std::fs::File::open(path)?;

    match format {
        LtsType::Aut => read_aut(&file, hidden_labels),
        LtsType::Lts => read_lts(&file, hidden_labels),
        LtsType::Sym => {
            panic!("Cannot read symbolic LTS as explicit LTS.")
        }
    }
}
