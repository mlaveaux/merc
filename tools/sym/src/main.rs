use std::fs::File;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;

use merc_ldd::Storage;
use merc_symbolic::read_symbolic_lts;
use merc_tools::Version;
use merc_tools::VersionFlag;
use merc_tools::verbosity::VerbosityFlag;
use merc_unsafety::print_allocator_metrics;
use merc_utilities::LargeFormatter;
use merc_utilities::MercError;
use merc_utilities::Timing;

#[derive(clap::Parser, Debug)]
#[command(
    about = "A command line tool for labelled transition systems",
    arg_required_else_help = true
)]
struct Cli {
    #[command(flatten)]
    version: VersionFlag,

    #[command(flatten)]
    verbosity: VerbosityFlag,

    #[command(subcommand)]
    commands: Option<Commands>,

    #[arg(long, global = true)]
    timings: bool,
}

/// Defines the subcommands for this tool.
#[derive(Debug, Subcommand)]
enum Commands {
    Info(InfoArgs),
}

#[derive(clap::Args, Debug)]
#[command(about = "Prints information related to the given LTS")]
struct InfoArgs {
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

    if let Some(command) = cli.commands {
        match command {
            Commands::Info(args) => handle_info(args, &mut timing)?,
        }
    }

    if cli.timings {
        timing.print();
    }

    print_allocator_metrics();
    Ok(ExitCode::SUCCESS)
}

/// Reads the given symbolic LTS and prints information about it.
fn handle_info(args: InfoArgs, timing: &mut Timing) -> Result<(), MercError> {
    let path = Path::new(&args.filename);
    let mut storage = Storage::new();

    let mut time_read = timing.start("read_symbolic_lts");
    let lts = read_symbolic_lts(File::open(path)?, &mut storage)?;
    time_read.finish();

    println!("Symbolic LTS information:");
    println!("  Number of states: {}", LargeFormatter(merc_ldd::len(&mut storage, lts.states())));
    println!("  Number of summand groups: {}", lts.summand_groups().len());

    Ok(())
}