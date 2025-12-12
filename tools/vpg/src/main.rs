use std::fs::File;
use std::fs::read_to_string;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;

use merc_vpg::CubeIterAll;
use merc_vpg::PgDot;
use merc_vpg::compute_reachable;
use merc_vpg::write_pg;
use oxidd::BooleanFunction;

use log::debug;
use merc_syntax::UntypedStateFrmSpec;
use merc_tools::VerbosityFlag;
use merc_tools::Version;
use merc_tools::VersionFlag;
use merc_unsafety::print_allocator_metrics;
use merc_utilities::MercError;
use merc_utilities::Timing;
use merc_vpg::FeatureDiagram;
use merc_vpg::FormatConfig;
use merc_vpg::ParityGameFormat;
use merc_vpg::guess_format_from_extension;
use merc_vpg::read_fts;
use merc_vpg::read_pg;
use merc_vpg::read_vpg;
use merc_vpg::solve_variability_zielonka;
use merc_vpg::solve_zielonka;
use merc_vpg::translate;
use merc_vpg::write_vpg;

#[derive(clap::Parser, Debug)]
#[command(
    about = "A command line tool for variability parity games",
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
    Solve(SolveArgs),
    Reachable(ReachableArgs),
    Translate(TranslateArgs),
    Display(DisplayArgs),
}

/// Arguments for solving a parity game
#[derive(clap::Args, Debug)]
struct SolveArgs {
    filename: String,

    /// The parity game file format
    format: Option<ParityGameFormat>,

    /// Whether to output the solution for every single vertex, not just in the initial vertex.
    #[arg(long, default_value_t = false)]
    full_solution: bool,
}

/// Arguments for computing the reachable part of a parity game
#[derive(clap::Args, Debug)]
struct ReachableArgs {
    filename: String,

    output: String,

    #[arg(long, short)]
    format: Option<ParityGameFormat>,
}

/// Arguments for translating a feature transition system and a modal formula into a variability parity game
#[derive(clap::Args, Debug)]
struct TranslateArgs {
    /// The filename of the feature diagram
    feature_diagram_filename: String,

    /// The filename of the feature transition system
    fts_filename: String,

    /// The filename of the modal formula
    formula_filename: String,

    /// The variability parity game output filename
    output: String,
}

/// Arguments for displaying a (variability) parity game
#[derive(clap::Args, Debug)]
struct DisplayArgs {
    filename: String,

    /// The parity game file format
    #[arg(long, short)]
    format: Option<ParityGameFormat>,
}

fn main() -> Result<ExitCode, MercError> {
    let cli = Cli::parse();

    let mut timing = Timing::new();

    env_logger::Builder::new()
        .filter_level(cli.verbosity.log_level_filter())
        .parse_default_env()
        .init();

    if cli.version.into() {
        eprintln!("{}", Version);
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(command) = cli.commands {
        match command {
            Commands::Solve(args) => {
                let path = Path::new(&args.filename);
                let mut file = File::open(path)?;
                let format =
                    guess_format_from_extension(path, args.format).ok_or("Unknown parity game file format.")?;

                if format == ParityGameFormat::PG {
                    // Read and solve a standard parity game and solve it.
                    let mut time_read = timing.start("read_pg");
                    let game = read_pg(&mut file)?;
                    time_read.finish();

                    let mut time_solve = timing.start("solve_zielonka");
                    println!("{}", solve_zielonka(&game).solution());
                    time_solve.finish();
                } else {
                    // Read and solve a variability parity game and solve it.
                    let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);

                    let mut time_read = timing.start("read_vpg");
                    let game = read_vpg(&manager_ref, &mut file)?;
                    time_read.finish();

                    let mut time_solve = timing.start("solve_variability_zielonka");
                    let solutions = solve_variability_zielonka(&manager_ref, &game, false)?;
                    for (index, w) in solutions.iter().enumerate() {
                        println!("W{index}: ");

                        for entry in CubeIterAll::new(game.variables(), &game.configuration()) {
                            let (config, config_function) = entry?;

                            print!("For product {} the following vertices are in: ", FormatConfig(&config));
                            let mut first = true;
                            for (vertex, configuration) in w.iter() {
                                if !first {
                                    print!(", ");
                                }

                                if configuration.and(&config_function)?.satisfiable() {
                                    print!("{}", vertex);
                                }
                                first = false;

                                if !args.full_solution {
                                    // Only print the solution for the initial vertex
                                    break;
                                }
                            }
                            println!();
                        }
                    }
                    time_solve.finish();
                }
            }
            Commands::Reachable(args) => {
                // Read a parity game, compute its reachable part, and write it to a new file.
                let path = Path::new(&args.filename);
                let mut file = File::open(&path)?;

                let format =
                    guess_format_from_extension(&path, args.format).ok_or("Unknown parity game file format.")?;

                match format {
                    ParityGameFormat::PG => {
                        let mut time_read = timing.start("read_pg");
                        let game = read_pg(&mut file)?;
                        time_read.finish();

                        let mut time_reachable = timing.start("compute_reachable");
                        let (reachable_game, mapping) = compute_reachable(&game);
                        time_reachable.finish();

                        for (old_index, new_index) in mapping.iter().enumerate() {
                            debug!("{} -> {}", old_index, new_index);
                        }

                        let mut output_file = File::create(&args.output)?;
                        write_pg(&mut output_file, &reachable_game)?;
                    }
                    ParityGameFormat::VPG => {
                        let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);

                        let mut time_read = timing.start("read_vpg");
                        let game = read_vpg(&manager_ref, &mut file)?;
                        time_read.finish();

                        let mut time_reachable = timing.start("compute_reachable_vpg");
                        let (reachable_game, mapping) = compute_reachable(&game);
                        time_reachable.finish();

                        for (old_index, new_index) in mapping.iter().enumerate() {
                            debug!("{} -> {}", old_index, new_index);
                        }

                        let mut output_file = File::create(&args.output)?;
                        write_pg(&mut output_file, &reachable_game)?;
                    }
                }
            }
            Commands::Translate(args) => {
                // Read a feature diagram and a feature transition system, encode it into a variability parity game, and write it to a new file.
                let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);

                let mut feature_diagram_file = File::open(&args.feature_diagram_filename).map_err(|e| {
                    MercError::from(format!(
                        "Could not open feature diagram file '{}': {}",
                        &args.feature_diagram_filename, e
                    ))
                })?;
                let feature_diagram = FeatureDiagram::from_reader(&manager_ref, &mut feature_diagram_file)?;

                let mut fts_file = File::open(&args.fts_filename).map_err(|e| {
                    MercError::from(format!(
                        "Could not open feature transition system file '{}': {}",
                        &args.fts_filename, e
                    ))
                })?;
                let fts = read_fts(&manager_ref, &mut fts_file, feature_diagram)?;

                let formula_spec =
                    UntypedStateFrmSpec::parse(&read_to_string(&args.formula_filename).map_err(|e| {
                        MercError::from(format!(
                            "Could not open formula file '{}': {}",
                            &args.formula_filename, e
                        ))
                    })?)?;
                if !formula_spec.action_declarations.is_empty() {
                    return Err(MercError::from("We do not support formulas with action declarations."));
                }

                if !formula_spec.data_specification.is_empty() {
                    return Err(MercError::from("The formula must not contain a data specification."));
                }

                let vpg = translate(&manager_ref, &fts, &formula_spec.formula)?;
                let mut output_file = File::create(&args.output)?;
                write_vpg(&mut output_file, &vpg)?;
            }
            Commands::Display(args) => {
                // Read and display a (variability) parity game.
                let path = Path::new(&args.filename);
                let mut file = File::open(path)?;
                let format =
                    guess_format_from_extension(path, args.format).ok_or("Unknown parity game file format.")?;

                if format == ParityGameFormat::PG {
                    // Read and display a standard parity game.
                    let mut time_read = timing.start("read_pg");
                    let game = read_pg(&mut file)?;
                    time_read.finish();

                    println!("{}", PgDot::new(&game));

                    // If we can find 'dot' in the PATH, we can also generate a pdf image.
                } else {
                    // Read and display a variability parity game.
                    let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);

                    let mut time_read = timing.start("read_vpg");
                    let _game = read_vpg(&manager_ref, &mut file)?;
                    time_read.finish();

                    unimplemented!("Displaying variability parity games is not yet implemented.")
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
