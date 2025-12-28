//!
//! `xtask` is a crate that can be used to enable `make`-like commands in cargo. These commands are then implemented in Rust.
//!

#![forbid(unsafe_code)]

use std::error::Error;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;
use std::path::PathBuf;

mod coverage;
mod discover_tests;
mod package;
mod publish;
mod sanitizer;
mod tool_testing;

#[derive(Parser)]
#[command(name = "xtask")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generates a code coverage report using grcov.
    Coverage {
        #[clap(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Runs the given cargo command with AddressSanitizer enabled.
    AddressSanitizer {
        #[clap(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Runs the given cargo command with ThreadSanitizer enabled.
    ThreadSanitizer {
        #[clap(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Discovers tests from the examples folder, and prints them as a `#[test_case]` annotation.
    DiscoverTests,
    /// Builds and packages the binaries for release.
    Package,
    /// Publishes the crates to crates.io.
    Publish,
    TestTools {
        directory: PathBuf,
    },
}

fn main() -> Result<ExitCode, Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Coverage { args } => coverage::coverage(args)?,
        Commands::AddressSanitizer { args } => sanitizer::address_sanitizer(args)?,
        Commands::ThreadSanitizer { args } => sanitizer::thread_sanitizer(args)?,
        Commands::DiscoverTests => discover_tests::discover_tests()?,
        Commands::Package => package::package()?,
        Commands::Publish => publish::publish_crates(),
        Commands::TestTools { directory } => tool_testing::test_tools(directory.as_path())?,
    }

    Ok(ExitCode::SUCCESS)
}
