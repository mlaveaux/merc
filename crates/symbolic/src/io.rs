use std::ffi::OsStr;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum SymFormat {
    /// The mCRL2 symbolic format
    Sym,
    /// The Sylvan binary format
    Sylvan,
}

/// Guesses the symbolic LTS file format from the file extension. Returns None if it cannot be determined.
pub fn guess_format_from_extension(path: &Path, format: Option<SymFormat>) -> Option<SymFormat> {
    if let Some(format) = format {
        return Some(format);
    }

    if path.extension() == Some(OsStr::new("ldd")) {
        Some(SymFormat::Sylvan)
    } else if path.extension() == Some(OsStr::new("sym")) {
        Some(SymFormat::Sym)
    } else {
        None
    }
}
