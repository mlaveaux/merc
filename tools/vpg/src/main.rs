use std::fs::File;
use std::fs::read_to_string;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;
use duct::cmd;
use itertools::Itertools;
use log::debug;
use log::info;
use merc_lts::LtsFormat;
use merc_lts::guess_lts_format_from_extension;
use merc_lts::read_aut;
use merc_lts::write_aut;
use merc_symbolic::bits_to_bdd;
use merc_vpg::Projected;
use merc_vpg::ProjectedLts;
use merc_vpg::Solver;
use merc_vpg::project_feature_transition_system_iter;
use merc_vpg::solve_priority_promotion;
use merc_vpg::solve_two_sided_lifting;
use merc_vpg::verify_solution;
use oxidd::BooleanFunction;

use merc_symbolic::CubeIterAll;
use merc_symbolic::FormatConfig;
use merc_syntax::UntypedStateFrmSpec;
use merc_tools::VerbosityFlag;
use merc_tools::Version;
use merc_tools::VersionFlag;
use merc_tools::format_key_values_json;
use merc_tools::report_error;
use merc_unsafety::print_allocator_metrics;
use merc_utilities::MercError;
use merc_utilities::Timing;
use merc_vpg::FeatureDiagram;
use merc_vpg::ParityGameFormat;
use merc_vpg::PgDot;
use merc_vpg::Player;
use merc_vpg::VpgDot;
use merc_vpg::VpgSolver;
use merc_vpg::compute_reachable;
use merc_vpg::guess_format_from_extension;
use merc_vpg::make_vpg_total;
use merc_vpg::project_variability_parity_games_iter;
use merc_vpg::read_fts;
use merc_vpg::read_pg;
use merc_vpg::read_vpg;
use merc_vpg::solve_variability_product_zielonka;
use merc_vpg::solve_variability_zielonka;
use merc_vpg::solve_zielonka;
use merc_vpg::translate;
use merc_vpg::translate_vpg;
use merc_vpg::verify_variability_product_zielonka_solution;
use merc_vpg::write_pg;
use merc_vpg::write_pg_solution;
use merc_vpg::write_vpg;

/// Default node capacity for the Oxidd decision diagram manager. The choice
/// for this value is fairly arbitrary.
const DEFAULT_OXIDD_NODE_CAPACITY: usize = 2028;

/// A command line tool for variability parity games
#[derive(clap::Parser, Debug)]
#[command(arg_required_else_help = true)]
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
    ProjectVpg(ProjectVpgArgs),
    Translate(TranslateArgs),
    TranslateVpg(TranslateVpgArgs),
    Display(DisplayArgs),
}

/// Solve a parity game
#[derive(clap::Args, Debug)]
struct SolveArgs {
    filename: PathBuf,

    /// The input parity game file format
    #[arg(long)]
    format: Option<ParityGameFormat>,

    /// Sets the solving variant for regular parity games.
    #[arg(long, default_value_t = Solver::Zielonka)]
    solver: Solver,

    /// Sets the solving variant used for variability parity games.
    #[arg(long, default_value_t = VpgSolver::Family)]
    vpg_solver: VpgSolver,

    /// Output the solution for every single vertex instead of only the initial vertex.
    #[arg(long, default_value_t = false)]
    full_solution: bool,

    /// Whether to verify the solution after computing it
    #[arg(long, default_value_t = false)]
    verify_solution: bool,

    /// Write the solution of a parity game in the PGSolver solution format to this file.
    #[arg(long)]
    solution_output: Option<PathBuf>,
}

/// Compute the reachable part of a parity game
#[derive(clap::Args, Debug)]
struct ReachableArgs {
    /// The filename of the parity game to compute the reachable part of
    filename: PathBuf,

    /// The output filename for the reachable part of the parity game
    output: PathBuf,

    #[arg(long, short)]
    format: Option<ParityGameFormat>,
}

/// Project a feature transition system to a set of transition systems
#[derive(clap::Args, Debug)]
struct ProjectArgs {
    /// The filename of the variability parity game to project
    filename: PathBuf,

    /// The filename of the feature diagram
    feature_diagram_filename: PathBuf,

    /// The output filename pattern for the projected labelled transition systems.
    output: String,

    /// The input featured transition system file format
    #[arg(long, short)]
    format: Option<LtsFormat>,
}

/// Project a variability parity game to a set of parity games
#[derive(clap::Args, Debug)]
struct ProjectVpgArgs {
    /// The filename of the variability parity game to project
    filename: PathBuf,

    /// The output filename pattern for the projected parity games.
    output: String,

    /// Whether to compute the reachable part of each projection.
    #[arg(long, short)]
    reachable: bool,

    #[arg(long, short)]
    format: Option<ParityGameFormat>,
}

/// Translate a labelled transition system and a modal formula into a parity game
#[derive(clap::Args, Debug)]
struct TranslateArgs {
    /// The filename of the labelled transition system
    labelled_transition_system: PathBuf,

    /// The input labelled transition system file format
    #[arg(long, short)]
    format: Option<LtsFormat>,

    /// The filename of the modal formula
    formula_filename: PathBuf,

    /// The parity game output filename
    output: PathBuf,
}

/// Translate a feature transition system and a modal formula into a variability parity game
#[derive(clap::Args, Debug)]
struct TranslateVpgArgs {
    /// The filename of the feature diagram
    feature_diagram_filename: PathBuf,

    /// The input featured transition system file format
    #[arg(long, short)]
    format: Option<LtsFormat>,

    /// The filename of the feature transition system
    fts_filename: PathBuf,

    /// The filename of the modal formula
    formula_filename: PathBuf,

    /// The variability parity game output filename
    output: String,
}

/// Display a (variability) parity game
#[derive(clap::Args, Debug)]
struct DisplayArgs {
    /// The filename of the (variability) parity game to display
    filename: PathBuf,

    /// The .dot file output filename
    output: PathBuf,

    /// The parity game file format
    #[arg(long, short)]
    format: Option<ParityGameFormat>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let mut timing = Timing::new();

    env_logger::Builder::new()
        .filter_level(cli.verbosity.log_level_filter())
        .format_key_values(|formatter, source| format_key_values_json(formatter, source))
        .parse_default_env()
        .init();

    if cli.version.into() {
        eprintln!("{}", Version);
        return ExitCode::SUCCESS;
    }

    let result = handle_command(&cli, &mut timing);

    if cli.timings {
        timing.print();
    }

    print_allocator_metrics();
    if cfg!(feature = "merc_metrics") {
        oxidd::bdd::print_stats();
    }
    report_error(result)
}

fn handle_command(cli: &Cli, timing: &mut Timing) -> Result<(), MercError> {
    if let Some(command) = &cli.commands {
        match command {
            Commands::Solve(args) => handle_solve(cli, args, timing)?,
            Commands::Reachable(args) => handle_reachable(cli, args, timing)?,
            Commands::Project(args) => handle_project_fts(cli, args, timing)?,
            Commands::ProjectVpg(args) => handle_project_vpg(cli, args, timing)?,
            Commands::Translate(args) => handle_translate(args)?,
            Commands::TranslateVpg(args) => handle_translate_vpg(cli, args)?,
            Commands::Display(args) => handle_display(cli, args, timing)?,
        }
    }

    Ok(())
}

/// Handle the `solve` subcommand.
///
/// Reads either a standard parity game (PG) or a variability parity game (VPG)
/// based on the provided format or filename extension, then solves it using
/// Zielonka's algorithm.
fn handle_solve(cli: &Cli, args: &SolveArgs, timing: &mut Timing) -> Result<(), MercError> {
    let path = Path::new(&args.filename);
    let mut file = File::open(path)?;
    let format = guess_format_from_extension(path, args.format)
        .ok_or_else(|| format!("Unknown parity game file format for '{}'.", path.display()))?;

    if format == ParityGameFormat::PG {
        // Read and solve a standard parity game.
        let game = timing.measure("read_pg", || read_pg(&mut file))?;

        let compute_strategy = args.verify_solution || args.solution_output.is_some();
        let (solution, strategy) = timing.measure("solve_zielonka", || match args.solver {
            Solver::Zielonka => solve_zielonka(&game, compute_strategy),
            Solver::PriorityPromotion => solve_priority_promotion(&game, compute_strategy),
            Solver::TwoSidedLifting => solve_two_sided_lifting(&game, compute_strategy),
        });
        if let Some(output) = &args.solution_output {
            write_pg_solution(File::create(output)?, &game, &solution, strategy.as_ref())?;
        }
        if args.full_solution {
            for (index, player_set) in solution.iter().enumerate() {
                println!("W{index}: {}", player_set.iter_ones().format(", "));
            }
        } else if solution[0][0] {
            println!("{}", Player::Even.solution())
        } else {
            println!("{}", Player::Odd.solution())
        }

        if let Some(strategy) = strategy
            && args.verify_solution
        {
            verify_solution(&game, &solution, &strategy);
        }
    } else {
        // Read and solve a variability parity game.
        let manager_ref = oxidd::bdd::new_manager(
            cli.oxidd_node_capacity,
            cli.oxidd_cache_capacity.unwrap_or(cli.oxidd_node_capacity),
            cli.oxidd_workers,
        );

        let game = timing.measure("read_vpg", || -> Result<_, MercError> {
            read_vpg(&manager_ref, &mut file)
        })?;

        let game = if !game.is_vpg_total(&manager_ref)? {
            info!("Making the VPG total...");
            make_vpg_total(&manager_ref, &game)?
        } else {
            game
        };

        timing.measure("solve_variability_zielonka", || -> Result<_, MercError> {
            if args.vpg_solver == VpgSolver::Product {
                let solver = args.solver;

                // Since we want to print W0, W1 separately, we need to store the results temporarily.
                let mut results = [Vec::new(), Vec::new()];
                for result in solve_variability_product_zielonka(&game, solver, timing) {
                    let (cube, _bdd, solution) = result?;

                    for (index, w) in solution.iter().enumerate() {
                        results[index].push((cube.clone(), w.clone()));
                    }
                }

                for (index, w) in results.iter().enumerate() {
                    for (cube, vertices) in w {
                        println!(
                            "W{index}: For product {} the following vertices are in: {}",
                            FormatConfig(cube),
                            vertices
                                .iter_ones()
                                .filter(|v| args.full_solution || *v == 0)
                                .format(", ")
                        );
                    }
                }
            } else {
                let solutions = solve_variability_zielonka(&manager_ref, &game, args.vpg_solver, false)?;
                for (index, w) in solutions.iter().enumerate() {
                    for entry in CubeIterAll::new(game.configuration()) {
                        let config = entry?;
                        let config_function = bits_to_bdd(&manager_ref, game.variables(), &config)?;

                        // `and` can fail to allocate, so compute the filtered vertices up front
                        // (propagating the error with `?`) rather than inside the `.filter`
                        // closure, which cannot return a `Result`.
                        let vertices = w
                            .iter() // Do not use iter_vertices because the first one is the initial vertex only
                            .take(if args.full_solution { usize::MAX } else { 1 }) // Take only first if we don't want full solution
                            .filter_map(|(v, config)| match config.and(&config_function) {
                                Ok(intersection) => intersection.satisfiable().then_some(Ok(v)),
                                Err(err) => Some(Err(err)),
                            })
                            .collect::<Result<Vec<_>, _>>()?;

                        println!(
                            "W{index}: For product {} the following vertices are in: {}",
                            FormatConfig(&config),
                            vertices.iter().format(", ")
                        );
                    }
                }

                if args.verify_solution {
                    verify_variability_product_zielonka_solution(&game, &solutions, timing)?;
                }
            }

            Ok(())
        })?;
    }

    Ok(())
}

/// Handle the `reachable` subcommand.
///
/// Reads a PG or VPG, computes its reachable part, and writes it to `output`.
/// Also logs the vertex index mapping to aid inspection.
fn handle_reachable(cli: &Cli, args: &ReachableArgs, timing: &mut Timing) -> Result<(), MercError> {
    let path = Path::new(&args.filename);
    let format = guess_format_from_extension(path, args.format)
        .ok_or_else(|| format!("Unknown parity game file format for '{}'.", path.display()))?;

    let mut file = File::open(path)?;
    match format {
        ParityGameFormat::PG => {
            let game = timing.measure("read_pg", || read_pg(&mut file))?;

            let (reachable_game, mapping) = timing.measure("compute_reachable", || compute_reachable(&game));

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

            let game = timing.measure("read_vpg", || read_vpg(&manager_ref, &mut file))?;
            let (reachable_game, mapping) = timing.measure("compute_reachable_vpg", || compute_reachable(&game));

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

/// Projects a feature transition system to a set of transition systems and writes them to output.
fn handle_project_fts(cli: &Cli, args: &ProjectArgs, timing: &mut Timing) -> Result<(), MercError> {
    let format = guess_lts_format_from_extension(&args.filename, args.format).ok_or_else(|| {
        format!(
            "Unknown featured transition system format for '{}'.",
            args.filename.display()
        )
    })?;

    if format != LtsFormat::Aut {
        return Err(MercError::from(
            "The project command only works for featured transition systems in the .aut format.",
        ));
    }

    // Read and solve a variability parity game.
    let manager_ref = oxidd::bdd::new_manager(
        cli.oxidd_node_capacity,
        cli.oxidd_cache_capacity.unwrap_or(cli.oxidd_node_capacity),
        cli.oxidd_workers,
    );

    // Read feature diagram
    let mut feature_diagram_file = File::open(&args.feature_diagram_filename).map_err(|e| {
        MercError::from(format!(
            "Could not open feature diagram file '{}': {}",
            args.feature_diagram_filename.display(),
            e
        ))
    })?;
    let feature_diagram = FeatureDiagram::from_reader(&manager_ref, &mut feature_diagram_file)?;
    debug!("Feature diagram {:?}", feature_diagram);

    // Read the feature transition system.
    let mut fts_file = File::open(&args.filename)?;
    let fts = read_fts(&manager_ref, &mut fts_file, feature_diagram.features().clone())?;
    let output_path = Path::new(&args.output);

    for result in project_feature_transition_system_iter(&fts, &feature_diagram, timing) {
        let (ProjectedLts { bits, bdd: _, lts }, _) = result?;

        let extension = output_path.extension().ok_or("Missing extension on output file")?;
        let new_path = output_path
            .with_file_name(format!(
                "{}_{}",
                output_path
                    .file_stem()
                    .ok_or("Missing filename on output")?
                    .to_string_lossy(),
                FormatConfig(&bits)
            ))
            .with_added_extension(extension);
        let mut output_file = File::create(new_path)?;

        write_aut(&mut output_file, &lts)?;
    }

    Ok(())
}

/// Compute all the projections of a variability parity game and write them to output.
fn handle_project_vpg(cli: &Cli, args: &ProjectVpgArgs, timing: &mut Timing) -> Result<(), MercError> {
    let format = guess_format_from_extension(&args.filename, args.format)
        .ok_or_else(|| format!("Unknown parity game file format for '{}'.", args.filename.display()))?;

    let mut file = File::open(&args.filename)?;
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

    let vpg = timing.measure("read_vpg", || read_vpg(&manager_ref, &mut file))?;
    let output_path = Path::new(&args.output);

    for result in project_variability_parity_games_iter(&vpg, timing) {
        let (Projected { bits, bdd: _, game }, _) = result?;

        let extension = output_path.extension().ok_or("Missing extension on output file")?;
        let new_path = output_path
            .with_file_name(format!(
                "{}_{}",
                output_path
                    .file_stem()
                    .ok_or("Missing filename on output")?
                    .to_string_lossy(),
                FormatConfig(&bits)
            ))
            .with_added_extension(extension);

        let mut output_file = File::create(new_path)?;

        if args.reachable {
            let (reachable_pg, _projection) = compute_reachable(&game);
            write_pg(&mut output_file, &reachable_pg)?;
        } else {
            write_pg(&mut output_file, &game)?;
        }
    }

    Ok(())
}

/// Handle the `translate` subcommand.
///
/// Translates a feature diagram, a feature transition system (FTS), and a modal
/// formula into a variability parity game.
fn handle_translate(args: &TranslateArgs) -> Result<(), MercError> {
    let format = guess_lts_format_from_extension(&args.labelled_transition_system, args.format).ok_or_else(|| {
        format!(
            "Unknown labelled transition system format for '{}'.",
            args.labelled_transition_system.display()
        )
    })?;

    if format != LtsFormat::Aut {
        return Err(MercError::from(
            "The translate command only works for labelled transition systems in the .aut format.",
        ));
    }

    // Read LTS
    let mut lts_file = File::open(&args.labelled_transition_system).map_err(|e| {
        MercError::from(format!(
            "Could not open feature transition system file '{}': {}",
            args.labelled_transition_system.display(),
            e
        ))
    })?;
    let lts = read_aut(&mut lts_file)?;

    // `translate` type checks the formula itself, with or without `act`/data declarations: a
    // formula with none is checked leniently, as "simple actions" (see
    // `merc_typecheck::ModalSpecification`).
    let formula_spec = UntypedStateFrmSpec::parse(&read_to_string(&args.formula_filename).map_err(|e| {
        MercError::from(format!(
            "Could not open formula file '{}': {}",
            args.formula_filename.display(),
            e
        ))
    })?)?;

    let vpg = translate(&lts, formula_spec)?;

    let mut output_file = File::create(&args.output)?;
    write_pg(&mut output_file, &vpg)?;

    Ok(())
}

/// Handle the `translate_vpg` subcommand.
///
/// Translates a feature diagram, a feature transition system (FTS), and a modal
/// formula into a variability parity game.
fn handle_translate_vpg(cli: &Cli, args: &TranslateVpgArgs) -> Result<(), MercError> {
    let manager_ref = oxidd::bdd::new_manager(
        cli.oxidd_node_capacity,
        cli.oxidd_cache_capacity.unwrap_or(cli.oxidd_node_capacity),
        cli.oxidd_workers,
    );

    // Read feature diagram
    let mut feature_diagram_file = File::open(&args.feature_diagram_filename).map_err(|e| {
        MercError::from(format!(
            "Could not open feature diagram file '{}': {}",
            args.feature_diagram_filename.display(),
            e
        ))
    })?;
    let feature_diagram = FeatureDiagram::from_reader(&manager_ref, &mut feature_diagram_file)?;

    // Read FTS
    let mut fts_file = File::open(&args.fts_filename).map_err(|e| {
        MercError::from(format!(
            "Could not open feature transition system file '{}': {}",
            args.fts_filename.display(),
            e
        ))
    })?;
    let fts = read_fts(&manager_ref, &mut fts_file, feature_diagram.features().clone())?;

    // `translate_vpg` type checks the formula itself, with or without `act`/data declarations: a
    // formula with none is checked leniently, as "simple actions" (see
    // `merc_typecheck::ModalSpecification`).
    let formula_spec = UntypedStateFrmSpec::parse(&read_to_string(&args.formula_filename).map_err(|e| {
        MercError::from(format!(
            "Could not open formula file '{}': {}",
            args.formula_filename.display(),
            e
        ))
    })?)?;

    let vpg = translate_vpg(
        &manager_ref,
        &fts,
        feature_diagram.configuration().clone(),
        formula_spec,
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
    let format = guess_format_from_extension(path, args.format)
        .ok_or_else(|| format!("Unknown parity game file format for '{}'.", path.display()))?;

    if format == ParityGameFormat::PG {
        // Read and display a standard parity game.
        let game = timing.measure("read_pg", || read_pg(&mut file))?;

        let mut output_file = File::create(&args.output)?;
        write!(&mut output_file, "{}", PgDot::new(&game))?;
    } else {
        // Read and display a variability parity game.
        let manager_ref = oxidd::bdd::new_manager(
            cli.oxidd_node_capacity,
            cli.oxidd_cache_capacity.unwrap_or(cli.oxidd_node_capacity),
            cli.oxidd_workers,
        );

        let game = timing.measure("read_vpg", || read_vpg(&manager_ref, &mut file))?;

        let mut output_file = File::create(&args.output)?;
        write!(&mut output_file, "{}", VpgDot::new(&game))?;
    }

    if let Ok(dot_path) = which::which("dot") {
        info!("Generating PDF using dot...");
        cmd!(dot_path, "-Tpdf", &args.output, "-O").run()?;
    }

    Ok(())
}
