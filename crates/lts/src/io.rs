use std::ffi::OsStr;
use std::path::Path;

use clap::ValueEnum;
use merc_utilities::MercError;
use merc_utilities::Timing;

use crate::LabelledTransitionSystem;
use crate::MultiAction;
use crate::read_aut;
use crate::read_lts;

/// Explicitly specify the LTS file format.
#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum LtsFormat {
    Aut,
    Lts,
}

/// Guesses the LTS file format from the file extension.
pub fn guess_lts_format_from_extension(path: &Path, format: Option<LtsFormat>) -> Option<LtsFormat> {
    if let Some(format) = format {
        return Some(format);
    }

    if path.extension() == Some(OsStr::new("aut")) {
        Some(LtsFormat::Aut)
    } else if path.extension() == Some(OsStr::new("lts")) {
        Some(LtsFormat::Lts)
    } else {
        None
    }
}

/// A helper struct to deal with the polymorphic LTS types.
pub enum GenericLts {
    /// The LTS in the Aldebaran format.
    Aut(LabelledTransitionSystem<String>),
    /// The LTS in the mCRL2 .lts format.
    Lts(LabelledTransitionSystem<MultiAction>),
}

/// Reads an explicit labelled transition system from the given path and format.
pub fn read_explicit_lts(
    path: &Path,
    format: LtsFormat,
    hidden_labels: Vec<String>,
    timing: &mut Timing,
) -> Result<GenericLts, MercError> {
    let file = std::fs::File::open(path)?;
    let mut time_read = timing.start("read_aut");

    let result = match format {
        LtsFormat::Aut => GenericLts::Aut(read_aut(&file, hidden_labels)?),
        LtsFormat::Lts => GenericLts::Lts(read_lts(&file, hidden_labels)?),
        LtsFormat::Sym => {
            panic!("Cannot read symbolic LTS as explicit LTS.")
        }
    };

    time_read.finish();
    Ok(result)
}
