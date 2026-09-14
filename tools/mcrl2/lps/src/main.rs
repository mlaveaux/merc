use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;
use log::info;

use merc_explore::CachingStrategy;
use merc_explore::ExplorationStrategy;
use merc_lps::LpsFormat;
use merc_lts::AutFormat;
use merc_lts::AutStream;
use merc_lts::LtsAction;
use merc_lts::LtsFormat;
use merc_lts::LtsMultiAction;
use merc_lts::LtsStream;
use merc_lts::MutexLtsBuilder;
use merc_lts::guess_lts_output_format;
use merc_symbolic::ExplorationStrategy as SymbolicExplorationStrategy;
use merc_symbolic::Order;
use merc_symbolic::ReachabilityOptions;
use merc_symbolic::SummandGrouping;
use merc_symbolic::SymbolicLTS;
use merc_symbolic::SymbolicLpsOptions;
use merc_symbolic::VariableOrder;
use merc_symbolic::parse_order;
use merc_symbolic::write_symbolic_lts;
use merc_tools::KaHyParArgs;
use merc_tools::VerbosityFlag;
use merc_tools::Version;
use merc_tools::VersionFlag;
use merc_tools::report_error;
use merc_unsafety::print_allocator_metrics;
use merc_utilities::MercError;
use merc_utilities::Timing;

use mcrl2::LinearProcessSpecification;
use mcrl2::PreprocessOptions;
use mcrl2::preprocess;
use mcrl2::read_lps;
use mcrl2::read_lps_text;
use mcrl2::set_reporting_level;
use mcrl2::verbosity_to_log_level;

use merc_lps::LtsMultiActionAdapter;
use merc_lps::Mcrl2MultiActionLabel;
use merc_lps::convert_data_specification;
use merc_lps::explore_lps_explicit;
use merc_lps::explore_lps_explicit_parallel;
use merc_lps::explore_lps_symbolic;
use merc_lps::explore_lps_symbolic_to_sym;
use merc_lps_native::explore_lps;
use merc_lps_native::read_lps_file;

/// Default number of nodes for the Oxidd LDD manager.
const DEFAULT_OXIDD_NODE_CAPACITY: usize = 1 << 24;

/// A command line tool for linear process specifications (LPSs)
#[derive(clap::Parser, Debug)]
#[command(arg_required_else_help = true)]
struct Cli {
    #[command(flatten)]
    version: VersionFlag,

    #[command(flatten)]
    verbosity: VerbosityFlag,

    #[arg(long, global = true)]
    timings: bool,

    /// Skip the preprocessing that mCRL2 applies to an LPS before exploring it.
    #[arg(long, global = true, default_value_t = false)]
    no_preprocess: bool,

    /// The number of worker threads for the Oxidd LDD manager.
    #[arg(long, global = true, default_value_t = 1)]
    oxidd_workers: u32,

    /// The number of nodes for the Oxidd LDD manager.
    #[arg(long, global = true, default_value_t = DEFAULT_OXIDD_NODE_CAPACITY)]
    oxidd_node_capacity: usize,

    /// The apply cache capacity for the Oxidd LDD manager, defaults to the node capacity.
    #[arg(long, global = true)]
    oxidd_cache_capacity: Option<usize>,

    #[command(subcommand)]
    commands: Option<Commands>,
}

/// Initializes the Oxidd LDD manager based on CLI arguments.
fn init_ldd_manager(cli: &Cli) -> oxidd::ldd::LDDManagerRef {
    oxidd::ldd::new_manager(
        cli.oxidd_node_capacity,
        cli.oxidd_cache_capacity.unwrap_or(cli.oxidd_node_capacity),
        cli.oxidd_workers,
    )
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Explores the state space of an LPS symbolically
    Explore(ExploreArgs),
    /// Explores the state space of an LPS explicitly
    ExploreExplicit(ExploreExplicitArgs),
    /// Explores the state space of an LPS explicitly, using a native (non-FFI)
    /// binary `.lps` reader and enumerator instead of the mCRL2 toolset.
    ExploreNative(ExploreNativeArgs),
}

/// The input LPS shared by every subcommand.
#[derive(clap::Args, Debug)]
struct InputArgs {
    /// The input LPS file.
    filename: String,

    /// Explicitly choose the format of the input LPS file.
    #[arg(long, short('i'), value_enum)]
    format: Option<LpsFormat>,
}

impl InputArgs {
    /// Reads the LPS in the explicitly chosen format, or the binary LPS format
    /// when no format is given.
    ///
    /// If `preprocess_lps` is true, the LPS is preprocessed using the default
    /// preprocessing options.
    fn read(&self, timing: &Timing, preprocess_lps: bool) -> Result<LinearProcessSpecification, MercError> {
        let lps = timing.measure("load LPS", || match self.format.unwrap_or(LpsFormat::Lps) {
            LpsFormat::Lps => read_lps(&self.filename),
            LpsFormat::Text => read_lps_text(&self.filename),
        })?;

        if preprocess_lps {
            timing.measure("preprocess LPS", || preprocess(&lps, &PreprocessOptions::default()))
        } else {
            info!("Skipping LPS preprocessing (--no-preprocess)");
            Ok(lps)
        }
    }
}

#[derive(clap::Args, Debug)]
struct ExploreArgs {
    #[command(flatten)]
    input: InputArgs,

    /// The strategy used to apply the transition groups during reachability.
    #[arg(long, short('s'), value_enum, default_value_t = SymbolicExplorationStrategy::default())]
    strategy: SymbolicExplorationStrategy,

    /// How the summands are distributed over the transition groups: 'none' (one group per summand),
    /// 'used' (join summands using the same parameters), 'simple' (join summands with the same
    /// read/write pattern) or a partition of the summand indices, e.g. '0; 1 3 4; 2 5'.
    #[arg(long, default_value_t = SummandGrouping::default(), value_parser = parse_grouping)]
    groups: SummandGrouping,

    /// Reorder the process parameters with the MINCE algorithm before exploring
    #[arg(long, default_value_t = Order::None, value_parser = parse_order)]
    reorder: Order,

    #[command(flatten)]
    kahypar: KaHyParArgs,

    /// Detect and report deadlock states (reachable states with no outgoing transition).
    #[arg(long)]
    deadlocks: bool,

    /// Cache the domain of every transition relation, so that successors are only learned for
    /// process parameter values that a group has not seen before.
    #[arg(long)]
    cached: bool,

    /// Write the reachable symbolic LTS to this .sym file, including the data specification,
    /// process parameters, parameter values and action labels. If not given, the LTS is not written.
    #[arg(long, short('o'))]
    output: Option<PathBuf>,
}

impl ExploreArgs {
    /// Returns the variable order to explore with, resolving the KaHyPar tool when `--reorder` is set.
    fn variable_order(&self) -> Result<VariableOrder, MercError> {
        self.reorder.resolve(|| self.kahypar.resolve())
    }
}

/// Parses the `--groups` argument, since [`MercError`] is not a [`std::error::Error`] that clap accepts.
fn parse_grouping(text: &str) -> Result<SummandGrouping, String> {
    text.parse::<SummandGrouping>().map_err(|error| error.to_string())
}

#[derive(clap::Args, Debug)]
struct ExploreExplicitArgs {
    #[command(flatten)]
    input: InputArgs,

    /// Explicitly specify the output LTS file format.
    #[arg(long, short('o'), value_enum)]
    out_format: Option<LtsFormat>,

    /// Specify the output LTS. If not given, the LTS is not written.
    #[arg(long)]
    output: Option<PathBuf>,

    #[arg(long, short('c'), value_enum, default_value_t = CachingStrategy::None)]
    caching: CachingStrategy,

    /// Order in which discovered states are explored.
    #[arg(long, short('s'), value_enum, default_value_t = ExplorationStrategy::Dfs)]
    strategy: ExplorationStrategy,

    /// Use a control flow graph analysis to prune summands whose control flow
    /// guard cannot hold in the current state. The explored transition system is
    /// unchanged.
    #[arg(long)]
    control_flow: bool,

    /// Number of worker threads used for exploration.
    #[arg(long, default_value_t = 1)]
    threads: usize,

    /// Pin each worker thread round-robin to the available CPU cores.
    #[arg(long, default_value_t = false)]
    pinned: bool,
}

/// Arguments for the native (non-FFI) explicit exploration subcommand.
///
/// Unlike [`ExploreExplicitArgs`], there is no `--format`/`--no-preprocess`
/// here: the native reader only understands the binary `.lps` format, and
/// mCRL2's own LPS preprocessing passes are an FFI-only feature this path
/// doesn't perform. There is also no control-flow pruning or parallel
/// exploration yet, since `merc_lps_native::ExploreLinearProcessSpecification`
/// doesn't support either.
#[derive(clap::Args, Debug)]
struct ExploreNativeArgs {
    /// The input .lps file.
    filename: String,

    /// Explicitly specify the output LTS file format.
    #[arg(long, short('o'), value_enum)]
    out_format: Option<LtsFormat>,

    /// Specify the output LTS. If not given, the LTS is not written.
    #[arg(long)]
    output: Option<PathBuf>,

    #[arg(long, short('c'), value_enum, default_value_t = CachingStrategy::None)]
    caching: CachingStrategy,

    /// Order in which discovered states are explored.
    #[arg(long, short('s'), value_enum, default_value_t = ExplorationStrategy::Dfs)]
    strategy: ExplorationStrategy,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    env_logger::Builder::new()
        .filter_level(cli.verbosity.log_level_filter())
        .parse_default_env()
        .init();

    // Enable logging on the mCRL2 side
    set_reporting_level(verbosity_to_log_level(cli.verbosity.verbosity()));

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
    let preprocess_lps = !cli.no_preprocess;

    if let Some(command) = &cli.commands {
        match command {
            Commands::Explore(args) => handle_explore(cli, args, timing, preprocess_lps)?,
            Commands::ExploreExplicit(args) => handle_explore_explicit(args, timing, preprocess_lps)?,
            Commands::ExploreNative(args) => handle_explore_native(args, timing)?,
        }
    }

    Ok(())
}

/// Handles symbolic exploration of an LPS.
fn handle_explore(cli: &Cli, args: &ExploreArgs, timing: &Timing, preprocess_lps: bool) -> Result<(), MercError> {
    let lps = args.input.read(timing, preprocess_lps)?;

    let storage = init_ldd_manager(cli);

    let options = ReachabilityOptions {
        strategy: args.strategy,
        detect_deadlocks: args.deadlocks,
        cached: args.cached,
    };

    let encoding = SymbolicLpsOptions {
        grouping: args.groups.clone(),
        order: args.variable_order()?,
    };

    if let Some(output) = &args.output {
        let (lts, deadlocks) = explore_lps_symbolic_to_sym(&storage, lps, &encoding, &options, timing)?;
        println!("Number of states: {}", lts.states().len());
        if let Some(deadlocks) = &deadlocks {
            println!("Number of deadlocks: {}", deadlocks.len());
        }

        let mut file = BufWriter::new(File::create(output)?);
        write_symbolic_lts(&storage, &mut file, &lts)?;
    } else {
        let result = explore_lps_symbolic(&storage, lps, &encoding, &options, timing)?;
        println!("Number of states: {}", result.states.len());
        if let Some(deadlocks) = &result.deadlocks {
            println!("Number of deadlocks: {}", deadlocks.len());
        }
    }

    Ok(())
}

/// Handles the explicit exploration of an LPS.
fn handle_explore_explicit(args: &ExploreExplicitArgs, timing: &Timing, preprocess_lps: bool) -> Result<(), MercError> {
    let lps = args.input.read(timing, preprocess_lps)?;

    let output_format = guess_lts_output_format(args.output.as_deref(), args.out_format, LtsFormat::Aut);

    // The binary `.lts` format additionally carries a data specification, so it's handled
    // separately (see `handle_explore_explicit_lts`).
    if output_format == LtsFormat::Lts {
        return handle_explore_explicit_lts(args, lps, timing);
    }

    let aut_format = match output_format {
        LtsFormat::Aut => AutFormat::Aut,
        LtsFormat::AutMcrl2 => AutFormat::AutMcrl2,
        LtsFormat::Bcg => {
            return Err(
                "BCG output is not supported by explore-explicit; write AUT or .lts and convert with merc-lts.".into(),
            );
        }
        LtsFormat::Lts => unreachable!("handled above"),
    };

    if args.threads > 1 {
        // Parallel exploration adds transitions concurrently, so the AUT writer
        // is wrapped in a `MutexLtsBuilder` that serialises the writes; the
        // expensive enumeration still happens outside the lock.
        if let Some(output) = &args.output {
            let mut file = BufWriter::new(File::create(output)?);
            let mut builder = MutexLtsBuilder::new(AutStream::with_format(&mut file, aut_format)?);
            explore_lps_explicit_parallel(
                &mut builder,
                lps,
                args.caching,
                args.threads,
                args.control_flow,
                args.pinned,
                timing,
            )?;
        } else {
            // No output requested, discard the explored transitions.
            explore_lps_explicit_parallel(
                &mut (),
                lps,
                args.caching,
                args.threads,
                args.control_flow,
                args.pinned,
                timing,
            )?;
        }
    } else if let Some(output) = &args.output {
        let mut file = BufWriter::new(File::create(output)?);
        let mut builder: AutStream<_, Mcrl2MultiActionLabel> = AutStream::with_format(&mut file, aut_format)?;
        explore_lps_explicit(
            &mut builder,
            lps,
            args.caching,
            args.strategy,
            args.control_flow,
            timing,
        )?;
    } else {
        // No output requested, discard the explored transitions.
        let mut builder: () = ();
        explore_lps_explicit(
            &mut builder,
            lps,
            args.caching,
            args.strategy,
            args.control_flow,
            timing,
        )?;
    }

    Ok(())
}

/// Explores `lps` explicitly and streams the result straight to disk in the binary mCRL2 `.lts`
/// format via [`LtsStream`].
fn handle_explore_explicit_lts(
    args: &ExploreExplicitArgs,
    lps: LinearProcessSpecification,
    timing: &Timing,
) -> Result<(), MercError> {
    let Some(output) = &args.output else {
        // No output requested: still explore (for the stats and timing), but discard the
        // transitions rather than converting and writing them for nothing.
        if args.threads > 1 {
            explore_lps_explicit_parallel(
                &mut (),
                lps,
                args.caching,
                args.threads,
                args.control_flow,
                args.pinned,
                timing,
            )?;
        } else {
            explore_lps_explicit(&mut (), lps, args.caching, args.strategy, args.control_flow, timing)?;
        }
        return Ok(());
    };

    // The data specification never changes during exploration, so convert it once up front,
    // before `lps` is consumed by the explorer below. `LtsStream::new` writes it immediately, as
    // the format's header.
    let data_spec = convert_data_specification(&lps);
    let stream = LtsStream::new(File::create(output)?, &data_spec)?;

    if args.threads > 1 {
        let mut builder = MutexLtsBuilder::new(LtsMultiActionAdapter::new(stream));
        explore_lps_explicit_parallel(
            &mut builder,
            lps,
            args.caching,
            args.threads,
            args.control_flow,
            args.pinned,
            timing,
        )?;
    } else {
        let mut builder = LtsMultiActionAdapter::new(stream);
        explore_lps_explicit(
            &mut builder,
            lps,
            args.caching,
            args.strategy,
            args.control_flow,
            timing,
        )?;
    }

    Ok(())
}

/// Handles the explicit exploration of an LPS via the native (non-FFI) binary
/// `.lps` reader and `merc_enumerate`-backed enumerator.
fn handle_explore_native(args: &ExploreNativeArgs, timing: &Timing) -> Result<(), MercError> {
    let lps = timing.measure("load LPS", || read_lps_file(&args.filename))?;

    let output_format = guess_lts_output_format(args.output.as_deref(), args.out_format, LtsFormat::Aut);

    // The binary `.lts` format additionally carries a data specification, so it's handled
    // separately, as in `handle_explore_explicit_lts`.
    if output_format == LtsFormat::Lts {
        let Some(output) = &args.output else {
            // No output requested: still explore (for the stats and timing), but discard the
            // transitions rather than converting and writing them for nothing.
            explore_lps(&mut (), lps, args.caching, args.strategy, timing)?;
            return Ok(());
        };

        // `LtsStream::new` only borrows the data specification to write it immediately as the
        // format's header, so `lps` is free to move into the explorer right after.
        let mut stream = LtsStream::new(File::create(output)?, &lps.data_spec)?;
        explore_lps(&mut stream, lps, args.caching, args.strategy, timing)?;
        return Ok(());
    }

    let aut_format = match output_format {
        LtsFormat::Aut => AutFormat::Aut,
        LtsFormat::AutMcrl2 => AutFormat::AutMcrl2,
        LtsFormat::Bcg => {
            return Err(
                "BCG output is not supported by explore-native; write AUT or .lts and convert with merc-lts.".into(),
            );
        }
        LtsFormat::Lts => unreachable!("handled above"),
    };

    if let Some(output) = &args.output {
        let mut file = BufWriter::new(File::create(output)?);
        let mut builder: AutStream<_, LtsMultiAction<LtsAction>> = AutStream::with_format(&mut file, aut_format)?;
        explore_lps(&mut builder, lps, args.caching, args.strategy, timing)?;
    } else {
        let mut builder: () = ();
        explore_lps(&mut builder, lps, args.caching, args.strategy, timing)?;
    }

    Ok(())
}
