use std::ffi::OsStr;
use std::path::Path;

use clap::ValueEnum;

/// Specify the parity game file format.
#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum ParityGameFormat {
    PG,
    VPG,
}

/// Guesses the parity game file format from the file extension, or uses a fixed format if provided.
pub fn guess_format_from_extension(path: &Path, format: Option<ParityGameFormat>) -> Option<ParityGameFormat> {
    if let Some(format) = format {
        return Some(format);
    }

    if path.extension() == Some(OsStr::new("pg")) {
        Some(ParityGameFormat::PG)
    } else if path.extension() == Some(OsStr::new("vpg")) || path.extension() == Some(OsStr::new("svpg")) {
        Some(ParityGameFormat::VPG)
    } else {
        None
    }
}
