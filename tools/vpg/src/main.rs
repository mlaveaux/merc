use std::fs::File;
use std::fs::read_to_string;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;

use duct::cmd;
use log::info;
use merc_vpg::CubeIterAll;
use merc_vpg::PgDot;
use merc_vpg::Player;
use merc_vpg::VpgDot;
use merc_vpg::ZielonkaVariant;
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

    /// The .dot file output filename
    output: String,

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
            Commands::Solve(args) => handle_solve(args, &mut timing)?,
            Commands::Reachable(args) => handle_reachable(args, &mut timing)?,
            Commands::Translate(args) => handle_translate(args)?,
            Commands::Display(args) => handle_display(args, &mut timing)?,
        }
    }

    if cli.timings {
        timing.print();
    }

    print_allocator_metrics();
    Ok(ExitCode::SUCCESS)
}


/// Handle the `solve` subcommand.
///
/// Reads either a standard parity game (PG) or a variability parity game (VPG)
/// based on the provided format or filename extension, then solves it using
/// Zielonka's algorithm.
fn handle_solve(args: SolveArgs, timing: &mut Timing) -> Result<(), MercError> {
    let path = Path::new(&args.filename);
    let mut file = File::open(path)?;
    let format =
        guess_format_from_extension(path, args.format).ok_or("Unknown parity game file format.")?;

    if format == ParityGameFormat::PG {
        // Read and solve a standard parity game.
        let mut time_read = timing.start("read_pg");
        let game = read_pg(&mut file)?;
        time_read.finish();

        let mut time_solve = timing.start("solve_zielonka");
        let solution = solve_zielonka(&game);
        if solution[0][0] {
            println!("{}", Player::Even.solution())
        } else {
            println!("{}", Player::Odd.solution())
        }
        time_solve.finish();
    } else {
        // Read and solve a variability parity game.
        let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);

        let mut time_read = timing.start("read_vpg");
        let game = read_vpg(&manager_ref, &mut file)?;
        time_read.finish();

        let mut time_solve = timing.start("solve_variability_zielonka");
        let solutions = solve_variability_zielonka(&manager_ref, &game, ZielonkaVariant::Standard, false)?;
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

    Ok(())
}

/// Handle the `reachable` subcommand.
///
/// Reads a PG or VPG, computes its reachable part, and writes it to `output`.
/// Also logs the vertex index mapping to aid inspection.
fn handle_reachable(args: ReachableArgs, timing: &mut Timing) -> Result<(), MercError> {
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
                debug!("{} -> {:?}", old_index, new_index);
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
                debug!("{} -> {:?}", old_index, new_index);
            }

            let mut output_file = File::create(&args.output)?;
            // Write reachable part using the PG writer, as reachable_game is a ParityGame.
            write_pg(&mut output_file, &reachable_game)?;
        }
    }

    Ok(())
}

/// Handle the `translate` subcommand.
///
/// Translates a feature diagram, a feature transition system (FTS), and a modal
/// formula into a variability parity game.
fn handle_translate(args: TranslateArgs) -> Result<(), MercError> {
    let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);

    // Read feature diagram
    let mut feature_diagram_file = File::open(&args.feature_diagram_filename).map_err(|e| {
        MercError::from(format!(
            "Could not open feature diagram file '{}': {}",
            &args.feature_diagram_filename, e
        ))
    })?;
    let feature_diagram = FeatureDiagram::from_reader(&manager_ref, &mut feature_diagram_file)?;

    // Read FTS
    let mut fts_file = File::open(&args.fts_filename).map_err(|e| {
        MercError::from(format!(
            "Could not open feature transition system file '{}': {}",
            &args.fts_filename, e
        ))
    })?;
    let fts = read_fts(&manager_ref, &mut fts_file, feature_diagram)?;

    // Read and validate formula (no actions/data specs supported here)
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

    Ok(())
}

/// Handle the `display` subcommand.
///
/// Reads a PG or VPG and writes a Graphviz `.dot` representation to `output`.
/// If the `dot` tool is available, also generates a PDF (`output.pdf`).
fn handle_display(args: DisplayArgs, timing: &mut Timing) -> Result<(), MercError> {
    let path = Path::new(&args.filename);
    let mut file = File::open(path)?;
    let format =
        guess_format_from_extension(path, args.format).ok_or("Unknown parity game file format.")?;

    if format == ParityGameFormat::PG {
        // Read and display a standard parity game.
        let mut time_read = timing.start("read_pg");
        let game = read_pg(&mut file)?;
        time_read.finish();

        let mut output_file = File::create(&args.output)?;
        write!(&mut output_file, "{}", PgDot::new(&game))?;
    } else {
        // Read and display a variability parity game.
        let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);

        let mut time_read = timing.start("read_vpg");
        let game = read_vpg(&manager_ref, &mut file)?;
        time_read.finish();

        let mut output_file = File::create(&args.output)?;
        write!(&mut output_file, "{}", VpgDot::new(&game))?;
    }

    if let Ok(dot_path) = which::which("dot") {
        info!("Generating PDF using dot...");
        cmd!(dot_path, "-Tpdf", &args.output, "-O").run()?;
    }

    Ok(())
}
