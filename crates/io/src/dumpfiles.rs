use std::fs::File;
use std::path::Path;
use std::path::PathBuf;

use log::info;
use merc_utilities::MercError;

/// A utility for dumping files, mostly used for testing and debugging
///
/// # Details
///
/// The given name is used to create a dedicated directory for the output files,
/// this is especially useful for files dumped from (random) tests.
///
/// Uses the `MERC_DUMP=1` environment variable to enable or disable dumping files
/// to disk, to avoid unnecessary writes during normal runs. In combination with
/// `MERC_SEED` we can reproduce specific tests cases for random runs.
pub struct DumpFiles {
    // None when dumping is disabled.
    directory: Option<PathBuf>,
}

impl DumpFiles {
    /// Creates a new `DumpFiles` instance with the given directory as output.
    pub fn new(directory: &str) -> Self {
        if let Ok(dump_dir) = std::env::var("MERC_DUMP") {
            // Check if the directory is an absolute path
            if !Path::new(dump_dir.as_str()).is_absolute() {
                panic!("MERC_DUMP must be an absolute path, because tests write relative to their source file.");
            }

            Self {
                directory: Some(Path::new(&dump_dir).join(directory)),
            }
        } else {
            // Dumping disabled.
            Self { directory: None }
        }
    }

    /// Dumps a file with the given filename suffix by calling the provided function
    /// to write the contents.
    pub fn dump<F>(&mut self, filename: &str, mut write: F) -> Result<(), MercError>
    where
        F: FnMut(&mut File) -> Result<(), MercError>,
    {
        if let Some(directory) = &self.directory {
            // Ensure the dump directory exists.
            let _ = std::fs::create_dir_all(directory);

            let path = Path::new(&directory).join(filename);
            let mut file = File::create(&path)?;
            write(&mut file)?;

            info!("Dumped file: {}", path.to_string_lossy());
        } else {
            info!("No MERC_DUMP set, skipping dump: {}", filename);
        }
        Ok(())
    }
}
