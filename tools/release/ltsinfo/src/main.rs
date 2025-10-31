use std::ffi::OsStr;
use std::fs::File;
use std::io::BufWriter;
use std::io::stdout;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;

use mcrl3_gui::verbosity::Verbosity;
use mcrl3_ldd::Storage;
use mcrl3_lts::read_aut;
use mcrl3_lts::read_lts;
use mcrl3_lts::write_aut;
use mcrl3_reduction::reduce;

use mcrl3_reduction::Equivalence;
use mcrl3_symbolic::read_symbolic_lts;
use mcrl3_unsafety::print_allocator_metrics;
use mcrl3_utilities::MCRL3Error;
use mcrl3_utilities::Timing;
use mcrl3_version::Version;

#[derive(clap::Parser, Debug)]
#[command(name = "Maurice Laveaux", about = "A command line rewriting tool")]
struct Cli {
    #[arg(long, default_value_t = false, help = "Print the version of this tool")]
    version: bool,

    #[arg(short, long, default_value_t = Verbosity::Quiet, help = "Sets the verbosity of the logger")]
    verbosity: Verbosity,

    #[command(subcommand)]
    commands: Option<Commands>,

    #[arg(long)]
    timings: bool,
}

/// Defines the subcommands for this tool.
#[derive(Debug, Subcommand)]
enum Commands {
    Info(InfoArgs),
    Reduce(ReduceArgs),
}

#[derive(clap::Args, Debug)]
#[command(about = "Prints information related to the given LTS")]
struct InfoArgs {
    filename: String,
}

#[derive(clap::Args, Debug)]
#[command(about = "Reduces the given explicit LTS modulo an equivalent relation")]
struct ReduceArgs {
    equivalence: Equivalence,

    filename: String,

    output: Option<String>,

    #[arg(
        short,
        long,
        help = "List of actions that are considered tau actions",
        value_delimiter = ','
    )]
    tau: Option<Vec<String>>,
}

fn main() -> Result<ExitCode, MCRL3Error> {
    let cli = Cli::parse();

    env_logger::Builder::new()
        .filter_level(cli.verbosity.log_level_filter())
        .parse_default_env()
        .init();

    if cli.version {
        eprintln!("{}", Version);
        return Ok(ExitCode::SUCCESS);
    }

    let mut timing = Timing::new();

    if let Some(command) = cli.commands {
        match command {
            Commands::Info(args) => {
                let path = Path::new(&args.filename);
                let file = File::open(path)?;

                if path.extension() == Some(OsStr::new("aut")) {
                    let lts = read_aut(&file, Vec::new())?;
                    println!("Number of states: {}", lts.num_of_states())
                } else if path.extension() == Some(OsStr::new("lts")) {
                    let lts = read_lts(&file)?;
                    println!("Number of states: {}", lts.num_of_states())
                } else if path.extension() == Some(OsStr::new("sym")) {
                    let mut storage = Storage::new();
                    let lts = read_symbolic_lts(&file, &mut storage)?;
                    println!("Number of states: {}", mcrl3_ldd::len(&mut storage, lts.states()))
                } else {
                    return Err("Unsupported LTS file format.".into());
                }
            }
            Commands::Reduce(args) => {
                let path = Path::new(&args.filename);
                let file = File::open(path)?;

                if path.extension() == Some(OsStr::new("aut")) {
                    let lts = read_aut(&file, args.tau.unwrap_or_default())?;
                    print_allocator_metrics();

                    let reduced_lts = reduce(lts, args.equivalence, &mut timing);

                    if let Some(file) = args.output {
                        let mut writer = BufWriter::new(File::create(file)?);
                        write_aut(&mut writer, &reduced_lts)?;
                    } else {
                        write_aut(&mut stdout(), &reduced_lts)?;
                    }
                } else if path.extension() == Some(OsStr::new("sym")) {
                    let mut storage = Storage::new();
                    let _lts = read_symbolic_lts(&file, &mut storage)?;
                } else {
                    return Err("Unsupported file format for LTS reduce.".into());
                }
            }
        }
    }

    if cli.timings {
        timing.print();
    }

    print_allocator_metrics();
    Ok(ExitCode::SUCCESS)
}
