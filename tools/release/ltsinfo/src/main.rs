use std::ffi::OsStr;
use std::fs::File;
use std::io::BufWriter;
use std::io::stdout;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;

use mcrl3_gui::verbosity::Verbosity;
use mcrl3_ldd::Storage;
use mcrl3_lts::read_aut;
use mcrl3_lts::write_aut;
use mcrl3_reduction::branching_bisim_sigref;
use mcrl3_reduction::branching_bisim_sigref_naive;
use mcrl3_reduction::quotient_lts;
use mcrl3_reduction::strong_bisim_sigref;
use mcrl3_reduction::strong_bisim_sigref_naive;

use mcrl3_symbolic::read_symbolic_lts;
use mcrl3_unsafety::print_allocator_metrics;
use mcrl3_utilities::MCRL3Error;
use mcrl3_utilities::Timing;
use mcrl3_version::Version;

#[derive(Clone, Debug, ValueEnum)]
enum Equivalence {
    StrongBisim,
    StrongBisimNaive,
    BranchingBisim,
    BranchingBisimNaive,
}

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

    #[arg(short, long, help="List of actions that are considered tau actions", value_delimiter = ',')]
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
                } else if path.extension() == Some(OsStr::new("sym"))  {
                    let mut storage = Storage::new();
                    let lts = read_symbolic_lts(&file, &mut storage)?;
                    println!("Number of states: {}", mcrl3_ldd::len(&mut storage, lts.states()))
                } else {
                    return Err("Unsupported LTS file format.".into());                    
                }
            },
            Commands::Reduce(args) => {
                let path = Path::new(&args.filename);
                let file = File::open(path)?;

                if  path.extension() == Some(OsStr::new("aut")) {
                    let lts = read_aut(&file, args.tau.unwrap_or_default())?;
                    print_allocator_metrics();

                    let (preprocessed_lts, partition) = match args.equivalence {
                        Equivalence::StrongBisim => strong_bisim_sigref(lts, &mut timing),
                        Equivalence::StrongBisimNaive => strong_bisim_sigref_naive(lts, &mut timing),
                        Equivalence::BranchingBisim => branching_bisim_sigref(lts, &mut timing),
                        Equivalence::BranchingBisimNaive => branching_bisim_sigref_naive(lts, &mut timing),
                    };

                    let mut quotient_time = timing.start("quotient");
                    let quotient_lts = quotient_lts(
                        &preprocessed_lts,
                        &partition,
                        matches!(args.equivalence, Equivalence::BranchingBisim)
                            || matches!(args.equivalence, Equivalence::BranchingBisimNaive),
                    );
                    if let Some(file) = args.output {
                        let mut writer = BufWriter::new(File::create(file)?);
                        write_aut(&mut writer, &quotient_lts)?;
                    } else {
                        write_aut(&mut stdout(), &quotient_lts)?;
                    }

                    quotient_time.finish();
                } else if path.extension() == Some(OsStr::new("sym"))  {
                    let mut storage = Storage::new();
                    let lts = read_symbolic_lts(&file, &mut storage)?;
                    
                } else {
                    return Err("Unsupported file format for LTS reduce.".into());                    
                }
            },
        }
    }

    if cli.timings {
        timing.print();
    }

    print_allocator_metrics();
    Ok(ExitCode::SUCCESS)
}
