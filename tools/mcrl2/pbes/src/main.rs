use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;

use merc_tools::VerbosityFlag;
use merc_tools::Version;
use merc_tools::VersionFlag;
use merc_utilities::MercError;
use merc_utilities::Timing;

#[derive(clap::Parser, Debug)]
#[command(about = "A command line tool for variability parity games", arg_required_else_help = true)]
struct Cli {
    #[command(flatten)]
    version: VersionFlag,

    #[command(flatten)]
    verbosity: VerbosityFlag,

    #[arg(long, global = true)]
    timings: bool,

    #[command(subcommand)]
    commands: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Symmetry(SymmetryArgs),
}

/// Arguments for solving a parity game
#[derive(clap::Args, Debug)]
struct SymmetryArgs {
    filename: String,
}

fn main() -> Result<ExitCode, MercError> {
    let cli = Cli::parse();

    env_logger::Builder::new()
        .filter_level(cli.verbosity.log_level_filter())
        .parse_default_env()
        .init();

    if cli.version.into() {
        eprintln!("{}", Version);
        return Ok(ExitCode::SUCCESS);
    }

    let mut timing = Timing::new();

    if let Some(Commands::Symmetry(args)) = cli.commands {
        let pbes = mcrl2_sys::pbes::ffi::load_pbes_from_file(&args.filename)?;

        let result = mcrl2_sys::pbes::ffi::run_stategraph_local_algorithm(&pbes)?;
    }

    
    if cli.timings {
        timing.print();
    }

    Ok(ExitCode::SUCCESS)
}