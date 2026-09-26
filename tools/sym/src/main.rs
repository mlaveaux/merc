use std::ffi::OsStr;
use std::fs::File;
use std::ops::ControlFlow;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;

use itertools::Itertools;
use log::warn;
use merc_io::LargeFormatter;
use merc_lts::AutStream;
use merc_lts::LtsAction;
use merc_lts::LtsBuilderMem;
use merc_lts::LtsFormat;
use merc_lts::LtsMultiAction;
use merc_lts::guess_lts_format_from_extension;
use merc_lts::write_bcg;
use merc_symbolic::ExplorationStrategy;
use merc_symbolic::LddLenCache;
use merc_symbolic::OxiddArgs;
use merc_symbolic::ReachabilityOptions;
use merc_symbolic::SatCountCache;
use merc_symbolic::SymFormat;
use merc_symbolic::SymbolicLPS;
use merc_symbolic::SymbolicLTS;
use merc_symbolic::SymbolicLtsBdd;
use merc_symbolic::approx_satcount;
use merc_symbolic::convert_symbolic_lts;
use merc_symbolic::convert_symbolic_lts_bdd;
use merc_symbolic::guess_format_from_extension;
use merc_symbolic::ldd_len;
use merc_symbolic::parse_compacted_dependency_graph;
use merc_symbolic::quotient_symbolic;
use merc_symbolic::reachability_bdd;
use merc_symbolic::reachability_with_callback;
use merc_symbolic::read_sylvan;
use merc_symbolic::read_symbolic_lts;
use merc_symbolic::refine_bisimulation;
use merc_symbolic::reorder;
use merc_symbolic::sigref_symbolic;
use merc_symbolic::write_symbolic_lts;
use merc_tools::KaHyParArgs;
use merc_tools::VerbosityFlag;
use merc_tools::Version;
use merc_tools::VersionFlag;
use merc_tools::report_error;
use merc_unsafety::print_allocator_metrics;
use merc_utilities::MercError;
use merc_utilities::Timing;
use oxidd::BooleanFunction;
use oxidd::Function;
use oxidd::ldd::LDDFunction;
use which::which_in;

/// A command line tool for symbolic labelled transition systems
#[derive(clap::Parser, Debug)]
#[command(arg_required_else_help = true)]
struct Cli {
    #[command(flatten)]
    version: VersionFlag,

    #[command(flatten)]
    verbosity: VerbosityFlag,

    #[command(subcommand)]
    commands: Option<Commands>,

    #[command(flatten)]
    oxidd: OxiddArgs,

    #[arg(long, global = true)]
    timings: bool,
}

/// Defines the subcommands for this tool.
#[derive(Debug, Subcommand)]
enum Commands {
    /// Print information related to the given symbolic LTS.
    Info(InfoArgs),
    /// Computes the reachable state count of a symbolic LTS using LDD-based reachability.
    Reachability(ReachabilityArgs),
    /// Computes the reachable state count of a symbolic LTS using BDD-based reachability.
    ReachabilityBdd(ReachabilityBddArgs),
    /// Computes a reordering for a dependency graph given by lpsreach or pbessolvesymbolic.
    Reorder(ReorderArgs),
    /// Convert a symbolic LTS to a concrete LTS.
    Convert(ConvertArgs),
    /// Apply reductions to a symbolic LTS.
    Reduce(ReduceArgs),
}

#[derive(clap::Args, Debug)]
struct InfoArgs {
    filename: PathBuf,

    /// Sets the input symbolic LTS format.
    #[arg(long)]
    format: Option<SymFormat>,
}

#[derive(clap::Args, Debug)]
struct ReachabilityArgs {
    filename: PathBuf,

    /// Sets the input symbolic LTS format.
    #[arg(long)]
    format: Option<SymFormat>,

    /// Exploration strategy to use.
    #[arg(long, value_enum, default_value = "breadth-first")]
    strategy: ExplorationStrategy,

    /// Detect deadlock states (states without outgoing transitions).
    #[arg(long)]
    detect_deadlocks: bool,

    /// Stop the exploration after this many iterations (rounds for saturation), and report what has
    /// been found until then. The reported states and deadlocks may then be incomplete.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    max_iterations: Option<u32>,

    /// Write the reachable symbolic LTS to this .sym output file.
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(clap::Args, Debug)]
struct ReachabilityBddArgs {
    filename: PathBuf,

    /// Sets the input symbolic LTS format.
    #[arg(long)]
    format: Option<SymFormat>,

    /// Visualize intermediate BDDs using oxidd-vis.
    #[arg(long)]
    visualize: bool,
}

#[derive(clap::Args, Debug)]
struct ReorderArgs {
    /// Explicit path to the mCRL2 tools (lpsreach or pbessolvesymbolic)
    #[arg(long)]
    mcrl2_path: Option<PathBuf>,

    #[command(flatten)]
    kahypar: KaHyParArgs,

    /// The input linear process specification file in the mCRL2 .lps format.
    filename: PathBuf,
}

#[derive(clap::Args, Debug)]
struct ConvertArgs {
    /// Sets the input symbolic LTS format.
    #[arg(long)]
    format: Option<SymFormat>,

    /// The input symbolic LTS file path.
    filename: PathBuf,

    /// Sets the output LTS format.
    #[arg(long)]
    output_format: Option<LtsFormat>,

    /// The output LTS file path.
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Clone, Copy, clap::ValueEnum, Debug)]
enum Equivalence {
    StrongBisim,
    StrongBisimSigref,
}

#[derive(clap::Args, Debug)]
struct ReduceArgs {
    /// The equivalence relation to reduce modulo.
    equivalence: Equivalence,

    /// The input symbolic LTS file path.
    filename: PathBuf,

    /// Sets the input symbolic LTS format.
    #[arg(long)]
    format: Option<SymFormat>,

    /// Sets the output LTS format.
    #[arg(long)]
    output_format: Option<LtsFormat>,

    /// The output LTS file path.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Visualize the reduction steps in oxidd-vis.
    #[arg(long)]
    visualize: bool,

    /// Split the signature per transition group.
    #[arg(long)]
    split_signature: bool,

    /// Extend each transition relation to the full state and next-state domain.
    #[arg(long)]
    extend_relation: bool,

    /// Merge transition relations into a single relation before signature computation.
    #[arg(long)]
    merge_transitions: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    env_logger::Builder::new()
        .filter_level(cli.verbosity.log_level_filter())
        .parse_default_env()
        .init();

    if cli.version.into() {
        eprintln!("{}", Version);
        return ExitCode::SUCCESS;
    }

    let timing = Timing::new();
    let result = handle_command(&cli, &timing);

    if cli.timings {
        timing.print();
    }

    print_allocator_metrics();
    report_error(result)
}

fn handle_command(cli: &Cli, timing: &Timing) -> Result<(), MercError> {
    if let Some(command) = &cli.commands {
        match command {
            Commands::Info(args) => handle_info(cli, args, timing)?,
            Commands::Reachability(args) => handle_reachability(cli, args, timing)?,
            Commands::ReachabilityBdd(args) => handle_reachability_bdd(cli, args, timing)?,
            Commands::Reorder(args) => handle_reorder(args, timing)?,
            Commands::Convert(args) => handle_convert(cli, args, timing)?,
            Commands::Reduce(args) => handle_reduce(cli, args, timing)?,
        }
    }

    Ok(())
}

/// Reads the given symbolic LTS and prints information about it.
fn handle_info(cli: &Cli, args: &InfoArgs, timing: &Timing) -> Result<(), MercError> {
    let storage = cli.oxidd.init_ldd_manager();

    let format =
        guess_format_from_extension(&args.filename, args.format).ok_or("Cannot determine input symbolic LTS format")?;

    if format != SymFormat::Sym {
        return Err("Currently only the .sym format is supported for info".into());
    }

    let lts = timing.measure("read_symbolic_lts", || {
        read_symbolic_lts(&storage, File::open(&args.filename)?)
    })?;

    println!("Number of states: {}", ldd_len(lts.states(), &mut LddLenCache::new()));
    println!("Number of summand groups: {}", lts.transition_groups().len());

    Ok(())
}

/// Computes the reachable state count of the given symbolic LTS using LDD-based reachability.
fn handle_reachability(cli: &Cli, args: &ReachabilityArgs, timing: &Timing) -> Result<(), MercError> {
    let storage = cli.oxidd.init_ldd_manager();

    let format = guess_format_from_extension(&args.filename, args.format).ok_or("Cannot determine input format")?;

    let options = ReachabilityOptions {
        strategy: args.strategy,
        detect_deadlocks: args.detect_deadlocks,
        // A symbolic LTS read from file has its transition relations already learned in full, so
        // there is no domain worth caching.
        cached: false,
    };

    // Stops the exploration once `--max-iterations` iterations are done, and remembers that it did.
    let mut stopped_after = None;
    let on_iteration = |iteration: usize, _states: &LDDFunction| match args.max_iterations {
        Some(max) if iteration >= max as usize => {
            stopped_after = Some(iteration);
            ControlFlow::Break(())
        }
        _ => ControlFlow::Continue(()),
    };

    let mut file = File::open(&args.filename)?;
    match format {
        SymFormat::Sylvan => {
            if args.output.is_some() {
                return Err("Writing output is only supported for the .sym format".into());
            }

            let mut lts = timing.measure("read_symbolic_lts", || read_sylvan(&storage, &mut file))?;

            let mut context = lts.create_context();
            let result = timing.measure("reachability", || {
                reachability_with_callback(&storage, &mut lts, &mut context, &options, timing, on_iteration)
            })?;

            print_reachable_states(&result.states);
        }
        SymFormat::Sym => {
            let mut lts = timing.measure("read_symbolic_lts", || read_symbolic_lts(&storage, &mut file))?;

            let mut context = lts.create_context();
            let result = timing.measure("reachability", || {
                reachability_with_callback(&storage, &mut lts, &mut context, &options, timing, on_iteration)
            })?;

            print_reachable_states(&result.states);

            if let Some(output) = &args.output {
                lts.set_states(result.states);
                timing.measure("write_symbolic_lts", || {
                    write_symbolic_lts(&storage, File::create(output)?, &lts)
                })?;
            }
        }
    }

    if let Some(iteration) = stopped_after {
        warn!(
            "Stopped after {iteration} iteration(s) because of --max-iterations, the reported states and deadlocks may be incomplete"
        );
    }

    Ok(())
}

/// Prints the number of reachable states, and the number of LDD nodes that represent them.
///
/// The number of nodes is the size of this diagram only (shared nodes and the two terminals are
/// counted once), and not the total number of nodes in the manager, which also holds garbage.
fn print_reachable_states(states: &LDDFunction) {
    println!("LTS has {} states", ldd_len(states, &mut LddLenCache::new()));
    println!(
        "The reachable states use {} LDD nodes",
        LargeFormatter(states.node_count())
    );
}

/// Computes the reachable state count of the given symbolic LTS using BDD-based reachability.
fn handle_reachability_bdd(cli: &Cli, args: &ReachabilityBddArgs, timing: &Timing) -> Result<(), MercError> {
    let storage = cli.oxidd.init_ldd_manager();

    let format = guess_format_from_extension(&args.filename, args.format).ok_or("Cannot determine input format")?;

    if format != SymFormat::Sym {
        // Sylvan format carries no action labels or parameter values, so BDD conversion is not supported.
        return Err("BDD reachability is only supported for the .sym format".into());
    }

    let mut file = File::open(&args.filename)?;
    let lts = timing.measure("read_symbolic_lts", || read_symbolic_lts(&storage, &mut file))?;

    let manager_ref = cli.oxidd.init_bdd_manager();

    let lts_bdd = timing.measure("convert_bdd", || {
        SymbolicLtsBdd::from_symbolic_lts(&storage, &manager_ref, &lts)
    })?;

    if !lts_bdd.initial_state().satisfiable() {
        return Err("BDD conversion produced an unsatisfiable initial state; \
             SymbolicLtsBdd::from_symbolic_lts is likely miscompiling this input"
            .into());
    }

    println!(
        "LTS has {} states",
        timing.measure("reachability_bdd", || -> Result<_, MercError> {
            let reachable_states_bdd = reachability_bdd(&manager_ref, &lts_bdd, args.visualize)?;

            if !reachable_states_bdd.satisfiable() {
                return Err(
                    "BDD reachability returned an unsatisfiable state set despite a satisfiable \
                     initial state; this indicates a reachability_bdd regression"
                        .into(),
                );
            }

            // Use approx_satcount as the number of state variables can exceed 63,
            // which would overflow the u64 accumulator used by sat_count.
            let num_reachable_states_bdd = approx_satcount(
                &reachable_states_bdd,
                lts_bdd.state_variables().len() as u32,
                &mut SatCountCache::new(),
            );
            Ok(num_reachable_states_bdd)
        })?
    );

    Ok(())
}

/// Computes a variable reordering for the output of lpsreach.
fn handle_reorder(args: &ReorderArgs, _timing: &Timing) -> Result<(), MercError> {
    let (kahypar_path, kahypar_ini_path) = args.kahypar.resolve()?;

    if args.filename.extension() == Some(OsStr::new("lps")) {
        // Find lpsreach
        let lpsreach_path = if let Some(path) = &args.mcrl2_path {
            which_in("lpsreach", Some(path), std::env::current_dir()?).map_err(|_e| "Cannot find lpsreach")?
        } else {
            which::which("lpsreach").map_err(|_e| "Cannot find lpsreach in PATH")?
        };

        // Run lpsreach with the --info flag to get dependency information
        let proc = duct::cmd!(lpsreach_path, "--info", &args.filename)
            .stdout_capture()
            .run()
            .map_err(|e| e.to_string())?;

        let graph = parse_compacted_dependency_graph(str::from_utf8(&proc.stdout)?);

        let order = reorder(&kahypar_path, &kahypar_ini_path, &graph)?;
        println!("Computed variable order: {}", order.iter().format(" "));
    } else if args.filename.extension() == Some(OsStr::new("pbes")) {
        // Find pbessolvesymbolic
        let pbessolvesymbolic = if let Some(path) = &args.mcrl2_path {
            which_in("pbessolvesymbolic", Some(path), std::env::current_dir()?)
                .map_err(|_e| "Cannot find pbessolvesymbolic")?
        } else {
            which::which("pbessolvesymbolic").map_err(|_e| "Cannot find pbessolvesymbolic in PATH")?
        };

        // Run pbessolvesymbolic with the --info flag to get dependency information
        let proc = duct::cmd!(pbessolvesymbolic, "--info", &args.filename)
            .stdout_capture()
            .run()
            .map_err(|e| e.to_string())?;

        let graph = parse_compacted_dependency_graph(str::from_utf8(&proc.stdout)?);
        let order = reorder(&kahypar_path, &kahypar_ini_path, &graph)?;

        // Ensure that the first variable is 0 by removing it from order and printing it explicitly
        println!(
            "Computed variable order: 0 {}",
            order.iter().filter(|&x| *x != 0).format(" ")
        );
    } else {
        return Err("Input file must be either a .lps or .pbes file".into());
    }

    Ok(())
}

/// Converts a symbolic LTS to an explicit LTS.
fn handle_convert(cli: &Cli, args: &ConvertArgs, _timing: &Timing) -> Result<(), MercError> {
    let storage = cli.oxidd.init_ldd_manager();

    let format =
        guess_format_from_extension(&args.filename, args.format).ok_or("Cannot determine input symbolic LTS format")?;
    if format != SymFormat::Sym {
        return Err("Currently only the .sym format is supported for conversion".into());
    }

    let output = args
        .output
        .as_ref()
        .ok_or("An output path must be specified with --output; `convert` has nothing to write otherwise.")?;

    let mut file = File::open(&args.filename)?;
    let lts = read_symbolic_lts(&storage, &mut file)?;

    let output_format =
        guess_lts_format_from_extension(output, args.output_format).ok_or("Cannot determine output LTS format")?;

    match output_format {
        LtsFormat::Lts => {
            return Err("Writing LTS format is not yet supported".into());
        }
        LtsFormat::Aut => {
            let mut output = File::create(output)?;
            let mut stream = AutStream::new(&mut output)?;
            convert_symbolic_lts(&storage, &mut stream, &lts)?;
        }
        LtsFormat::AutMcrl2 => {
            let mut output = File::create(output)?;
            let mut stream = AutStream::new_mcrl2(&mut output)?;
            convert_symbolic_lts(&storage, &mut stream, &lts)?;
        }
        LtsFormat::Bcg => {
            let explicit_lts = convert_symbolic_lts(&storage, &mut LtsBuilderMem::new(Vec::new(), Vec::new()), &lts)?;
            write_bcg(&explicit_lts, output)?;
        }
    }

    Ok(())
}

/// Applies reductions to a symbolic LTS.
fn handle_reduce(cli: &Cli, args: &ReduceArgs, timing: &Timing) -> Result<(), MercError> {
    let output = args
        .output
        .as_ref()
        .ok_or("An output path must be specified with --output; `reduce` has nothing to write otherwise.")?;

    let format =
        guess_format_from_extension(&args.filename, args.format).ok_or("Cannot determine input symbolic LTS format")?;
    if format != SymFormat::Sym {
        return Err("Currently only the .sym format is supported for reduction".into());
    }

    let storage = cli.oxidd.init_ldd_manager();
    let manager_ref = cli.oxidd.init_bdd_manager();

    let mut file = File::open(&args.filename)?;
    let lts = read_symbolic_lts(&storage, &mut file)?;

    let lts_bdd = timing.measure("convert_bdd", || {
        SymbolicLtsBdd::from_symbolic_lts(&storage, &manager_ref, &lts)
    })?;

    let quotient_lts = timing.measure(
        "reduction",
        || -> Result<Option<SymbolicLtsBdd<LtsMultiAction<LtsAction>>>, MercError> {
            match args.equivalence {
                Equivalence::StrongBisimSigref => {
                    let (partition, block_vars, _num_of_blocks) = sigref_symbolic(
                        &manager_ref,
                        &lts_bdd,
                        timing,
                        args.split_signature,
                        args.extend_relation,
                        args.merge_transitions,
                        args.visualize,
                    )?;

                    let quotient = timing.measure("quotient", || {
                        quotient_symbolic(&manager_ref, &lts_bdd, &partition, &block_vars)
                    })?;
                    Ok(Some(quotient))
                }
                Equivalence::StrongBisim => {
                    let (partition, block_vars) = refine_bisimulation(&manager_ref, &lts_bdd)?;

                    let quotient = timing.measure("quotient", || {
                        quotient_symbolic(&manager_ref, &lts_bdd, &partition, &block_vars)
                    })?;
                    Ok(Some(quotient))
                }
            }
        },
    )?;

    // Every `Equivalence` variant now produces a quotient (see the `match` above), so this can no
    // longer be `None` in practice; kept as a clear error rather than an `unwrap` in case a future
    // equivalence is added without wiring up its quotient.
    let quotient_lts = quotient_lts.ok_or("Writing the quotient is not yet supported for the selected equivalence")?;

    if guess_format_from_extension(output, None) == Some(SymFormat::Sym) {
        let quotient_ldd = timing.measure("convert_ldd", || quotient_lts.to_symbolic_lts(&storage, &manager_ref))?;

        let mut output_file = File::create(output)?;
        write_symbolic_lts(&storage, &mut output_file, &quotient_ldd)?;
        return Ok(());
    }

    let output_format =
        guess_lts_format_from_extension(output, args.output_format).ok_or("Cannot determine output LTS format")?;

    match output_format {
        LtsFormat::Lts => {
            return Err("Writing LTS format is not yet supported".into());
        }
        LtsFormat::Aut => {
            let mut output = File::create(output)?;
            let mut stream = AutStream::new(&mut output)?;
            convert_symbolic_lts_bdd(&manager_ref, &mut stream, &quotient_lts)?;
        }
        LtsFormat::AutMcrl2 => {
            let mut output = File::create(output)?;
            let mut stream = AutStream::new_mcrl2(&mut output)?;
            convert_symbolic_lts_bdd(&manager_ref, &mut stream, &quotient_lts)?;
        }
        LtsFormat::Bcg => {
            let explicit_lts = convert_symbolic_lts_bdd(
                &manager_ref,
                &mut LtsBuilderMem::new(Vec::new(), Vec::new()),
                &quotient_lts,
            )?;
            write_bcg(&explicit_lts, output)?;
        }
    }

    Ok(())
}
