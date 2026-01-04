use std::fs::File;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;

use merc_io::LargeFormatter;
use merc_ldd::Storage;
use merc_symbolic::SymFormat;
use merc_symbolic::SymbolicLTS;
use merc_symbolic::guess_format_from_extension;
use merc_symbolic::reachability;
use merc_symbolic::read_sylvan;
use merc_symbolic::read_symbolic_lts;
use merc_tools::Version;
use merc_tools::VersionFlag;
use merc_tools::verbosity::VerbosityFlag;
use merc_unsafety::print_allocator_metrics;
use merc_utilities::MercError;
use merc_utilities::Timing;

#[derive(clap::Parser, Debug)]
#[command(
    about = "A command line tool for symbolic labelled transition systems",
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
    Explore(ExploreArgs),
}

#[derive(clap::Args, Debug)]
#[command(about = "Prints information related to the given symbolic LTS")]
struct InfoArgs {
    filename: PathBuf,
}

#[derive(clap::Args, Debug)]
#[command(about = "Explores the given symbolic LTS")]
struct ExploreArgs {
    filename: PathBuf,

    format: Option<SymFormat>,
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
            Commands::Explore(args) => handle_explore(args, &mut timing)?,
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
    let mut storage = Storage::new();

    let mut time_read = timing.start("read_symbolic_lts");
    let lts = read_symbolic_lts(&mut storage, File::open(&args.filename)?)?;
    time_read.finish();

    println!("Symbolic LTS information:");
    println!(
        "  Number of states: {}",
        LargeFormatter(merc_ldd::len(&mut storage, lts.states()))
    );
    println!("  Number of summand groups: {}", lts.transition_groups().len());

    Ok(())
}

/// Explores the given symbolic LTS.
fn handle_explore(args: ExploreArgs, _timing: &mut Timing) -> Result<(), MercError> {
    let mut storage = Storage::new();

    let format = guess_format_from_extension(&args.filename, args.format).ok_or("Cannot determine input format")?;

    let mut file = File::open(&args.filename)?;
    let mut timing = Timing::new();

    match format {
        SymFormat::Sylvan => {
            let mut time_read = timing.start("read_lts");
            let lts = read_sylvan(&mut storage, &mut file)?;
            time_read.finish();

            let mut time_explore = timing.start("explore");
            println!("LTS has {} states", reachability(&mut storage, &lts)?);
            time_explore.finish();
        }
        SymFormat::Sym => {
            let _input = read_symbolic_lts(&mut storage, &mut file)?;
        }
    }

    Ok(())
}
