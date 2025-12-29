use std::env;
use std::error::Error;
use std::fs::create_dir;
use std::path::Path;

use duct::cmd;
use which::which_in;

/// Runs various tools for testing purposes. This can be used to test a release
/// package to ensure that basic functionality works fine.
pub fn test_tools(directory: &Path) -> Result<(), Box<dyn Error>> {
    // Create a temporary directory to perform the tests in.
    let tmp_path = Path::new("tmp/");
    if !tmp_path.exists() {
        create_dir(tmp_path)?;
    }

    // Find the binaries
    let merc_lts = which_in("merc-lts", Some(directory), env::current_dir()?)?;

    // Copy some test files to the temporary directory.
    std::fs::copy(directory.join("../examples/lts/abp.aut"), tmp_path.join("abp.aut"))?;

    for algorithm in ["strong-bisim", "branching-bisim", "weak-bisim"] {
        cmd!(
            &merc_lts,
            "reduce",
            algorithm,
            "abp.aut",
            format!("abp.{}.aut", algorithm)
        )
        .dir(tmp_path)
        .run()?;
    }

    Ok(())
}
