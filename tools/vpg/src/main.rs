use std::fs::File;
use std::fs::read_to_string;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;
use duct::cmd;
use itertools::Itertools;
use log::debug;
use log::info;
use merc_vpg::make_vpg_total;
use merc_vpg::verify_variability_product_zielonka_solution;
use oxidd::BooleanFunction;

use merc_symbolic::CubeIterAll;
use merc_symbolic::FormatConfig;
use merc_syntax::UntypedStateFrmSpec;
use merc_tools::VerbosityFlag;
use merc_tools::Version;
use merc_tools::VersionFlag;
use merc_unsafety::print_allocator_metrics;
use merc_utilities::MercError;
use merc_utilities::Timing;
use merc_vpg::FeatureDiagram;
use merc_vpg::ParityGameFormat;
use merc_vpg::PgDot;
use merc_vpg::Player;
use merc_vpg::VpgDot;
use merc_vpg::ZielonkaVariant;
use merc_vpg::compute_reachable;
use merc_vpg::guess_format_from_extension;
use merc_vpg::project_variability_parity_games_iter;
use merc_vpg::read_fts;
use merc_vpg::read_pg;
use merc_vpg::read_vpg;
use merc_vpg::solve_variability_product_zielonka;
use merc_vpg::solve_variability_zielonka;
use merc_vpg::solve_zielonka;
use merc_vpg::translate;
use merc_vpg::write_pg;
use merc_vpg::write_vpg;

/// Default node capacity for the Oxidd decision diagram manager.
const DEFAULT_OXIDD_NODE_CAPACITY: usize = 2024;

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

    #[arg(long, global = true, default_value_t = 1)]
    oxidd_workers: u32,

    #[arg(long, global = true, default_value_t = DEFAULT_OXIDD_NODE_CAPACITY)]
    oxidd_node_capacity: usize,

    #[arg(long, global = true)]
    oxidd_cache_capacity: Option<usize>,

    #[command(subcommand)]
    commands: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Solve(SolveArgs),
    Reachable(ReachableArgs),
    Project(ProjectArgs),
    Translate(TranslateArgs),
    Display(DisplayArgs),
}

/// Arguments for solving a parity game
#[derive(clap::Args, Debug)]
struct SolveArgs {
    filename: String,

    /// The parity game file format
    #[arg(long)]
    format: Option<ParityGameFormat>,

    /// For variability parity games there are several ways for solving.
    #[arg(long)]
    solve_variant: Option<ZielonkaVariant>,

    /// Whether to output the solution for every single vertex, not just in the initial vertex.
    #[arg(long, default_value_t = false)]
    full_solution: bool,

    /// Whether to verify the solution after computing it
    #[arg(long, default_value_t = false)]
    verify_solution: bool,
}

/// Arguments for computing the reachable part of a parity game
#[derive(clap::Args, Debug)]
struct ReachableArgs {
    filename: String,

    output: String,

    #[arg(long, short)]
    format: Option<ParityGameFormat>,
}

/// Arguments for projecting a variability parity game
#[derive(clap::Args, Debug)]
struct ProjectArgs {
    filename: String,

    output: String,

    /// Whether to compute the reachable part after outputting each projection
    #[arg(long, short, default_value_t = false)]
    reachable: bool,

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

    if let Some(command) = &cli.commands {
        match command {
            Commands::Solve(args) => handle_solve(&cli, args, &mut timing)?,
            Commands::Reachable(args) => handle_reachable(&cli, args, &mut timing)?,
            Commands::Project(args) => handle_project(&cli, args, &mut timing)?,
            Commands::Translate(args) => handle_translate(&cli, args)?,
            Commands::Display(args) => handle_display(&cli, args, &mut timing)?,
        }
    }

    if cli.timings {
        timing.print();
    }

    print_allocator_metrics();
    if cfg!(feature = "merc_metrics") {
        oxidd::bdd::print_stats();
    }
    Ok(ExitCode::SUCCESS)
}

/// Handle the `solve` subcommand.
///
/// Reads either a standard parity game (PG) or a variability parity game (VPG)
/// based on the provided format or filename extension, then solves it using
/// Zielonka's algorithm.
fn handle_solve(cli: &Cli, args: &SolveArgs, timing: &mut Timing) -> Result<(), MercError> {
    let path = Path::new(&args.filename);
    let mut file = File::open(path)?;
    let format = guess_format_from_extension(path, args.format).ok_or("Unknown parity game file format.")?;

    if format == ParityGameFormat::PG {
        // Read and solve a standard parity game.
        let mut time_read = timing.start("read_pg");
        let game = read_pg(&mut file)?;
        time_read.finish();

        let mut time_solve = timing.start("solve_zielonka");
        let solution = solve_zielonka(&game);
        if args.full_solution {
            for (index, player_set) in solution.iter().enumerate() {
                println!("W{index}: {}", player_set.iter_ones().format(", "));
            }
        } else if solution[0][0] {
            println!("{}", Player::Even.solution())
        } else {
            println!("{}", Player::Odd.solution())
        }
        time_solve.finish();
    } else {
        let solve_variant = args
            .solve_variant
            .ok_or("For variability parity game solving a solving strategy should be selected")?;

        // Read and solve a variability parity game.
        let manager_ref = oxidd::bdd::new_manager(
            cli.oxidd_node_capacity,
            cli.oxidd_cache_capacity.unwrap_or(cli.oxidd_node_capacity),
            cli.oxidd_workers,
        );

        let mut time_read = timing.start("read_vpg");
        let game = read_vpg(&manager_ref, &mut file)?;
        time_read.finish();

        let game = if !game.is_total(&manager_ref)? {
            make_vpg_total(&manager_ref, &game)?
        } else {
            game
        };

        let mut time_solve = timing.start("solve_variability_zielonka");
        if solve_variant == ZielonkaVariant::Product {
            // Since we want to print W0, W1 separately, we need to store the results temporarily.
            let mut results = [Vec::new(), Vec::new()];
            for (cube, _bdd, solution) in solve_variability_product_zielonka(&game) {
                for (index, w) in solution.iter().enumerate() {
                    results[index].push((cube.clone(), w.clone()));
                }
            }

            for (index, w) in results.iter().enumerate() {
                println!("W{index}: ");

                for (cube, vertices) in w {
                    println!(
                        "For product {} the following vertices are in: {}",
                        FormatConfig(cube),
                        vertices
                            .iter_ones()
                            .filter(|v| if args.full_solution { true } else { *v == 0 })
                            .format(", ")
                    );
                }
            }
        } else {
            let solutions = solve_variability_zielonka(&manager_ref, &game, solve_variant, false)?;
            for (index, w) in solutions.iter().enumerate() {
                println!("W{index}: ");

                for entry in CubeIterAll::new(game.variables(), game.configuration()) {
                    let (config, config_function) = entry?;

                    println!(
                        "For product {} the following vertices are in: {}",
                        FormatConfig(&config),
                        w.iter() // Do not use iter_vertices because the first one is the initial vertex only
                            .take(if args.full_solution { usize::MAX } else { 1 }) // Take only first if we don't want full solution
                            .filter(|(_v, config)| { config.and(&config_function).unwrap().satisfiable() })
                            .map(|(v, _)| v)
                            .format(", ")
                    );
                }
            }

            if args.verify_solution {
                verify_variability_product_zielonka_solution(&game, &solutions)?;
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
fn handle_reachable(cli: &Cli, args: &ReachableArgs, timing: &mut Timing) -> Result<(), MercError> {
    let path = Path::new(&args.filename);
    let mut file = File::open(path)?;

    let format = guess_format_from_extension(path, args.format).ok_or("Unknown parity game file format.")?;

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
            let manager_ref = oxidd::bdd::new_manager(
                cli.oxidd_node_capacity,
                cli.oxidd_cache_capacity.unwrap_or(cli.oxidd_node_capacity),
                cli.oxidd_workers,
            );

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

/// Compute all the projects of a variability parity game and write them to output.
fn handle_project(cli: &Cli, args: &ProjectArgs, timing: &mut Timing) -> Result<(), MercError> {
    let path = Path::new(&args.filename);
    let mut file = File::open(path)?;
    let format = guess_format_from_extension(path, args.format).ok_or("Unknown parity game file format.")?;

    if format != ParityGameFormat::VPG {
        return Err(MercError::from(
            "The project command only works for variability parity games.",
        ));
    }

    // Read the variability parity game.
    let manager_ref = oxidd::bdd::new_manager(
        cli.oxidd_node_capacity,
        cli.oxidd_cache_capacity.unwrap_or(cli.oxidd_node_capacity),
        cli.oxidd_workers,
    );

    let mut time_read = timing.start("read_vpg");
    let vpg = read_vpg(&manager_ref, &mut file)?;
    time_read.finish();

    let output_path = Path::new(&args.output);

    for result in project_variability_parity_games_iter(&vpg) {
        let (cube, _bdd, pg) = result?;

        let extension = output_path.extension().ok_or("Missing extension on output file")?;
        let new_path = output_path
            .with_file_name(format!(
                "{}_{}",
                output_path
                    .file_stem()
                    .ok_or("Missing filename on output")?
                    .to_string_lossy(),
                FormatConfig(&cube)
            ))
            .with_extension(extension);

        let mut output_file = File::create(new_path)?;

        if args.reachable {
            let (reachable_pg, _projection) = compute_reachable(&pg);
            write_pg(&mut output_file, &reachable_pg)?;
        } else {
            write_pg(&mut output_file, &pg)?;
        }
    }

    Ok(())
}

/// Handle the `translate` subcommand.
///
/// Translates a feature diagram, a feature transition system (FTS), and a modal
/// formula into a variability parity game.
fn handle_translate(cli: &Cli, args: &TranslateArgs) -> Result<(), MercError> {
    let manager_ref = oxidd::bdd::new_manager(
        cli.oxidd_node_capacity,
        cli.oxidd_cache_capacity.unwrap_or(cli.oxidd_node_capacity),
        cli.oxidd_workers,
    );

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
    let fts = read_fts(&manager_ref, &mut fts_file, feature_diagram.features().clone())?;

    // Read and validate formula (no actions/data specs supported here)
    let formula_spec = UntypedStateFrmSpec::parse(&read_to_string(&args.formula_filename).map_err(|e| {
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

    let vpg = translate(
        &manager_ref,
        &fts,
        feature_diagram.configuration().clone(),
        &formula_spec.formula,
    )?;
    let mut output_file = File::create(&args.output)?;
    write_vpg(&mut output_file, &vpg)?;

    Ok(())
}

/// Handle the `display` subcommand.
///
/// Reads a PG or VPG and writes a Graphviz `.dot` representation to `output`.
/// If the `dot` tool is available, also generates a PDF (`output.pdf`).
fn handle_display(cli: &Cli, args: &DisplayArgs, timing: &mut Timing) -> Result<(), MercError> {
    let path = Path::new(&args.filename);
    let mut file = File::open(path)?;
    let format = guess_format_from_extension(path, args.format).ok_or("Unknown parity game file format.")?;

    if format == ParityGameFormat::PG {
        // Read and display a standard parity game.
        let mut time_read = timing.start("read_pg");
        let game = read_pg(&mut file)?;
        time_read.finish();

        let mut output_file = File::create(&args.output)?;
        write!(&mut output_file, "{}", PgDot::new(&game))?;
    } else {
        // Read and display a variability parity game.
        let manager_ref = oxidd::bdd::new_manager(
            cli.oxidd_node_capacity,
            cli.oxidd_cache_capacity.unwrap_or(cli.oxidd_node_capacity),
            cli.oxidd_workers,
        );

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
