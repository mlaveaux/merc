use std::fs::File;
use std::io::stdout;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;
use log::info;

use merc_lts::LTS;
use merc_lts::LtsFormat;
use merc_lts::guess_format_from_extension;
use merc_lts::read_explicit_lts;
use merc_lts::write_aut;
use merc_reduction::Equivalence;
use merc_reduction::reduce_lts;
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
    Reduce(ReduceArgs),
    Compare(CompareArgs),
}

#[derive(clap::Args, Debug)]
#[command(about = "Prints information related to the given LTS")]
struct InfoArgs {
    filename: String,
    filetype: Option<LtsFormat>,
}

#[derive(clap::Args, Debug)]
#[command(about = "Reduces the given explicit LTS modulo an equivalent relation")]
struct ReduceArgs {
    equivalence: Equivalence,

    /// Specify the input LTS.
    filename: String,

    #[arg(long, help = "Explicitly specify the LTS file format")]
    filetype: Option<LtsFormat>,

    output: Option<String>,

    #[arg(
        short,
        long,
        help = "List of actions that should be considered tau actions",
        value_delimiter = ','
    )]
    tau: Option<Vec<String>>,
}

#[derive(clap::Args, Debug)]
#[command(about = "Reduces the given explicit LTS modulo an equivalent relation")]
struct CompareArgs {
    equivalence: Equivalence,

    /// Specify the input LTS.
    left_filename: String,

    /// Specify the input LTS.
    right_filename: String,

    #[arg(long, help = "Explicitly specify the LTS file format")]
    filetype: Option<LtsFormat>,

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

                let format = guess_format_from_extension(path, args.filetype).ok_or("Unknown LTS file format.")?;
                if format != LtsFormat::Sym {
                    let lts = read_explicit_lts(path, format, Vec::new(), &mut timing)?;
                    println!(
                        "LTS has {} states and {} transitions.",
                        LargeFormatter(lts.num_of_states()),
                        LargeFormatter(lts.num_of_transitions())
                    );
                } else {
                    return Err("Unsupported file format for info.".into());
                }
            }
            Commands::Reduce(args) => {
                let path = Path::new(&args.filename);
                let format = guess_format_from_extension(path, args.filetype).ok_or("Unknown LTS file format.")?;

                if format != LtsFormat::Sym {
                    let lts = read_explicit_lts(path, format, args.tau.unwrap_or_default(), &mut timing)?;
                    info!(
                        "LTS has {} states and {} transitions.",
                        LargeFormatter(lts.num_of_states()),
                        LargeFormatter(lts.num_of_transitions())
                    );

                    print_allocator_metrics();

                    let reduced_lts = reduce_lts(lts, args.equivalence, &mut timing);
                    info!(
                        "Reduced LTS has {} states and {} transitions.",
                        LargeFormatter(reduced_lts.num_of_states()),
                        LargeFormatter(reduced_lts.num_of_transitions())
                    );

                    if let Some(file) = args.output {
                        let mut writer = File::create(file)?;
                        write_aut(&mut writer, &reduced_lts)?;
                    } else {
                        write_aut(&mut stdout(), &reduced_lts)?;
                    }
                } else {
                    return Err("Unsupported file format for reduction.".into());
                }
            }
            Commands::Compare(args) => {
                let left_path = Path::new(&args.left_filename);
                let right_path = Path::new(&args.right_filename);
                let format = guess_format_from_extension(left_path, args.filetype).ok_or("Unknown LTS file format.")?;

                info!("Assuming format {:?} for both LTSs.", format);

                if format != LtsFormat::Sym {
                    let left_lts =
                        read_explicit_lts(left_path, format, args.tau.clone().unwrap_or_default(), &mut timing)?;
                    let right_lts = read_explicit_lts(right_path, format, args.tau.unwrap_or_default(), &mut timing)?;

                    info!(
                        "Left LTS has {} states and {} transitions.",
                        LargeFormatter(left_lts.num_of_states()),
                        LargeFormatter(left_lts.num_of_transitions())
                    );
                    info!(
                        "Right LTS has {} states and {} transitions.",
                        LargeFormatter(right_lts.num_of_states()),
                        LargeFormatter(right_lts.num_of_transitions())
                    );

                    print_allocator_metrics();

                    let equivalent = merc_reduction::compare_lts(args.equivalence, left_lts, &right_lts, &mut timing);
                    if equivalent {
                        println!("true");
                    } else {
                        println!("false");
                    }
                } else {
                    return Err("Unsupported file format for comparison.".into());
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
