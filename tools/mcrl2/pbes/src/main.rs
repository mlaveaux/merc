use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;
use log::info;

use mcrl2::Pbes;
use mcrl2::set_reporting_level;
use mcrl2::verbosity_to_log_level_t;
use merc_tools::VerbosityFlag;
use merc_tools::Version;
use merc_tools::VersionFlag;
use merc_utilities::MercError;
use merc_utilities::Timing;

use crate::permutation::Permutation;
use crate::symmetry::SymmetryAlgorithm;

mod clone_iterator;
mod permutation;
mod symmetry;

#[derive(clap::ValueEnum, Clone, Debug)]
enum PbesFormat {
    Text,
    Pbes,
}

#[derive(clap::Parser, Debug)]
#[command(
    about = "A command line tool for parameterised boolean equation systems (PBESs)",
    arg_required_else_help = true
)]
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

    #[arg(long, short('i'), value_enum)]
    format: Option<PbesFormat>,

    /// Pass a single permutation in cycles notation to check whether it is a symmetry.
    #[arg(long)]
    permutation: Option<String>,

    /// Search for all symmetries instead of only the first one.    
    #[arg(long, default_value_t = false)]
    all_symmetries: bool,

    /// Partition data parameters into their sorts before considering their permutation groups.
    #[arg(long, default_value_t = false)]
    partition_data_sorts: bool,
    
    /// Partition data parameters based on their updates.
    #[arg(long, default_value_t = false)]
    partition_data_updates: bool,
}

fn main() -> Result<ExitCode, MercError> {
    let cli = Cli::parse();

    env_logger::Builder::new()
        .filter_level(cli.verbosity.log_level_filter())
        .parse_default_env()
        .init();

    // Enable logging on the mCRL2 side
    set_reporting_level(verbosity_to_log_level_t(cli.verbosity.verbosity()));

    if cli.version.into() {
        eprintln!("{}", Version);
        return Ok(ExitCode::SUCCESS);
    }

    let timing = Timing::new();

    if let Some(Commands::Symmetry(args)) = cli.commands {
        let format = args.format.unwrap_or(PbesFormat::Pbes);

        let pbes = match format {
            PbesFormat::Pbes => Pbes::from_file(&args.filename)?,
            PbesFormat::Text => Pbes::from_text_file(&args.filename)?,
        };

        let algorithm = SymmetryAlgorithm::new(&pbes, false)?;
        if let Some(permutation) = &args.permutation {
            let pi = if permutation.trim_start().starts_with("[") {
                Permutation::from_mapping_notation(permutation)?
            } else {
                Permutation::from_cycle_notation(permutation)?
            };

            if let Err(x) = algorithm.is_valid_permutation(&pi) {
                info!("The given permutation is not valid: {x}");
                return Ok(ExitCode::FAILURE);
            }

            info!("Checking permutation: {}", pi);
            if algorithm.check_symmetry(&pi) {
                println!("true");
            } else {
                println!("false");
            }
        } else {
            for candidate in algorithm.candidates(args.partition_data_sorts, args.partition_data_updates) {
                info!("Found candidate: {}", candidate);

                if algorithm.check_symmetry(&candidate) {
                    info!("Found symmetry: {}", candidate);

                    if !args.all_symmetries && !candidate.is_identity() {
                        // Only search for the first symmetry
                        info!("Stopping search after first non-trivial symmetry.");
                        break;
                    }
                }
            }
        }
    }

    if cli.timings {
        timing.print();
    }

    Ok(ExitCode::SUCCESS)
}
