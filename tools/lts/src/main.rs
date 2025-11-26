use std::fs::File;
use std::io::BufWriter;
use std::io::stdout;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;

use merc_ldd::Storage;
use merc_lts::LTS;
use merc_lts::LtsType;
use merc_lts::guess_format_from_extension;
use merc_lts::is_explicit_lts;
use merc_lts::read_explicit_lts;
use merc_lts::write_aut;
use merc_reduction::Equivalence;
use merc_reduction::reduce;
use merc_symbolic::read_symbolic_lts;
use merc_tools::Version;
use merc_tools::VersionFlag;
use merc_tools::verbosity::VerbosityFlag;
use merc_unsafety::print_allocator_metrics;
use merc_utilities::MercError;
use merc_utilities::Timing;

#[derive(clap::Parser, Debug)]
#[command(
    name = "Maurice Laveaux",
    about = "A command line tool for labelled transition systems"
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
    Reduce(ReduceArgs),
}

#[derive(clap::Args, Debug)]
#[command(about = "Prints information related to the given LTS")]
struct InfoArgs {
    filename: String,
    filetype: Option<LtsType>,
}

#[derive(clap::Args, Debug)]
#[command(about = "Reduces the given explicit LTS modulo an equivalent relation")]
struct ReduceArgs {
    equivalence: Equivalence,

    /// Specify the input LTS.
    filename: String,

    #[arg(long, help = "Explicitly specify the LTS file format")]
    filetype: Option<LtsType>,

    output: Option<String>,

    #[arg(
        short,
        long,
        help = "List of actions that should be considered tau actions",
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

    if cli.version.into() {
        eprintln!("{}", Version);
        return Ok(ExitCode::SUCCESS);
    }

    let mut timing = Timing::new();

    if let Some(command) = cli.commands {
        match command {
            Commands::Info(args) => {
                let path = Path::new(&args.filename);
                let file = File::open(path)?;

                let format = guess_format_from_extension(path, args.filetype).ok_or("Unknown LTS file format.")?;
                if is_explicit_lts(&format) {
                    let lts = read_explicit_lts(path, format, Vec::new(), &mut timing)?;
                    println!("Number of states: {}", lts.num_of_states());
                    println!("Number of transitions: {}", lts.num_of_transitions());
                } else {
                    let mut storage = Storage::new();
                    let lts = read_symbolic_lts(&file, &mut storage)?;
                    println!("Number of states: {}", merc_ldd::len(&mut storage, lts.states()))
                }
            }
            Commands::Reduce(args) => {
                let path = Path::new(&args.filename);
                let format = guess_format_from_extension(path, args.filetype).ok_or("Unknown LTS file format.")?;

                if is_explicit_lts(&format) {
                    let lts = read_explicit_lts(path, format, args.tau.unwrap_or_default(), &mut timing)?;
                    print_allocator_metrics();

                    let reduced_lts = reduce(lts, args.equivalence, &mut timing);

                    if let Some(file) = args.output {
                        let mut writer = BufWriter::new(File::create(file)?);
                        write_aut(&mut writer, &reduced_lts)?;
                    } else {
                        write_aut(&mut stdout(), &reduced_lts)?;
                    }
                } else {
                    return Err("Unsupported file format for reduction.".into());
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
