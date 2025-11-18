use std::ffi::OsStr;
use std::fs::File;
use std::io::BufWriter;
use std::io::stdout;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;

use merc_gui::verbosity::VerbosityFlag;
use merc_ldd::Storage;
use merc_lts::LTS;
use merc_lts::read_aut;
use merc_lts::read_lts;
use merc_lts::write_aut;
use merc_reduction::reduce;

use merc_reduction::Equivalence;
use merc_symbolic::read_symbolic_lts;
use merc_unsafety::print_allocator_metrics;
use merc_utilities::MercError;
use merc_utilities::Timing;
use merc_version::Version;

#[derive(clap::Parser, Debug)]
#[command(name = "Maurice Laveaux", about = "A command line rewriting tool")]
struct Cli {
    #[arg(
        long,
        global = true,
        default_value_t = false,
        help = "Print the version of this tool"
    )]
    version: bool,

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

fn main() -> Result<ExitCode, MercError> {
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
                    println!("Number of states: {}", merc_ldd::len(&mut storage, lts.states()))
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
