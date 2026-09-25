use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use clap::Subcommand;
use log::debug;
use log::info;
use log::warn;

use mcrl2::Pbes;
use mcrl2::SrfPbes;
use mcrl2::set_reporting_level;
use mcrl2::verbosity_to_log_level;
use merc_symbolic::ExplorationArgs;
use merc_symbolic::LddLenCache;
use merc_symbolic::OxiddArgs;
use merc_symbolic::ReorderArgs;
use merc_symbolic::SummandGrouping;
use merc_symbolic::SymbolicLpsOptions;
use merc_symbolic::ldd_len;
use merc_tools::VerbosityFlag;
use merc_tools::Version;
use merc_tools::VersionFlag;
use merc_tools::format_key_values_json;
use merc_tools::report_error;
use merc_unsafety::print_allocator_metrics;
use merc_utilities::MercError;
use merc_utilities::Timing;

use merc_pbes::Bsgs;
use merc_pbes::Canonicaliser;
use merc_pbes::GapConfig;
use merc_pbes::ParameterLayoutLPS;
use merc_pbes::PbesLps;
use merc_pbes::PbesSrfLps;
use merc_pbes::PbesVertex;
use merc_pbes::Permutation;
use merc_pbes::QuotientLps;
use merc_pbes::SymmetryAlgorithm;
use merc_pbes::check_parameter_basis;
use merc_pbes::explore_common::UNIFY_IGNORE_CE_EQUATIONS;
use merc_pbes::explore_pbes;
use merc_pbes::explore_pbes_impl;
use merc_pbes::explore_pbes_parallel;
use merc_pbes::explore_pbes_parallel_impl;
use merc_pbes::explore_pbes_symbolic;
use merc_pbes::explore_pbes_symbolic_game;
use merc_pbes::explore_srf_pbes;
use merc_pbes::explore_srf_pbes_parallel;
use merc_pbes::graph_symmetries;
use merc_pbes::symmetry_parameter_basis;
use merc_pbes::write_dot;

use merc_explore::CacheLPS;
use merc_explore::CachingStrategy;
use merc_explore::ExplorationStrategy;
use merc_explore::Summand;
use merc_vpg::ExtendedParityGame;
use merc_vpg::PG;
use merc_vpg::PGBuilder;
use merc_vpg::ParityGame;
use merc_vpg::ParityGameBuilder;
use merc_vpg::Player;
use merc_vpg::Solver;
use merc_vpg::VertexIndex;
use merc_vpg::convert_symbolic_parity_game;
use merc_vpg::solve_priority_promotion;
use merc_vpg::solve_symbolic_zielonka;
use merc_vpg::solve_zielonka;
use merc_vpg::verify_solution;
use merc_vpg::verify_symbolic_strategy;
use merc_vpg::write_pg;

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
enum PbesFormat {
    Text,
    Pbes,
}

/// A command line tool for parameterised boolean equation systems (PBESs)
#[derive(clap::Parser, Debug)]
#[command(arg_required_else_help = true)]
struct Cli {
    #[command(flatten)]
    version: VersionFlag,

    #[command(flatten)]
    verbosity: VerbosityFlag,

    #[arg(long, global = true)]
    timings: bool,

    /// Skip the default preprocessing that is applied to a PBES before instantiating.
    #[arg(long, global = true, default_value_t = false)]
    no_preprocess: bool,

    #[command(flatten)]
    oxidd: OxiddArgs,

    #[command(subcommand)]
    commands: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Print a PBES in textual format.
    Print(PrintArgs),
    /// Analyze symmetries of a PBES using control flow graphs, generally superseded by the graph approach.
    CfgSymmetry(CfgSymmetryArgs),
    /// Compute symmetries of a PBES using symmetry detection graph and GAP.
    GraphSymmetry(GraphSymmetryArgs),
    /// Explore a PBES explicitly into a parity game.
    ExploreExplicit(ExploreExplicitArgs),
    /// Explore a PBES symbolically using LDD-based reachability.
    ExploreSymbolic(ExploreSymbolicArgs),
    /// Solve a PBES by exploring it into a parity game and solving the game.
    Solve(SolveArgs),
    /// Solve a PBES symbolically, by exploring it into a symbolic parity game and solving that
    /// with Zielonka's algorithm.
    SolveSymbolic(SolveSymbolicArgs),
}

/// The PBES to read, shared by every subcommand.
#[derive(clap::Args, Debug)]
struct InputArgs {
    /// The input PBES file.
    filename: String,

    /// Explicitly choose the format of the input PBES file.
    #[arg(long, short('i'), value_enum)]
    format: Option<PbesFormat>,
}

/// How to turn a PBES into a parity game, shared by every subcommand that
/// explores one. `solve` is `explore-explicit` followed by solving the resulting
/// game, so both accept exactly these flags.
#[derive(clap::Args, Debug)]
struct ExploreArgs {
    /// Strategy to explore the state space of the PBES. Only used for sequential
    /// exploration; ignored when `--threads > 1`.
    #[arg(long, value_enum, default_value_t = ExplorationStrategy::Bfs)]
    strategy: ExplorationStrategy,

    /// Caching strategy to use during exploration. Only valid with `--srf`.
    #[arg(long, value_enum, default_value_t = CachingStrategy::None)]
    caching: CachingStrategy,

    /// Number of worker threads used for exploration.
    #[arg(long, default_value_t = 1)]
    threads: usize,

    /// Pin each worker thread round-robin to the available CPU cores.
    #[arg(long, default_value_t = false)]
    pinned: bool,

    /// Apply symmetry reduction: compute graph automorphisms, build a BSGS, and
    /// canonicalize every next-state to its orbit representative before adding
    /// it to the state space.
    #[arg(long, default_value_t = false)]
    symmetry: bool,

    /// Supply generators directly in mapping '[0->1,...]' or cycle '(0 1)'
    /// notation to build the BSGS without running GAP symmetry detection.
    /// Repeat the flag for multiple generators: `--quotient '(0 1)' --quotient '(2 3)'`.
    #[arg(long, value_name = "PERM")]
    quotient: Vec<String>,

    /// Path or name of the GAP executable used to compute the BSGS (only
    /// relevant when `--symmetry` or `--quotient` is set).
    #[arg(long, default_value = "gap")]
    gap_path: String,

    /// Canonicalize next-states in the quotient LPS by asking GAP for the
    /// lex-min (its `Minimum` over the group elements) instead of walking a
    /// precomputed stabilizer chain. This also skips building the BSGS, since
    /// it is not used.
    #[arg(long, default_value_t = false)]
    use_gap_canonicalisation: bool,

    /// Convert to SRF before exploring.
    #[arg(long, default_value_t = false)]
    srf: bool,

    /// Use a control flow graph analysis to prune summands whose source-value
    /// condition cannot hold in the current state. Only meaningful together
    /// with `--srf`.
    #[arg(long)]
    control_flow: bool,

    /// Copy each equation's undeclared parameters through unchanged instead of
    /// resetting them to their sort's default value when unifying parameter
    /// lists. Only meaningful together with `--srf`.
    #[arg(long, default_value_t = false)]
    no_reset: bool,

    /// Write the unified SRF PBES actually explored to this file. Only
    /// meaningful together with `--srf`.
    #[arg(long, value_name = "FILE")]
    dump_srf: Option<PathBuf>,

    /// Write the resulting parity game to this file.
    #[arg(long, short('o'), value_name = "FILE")]
    output: Option<PathBuf>,
}

#[derive(clap::Args, Debug)]
struct PrintArgs {
    #[command(flatten)]
    input: InputArgs,

    /// Write the PBES to this file instead of standard output.
    #[arg(long, short('o'), value_name = "FILE")]
    output: Option<PathBuf>,
}

#[derive(clap::Args, Debug)]
struct CfgSymmetryArgs {
    #[command(flatten)]
    input: InputArgs,

    /// Pass a single permutation in mapping notation '[0->1,1->0,...]' or cycles notation '(0 1)' to check whether it is a symmetry.
    #[arg(long)]
    permutation: Option<String>,

    /// Search for all symmetries instead of only the first one.    
    #[arg(long, default_value_t = false)]
    all_symmetries: bool,

    /// Partition data parameters into their sorts before considering their permutation groups.
    #[arg(long, default_value_t = false)]
    partition_data_sorts: bool,

    /// Partition data parameters based on their updates.
    #[arg(long, default_value_t = false)]
    partition_data_updates: bool,

    /// Print the symmetry in the mapping notation instead of the cycle notation.
    #[arg(long, default_value_t = false)]
    mapping_notation: bool,

    /// Print the SRF representation of the PBES.
    #[arg(long, default_value_t = false)]
    print_srf: bool,
}

#[derive(clap::Args, Debug)]
struct GraphSymmetryArgs {
    #[command(flatten)]
    input: InputArgs,

    /// Path or name of the GAP executable.
    #[arg(long, default_value = "gap")]
    gap_path: String,

    /// Write the generated GAP script to this file (for debugging).
    #[arg(long)]
    dump_gap_script: Option<String>,

    /// Write the SDG as a Graphviz DOT file to this path.
    #[arg(long)]
    dot: Option<String>,

    /// Print symmetries in mapping notation instead of cycle notation.
    #[arg(long, default_value_t = false)]
    mapping_notation: bool,
}

#[derive(clap::Args, Debug)]
struct ExploreExplicitArgs {
    #[command(flatten)]
    input: InputArgs,

    #[command(flatten)]
    explore: ExploreArgs,
}

/// How to encode a PBES symbolically: the summand grouping and variable order, shared by every
/// subcommand that explores one (`explore-symbolic` and `solve-symbolic`), mirroring how
/// [`ExploreArgs`] is shared between `explore-explicit` and `solve`.
#[derive(clap::Args, Debug)]
struct SymbolicExploreArgs {
    #[command(flatten)]
    exploration: ExplorationArgs,

    /// How the equations are distributed over the transition groups: 'none' (one group per equation),
    /// 'used' (join equations using the same parameters), 'simple' (join equations with the same
    /// read/write pattern) or a partition of the equation indices, e.g. '0; 1 3 4; 2 5'.
    #[arg(long, default_value_t = SummandGrouping::default(), value_parser = parse_grouping)]
    groups: SummandGrouping,

    #[command(flatten)]
    reorder: ReorderArgs,

    /// Cache the domain of every transition relation, so that successors are only learned for
    /// parameter values that a group has not seen before.
    #[arg(long)]
    cached: bool,

    /// Reset each equation's undeclared parameters to their sort's default
    /// value instead of copying them through unchanged when unifying
    /// parameter lists.
    #[arg(long, default_value_t = false)]
    reset: bool,

    /// Write the unified SRF PBES actually explored to this file, after
    /// preprocessing, SRF conversion and parameter unification (i.e. exactly
    /// what every other option here sees).
    #[arg(long, value_name = "FILE")]
    dump_srf: Option<PathBuf>,
}

impl SymbolicExploreArgs {
    /// Converts `pbes` to SRF form and unifies its parameter lists according to
    /// `--reset`, writing the result to `--dump-srf` if requested. This is the
    /// exact PBES symbolic exploration then sees, unlike a dump produced by a
    /// separate, local unification.
    fn build_srf(&self, pbes: &Pbes) -> Result<SrfPbes, MercError> {
        let mut srf = SrfPbes::from(pbes)?;
        // `ignore_ce_equations = true`, unlike the explicit explorer: symbolic
        // exploration has no counter-example feature to keep consistent with,
        // so the counter-example equations are dropped from the parameter
        // vector entirely rather than padding it with unused positions.
        srf.unify_parameters(true, self.reset)?;
        if let Some(path) = &self.dump_srf {
            write!(File::create(path)?, "{}", srf.to_pbes())?;
            info!("Unified SRF PBES written to '{}'", path.display());
        }
        Ok(srf)
    }

    /// Returns the encoding options these flags select.
    fn encoding(&self) -> Result<SymbolicLpsOptions, MercError> {
        Ok(SymbolicLpsOptions {
            grouping: self.groups.clone(),
            order: self.reorder.variable_order()?,
        })
    }
}

/// The PBES to explore symbolically, and how its equations are grouped and ordered.
#[derive(clap::Args, Debug)]
struct ExploreSymbolicArgs {
    #[command(flatten)]
    input: InputArgs,

    #[command(flatten)]
    symbolic: SymbolicExploreArgs,
}

/// The PBES to solve symbolically, and how its equations are grouped and ordered.
#[derive(clap::Args, Debug)]
struct SolveSymbolicArgs {
    #[command(flatten)]
    input: InputArgs,

    #[command(flatten)]
    symbolic: SymbolicExploreArgs,

    /// Write the explicit decoding of the symbolic parity game to this file in the PGSolver `.pg`
    /// format, for debugging.
    #[arg(long, short('o'), value_name = "FILE")]
    output: Option<PathBuf>,

    /// Whether to verify the solution after computing it.
    #[arg(long, default_value_t = false)]
    verify_solution: bool,
}

/// Parses the `--groups` argument, since [`MercError`] is not a [`std::error::Error`] that clap accepts.
fn parse_grouping(text: &str) -> Result<SummandGrouping, String> {
    text.parse::<SummandGrouping>().map_err(|error| error.to_string())
}

#[derive(clap::Args, Debug)]
struct SolveArgs {
    #[command(flatten)]
    input: InputArgs,

    #[command(flatten)]
    explore: ExploreArgs,

    /// Sets the algorithm used to solve the resulting parity game.
    #[arg(long, value_enum, default_value_t = Solver::Zielonka)]
    solver: Solver,

    /// Whether to verify the solution after computing it.
    #[arg(long, default_value_t = false)]
    verify_solution: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    env_logger::Builder::new()
        .filter_level(cli.verbosity.log_level_filter())
        .format_key_values(|formatter, source| format_key_values_json(formatter, source))
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
    let preprocess = !cli.no_preprocess;

    if let Some(command) = &cli.commands {
        match command {
            Commands::Print(args) => handle_print(args, timing, preprocess)?,
            Commands::CfgSymmetry(args) => handle_symmetry(args, timing, preprocess)?,
            Commands::GraphSymmetry(args) => handle_graph_symmetry(args, timing, preprocess)?,
            Commands::ExploreExplicit(args) => handle_explore_explicit(args, timing, preprocess)?,
            Commands::ExploreSymbolic(args) => handle_explore_symbolic(cli, args, timing, preprocess)?,
            Commands::Solve(args) => handle_solve(args, timing, preprocess)?,
            Commands::SolveSymbolic(args) => handle_solve_symbolic(cli, args, timing, preprocess)?,
        }
    }

    Ok(())
}

impl InputArgs {
    /// Reads the PBES in the explicitly chosen format, or the binary PBES format
    /// when no format is given.
    ///
    /// Unless `preprocess` is false, the PBES is put through the same preprocessing
    /// that mCRL2 applies before instantiating one. Doing it here rather than inside
    /// a single explorer keeps every consumer of this PBES — the explorers, the
    /// symmetry detection and the parameter basis the generators index into —
    /// looking at the same equations.
    fn read(&self, timing: &Timing, preprocess: bool) -> Result<Pbes, MercError> {
        let mut pbes = timing.measure("load PBES", || match self.format.unwrap_or(PbesFormat::Pbes) {
            PbesFormat::Pbes => Pbes::from_file(&self.filename),
            PbesFormat::Text => Pbes::from_text_file(&self.filename),
        })?;

        if preprocess {
            pbes.preprocess(timing)?;
        } else {
            info!("Skipping PBES preprocessing (--no-preprocess)");
        }

        Ok(pbes)
    }
}

impl ExploreArgs {
    /// Validates the option combinations that do not make sense together,
    /// returning an error for each one rather than silently ignoring a flag.
    fn validate(&self) -> Result<(), MercError> {
        if self.caching != CachingStrategy::None && !self.srf {
            return Err(MercError::from(format!(
                "`--caching {:?}` requires `--srf`: the direct structure-graph explorer cannot benefit \
                 from a cache (see `PbesLps`), only SRF summands have the narrow positional state \
                 effect a cache key can exploit",
                self.caching
            )));
        }
        if self.control_flow && !self.srf {
            return Err(MercError::from(
                "`--control-flow` requires `--srf`: the control flow graph analysis prunes SRF summands \
                 whose source-value condition cannot hold, which only exists for SRF exploration",
            ));
        }
        if self.no_reset && !self.srf {
            return Err(MercError::from(
                "`--no-reset` requires `--srf`: it only changes how SRF parameter lists are unified",
            ));
        }
        if self.dump_srf.is_some() && !self.srf {
            return Err(MercError::from(
                "`--dump-srf` requires `--srf`: there is no SRF PBES to dump for the direct \
                 structure-graph explorer",
            ));
        }
        Ok(())
    }

    /// Converts `pbes` to SRF form and unifies its parameter lists according to
    /// `--no-reset`, writing the result to `--dump-srf` if requested. This is
    /// the exact PBES every other option on this subcommand then explores,
    /// unlike a dump produced by a separate, local unification.
    fn build_srf(&self, pbes: &Pbes) -> Result<SrfPbes, MercError> {
        let mut srf = SrfPbes::from(pbes)?;
        srf.unify_parameters(UNIFY_IGNORE_CE_EQUATIONS, !self.no_reset)?;
        if let Some(path) = &self.dump_srf {
            write!(File::create(path)?, "{}", srf.to_pbes())?;
            info!("Unified SRF PBES written to '{}'", path.display());
        }
        Ok(srf)
    }

    /// Explores `pbes` into a parity game using `builder`, applying symmetry
    /// reduction when generators are supplied or detected.
    ///
    /// Generic over [`PGBuilder`] so a caller that only needs the exploration's
    /// side effects (timing, or the vertex/edge counts the exploration loop
    /// itself logs) can pass `()` and skip materialising the game - see
    /// [`ExploreArgs::write_output`] for writing a real one to `--output`
    /// afterwards.
    fn explore<B: PGBuilder>(&self, pbes: Pbes, timing: &Timing, builder: B) -> Result<B::PG, MercError> {
        // Explicit generators take precedence over detection: giving both means
        // the user already knows the group and only detection would be redundant.
        let canonicaliser = if !self.quotient.is_empty() {
            Some(build_canonicaliser_from_user_generators(
                &pbes,
                &self.quotient,
                &self.gap_path,
                self.use_gap_canonicalisation,
                timing,
            )?)
        } else if self.symmetry {
            Some(build_canonicaliser_for_pbes(
                &pbes,
                &self.gap_path,
                self.use_gap_canonicalisation,
                timing,
            )?)
        } else {
            None
        };

        if let Some(canonicaliser) = canonicaliser {
            explore_with_symmetry(&pbes, self, canonicaliser, timing, builder)
        } else if self.srf {
            let srf = self.build_srf(&pbes)?;
            if self.threads > 1 {
                explore_srf_pbes_parallel(
                    srf,
                    self.threads,
                    self.caching,
                    self.control_flow,
                    self.pinned,
                    timing,
                    builder,
                )
            } else {
                explore_srf_pbes(srf, self.strategy, self.caching, self.control_flow, timing, builder)
            }
        } else if self.threads > 1 {
            explore_pbes_parallel(pbes, self.threads, self.pinned, timing, builder)
        } else {
            explore_pbes(pbes, self.strategy, timing, builder)
        }
    }

    /// Writes `game` to `--output` in PGSolver format, if requested. A no-op
    /// when `--output` was not given.
    fn write_output(&self, game: &ParityGame) -> Result<(), MercError> {
        if let Some(output) = &self.output {
            let mut output_file = File::create(output)?;
            write_pg(&mut output_file, game)?;
            info!("Parity game written to '{}'", output.display());
        }
        Ok(())
    }
}

fn handle_explore_explicit(args: &ExploreExplicitArgs, timing: &Timing, preprocess: bool) -> Result<(), MercError> {
    args.explore.validate()?;
    let pbes = args.input.read(timing, preprocess)?;

    if args.explore.output.is_some() {
        let game = args
            .explore
            .explore(pbes, timing, ParityGameBuilder::new(VertexIndex::new(0)))?;
        args.explore.write_output(&game)?;

        // Reported as log key-values so that `format_key_values_json` renders them as
        // a JSON object next to the human-readable message, which makes the sizes
        // machine-consumable without parsing the message text.
        info!(
            vertices = game.num_of_vertices(),
            edges = game.num_of_edges();
            "Parity game: {} vertices, {} edges",
            game.num_of_vertices(),
            game.num_of_edges()
        );
    } else {
        // No output requested, discard the explored game - the exploration loop
        // itself already logs the vertex/edge counts as it goes.
        args.explore.explore(pbes, timing, ())?;
    }

    Ok(())
}

/// Handles the print command, writing the textual PBES to `--output` or to
/// standard output.
///
/// Like every other subcommand this prints the PBES *after* preprocessing, so
/// that what is shown is what the explorers actually see; `--no-preprocess`
/// prints the PBES as it was read.
fn handle_print(args: &PrintArgs, timing: &Timing, preprocess: bool) -> Result<(), MercError> {
    let pbes = args.input.read(timing, preprocess)?;

    if let Some(output) = &args.output {
        write!(File::create(output)?, "{}", pbes)?;
        info!("PBES written to '{}'", output.display());
    } else {
        println!("{}", pbes);
    }

    Ok(())
}

/// Handles symbolic exploration of a PBES, reporting the number of reachable
/// BES equations (states).
fn handle_explore_symbolic(
    cli: &Cli,
    args: &ExploreSymbolicArgs,
    timing: &Timing,
    preprocess: bool,
) -> Result<(), MercError> {
    let pbes = args.input.read(timing, preprocess)?;
    let storage = cli.oxidd.init_ldd_manager();
    let encoding = args.symbolic.encoding()?;

    let srf_pbes = args.symbolic.build_srf(&pbes)?;
    let states = explore_pbes_symbolic(
        &storage,
        srf_pbes,
        &encoding,
        args.symbolic.exploration.strategy,
        args.symbolic.cached,
        timing,
    )?;
    println!("Number of states: {}", ldd_len(&states, &mut LddLenCache::new()));
    Ok(())
}

/// Handles the solve-symbolic command: explores a PBES symbolically into a symbolic parity game
/// and solves it with Zielonka's algorithm, printing the solution of the initial vertex.
///
/// Prints `winner.solution()` (`"true"`/`"false"`), the same as [`handle_solve`], so the two
/// commands' output is byte-identical and directly comparable.
fn handle_solve_symbolic(
    cli: &Cli,
    args: &SolveSymbolicArgs,
    timing: &Timing,
    preprocess: bool,
) -> Result<(), MercError> {
    let pbes = args.input.read(timing, preprocess)?;
    let storage = cli.oxidd.init_ldd_manager();
    let encoding = args.symbolic.encoding()?;
    let srf_pbes = args.symbolic.build_srf(&pbes)?;

    let symbolic = timing.measure("instantiation", || {
        explore_pbes_symbolic_game(
            &storage,
            srf_pbes,
            &encoding,
            args.symbolic.exploration.strategy,
            args.symbolic.cached,
            args.verify_solution,
            timing,
        )
    })?;

    if let Some(output) = &args.output {
        let (game, _) = convert_symbolic_parity_game(&storage, &symbolic.game, &symbolic.vertices)?;
        let mut output_file = File::create(output)?;
        write_pg(&mut output_file, &game)?;
        info!("Parity game written to '{}'", output.display());
    }

    let epg = ExtendedParityGame::new(symbolic.game, symbolic.initial_vertex, symbolic.sinks);

    // Verifying needs the full winning partition, not just the initial vertex's winner, so it
    // disables the early termination plain solving relies on for speed.
    let (winner, solution) = timing.measure("solve", || solve_symbolic_zielonka(&epg, !args.verify_solution))?;

    if args.verify_solution {
        timing.measure("verify", || {
            verify_symbolic_strategy(&epg.game, &epg.initial_vertex, &solution)
        })?;
    }

    println!("{}", winner.solution());

    Ok(())
}

/// Parse permutation strings in mapping `[0->1,...]` or cycle `(0 1)` notation.
fn parse_generators(strs: &[String]) -> Result<Vec<Permutation>, MercError> {
    strs.iter()
        .map(|s| {
            let s = s.trim();
            if s.starts_with('[') {
                Permutation::from_mapping_notation(s)
            } else {
                Permutation::from_cycle_notation(s)
            }
        })
        .collect()
}
/// Build a canonicaliser from user-supplied generator strings without running
/// graph-symmetry detection. When `use_gap_canonicalisation` is set, the BSGS
/// is skipped entirely and GAP is asked for the lex-min of each state instead.
fn build_canonicaliser_from_user_generators(
    pbes: &Pbes,
    strs: &[String],
    gap_path: &str,
    use_gap_canonicalisation: bool,
    timing: &Timing,
) -> Result<Arc<Canonicaliser>, MercError> {
    let generators = parse_generators(strs)?;
    let n = symmetry_parameter_basis(pbes)?.len();

    // Reject out-of-range points here: converting to a dense permutation would
    // silently truncate them to `0..n`, producing a mapping that is no longer a
    // permutation and panics when inverted.
    for (generator, text) in generators.iter().zip(strs) {
        if let Some(max_point) = generator.max_point()
            && max_point >= n
        {
            return Err(MercError::from(format!(
                "generator '{}' mentions parameter {max_point}, but the PBES has {n} parameter(s) (0..{})",
                text.trim(),
                n.saturating_sub(1)
            )));
        }
    }

    if use_gap_canonicalisation {
        info!(
            "Using GAP lex-min canonicalisation: {} generator(s), {n} parameter(s)",
            generators.len()
        );
        let config = GapConfig {
            executable: gap_path.to_string(),
            dump_script: None,
        };
        Ok(Arc::new(Canonicaliser::gap_lexmin(generators, n, &config)))
    } else {
        let config = GapConfig {
            executable: gap_path.to_string(),
            dump_script: None,
        };
        let bsgs = Arc::new(timing.measure("symmetry: BSGS", || Bsgs::from_generators(&generators, n, &config))?);
        info!(
            "User-supplied generators: |G| = {} ({} generator(s))",
            bsgs.order(),
            generators.len()
        );
        Ok(Arc::new(Canonicaliser::Bsgs(bsgs)))
    }
}

/// Compute graph symmetries for `pbes` and build a canonicaliser from them.
/// When `use_gap_canonicalisation` is set, the BSGS is skipped entirely and GAP
/// is asked for the lex-min of each state instead.
fn build_canonicaliser_for_pbes(
    pbes: &Pbes,
    gap_path: &str,
    use_gap_canonicalisation: bool,
    timing: &Timing,
) -> Result<Arc<Canonicaliser>, MercError> {
    let config = GapConfig {
        executable: gap_path.to_string(),
        dump_script: None,
    };
    let sym_result = timing.measure("symmetry: detection", || graph_symmetries(pbes, &config))?;
    let n = symmetry_parameter_basis(pbes)?.len();

    if use_gap_canonicalisation {
        info!(
            "Using GAP lex-min canonicalisation: |Sym(pbes)| = {}, {} parameter(s)",
            sym_result.symmetry_group_order, n
        );
        Ok(Arc::new(Canonicaliser::gap_lexmin(sym_result.generators, n, &config)))
    } else {
        let bsgs = Arc::new(timing.measure("symmetry: BSGS", || {
            Bsgs::from_generators(&sym_result.generators, n, &config)
        })?);
        info!("|G| = {} ({} generator(s))", bsgs.order(), sym_result.generators.len());

        // The two orders are computed by entirely separate routes — GAP's
        // `Size(Stabilizer(...))` on the detection graph versus the product of the
        // transversal sizes of the stabilizer chain built from the rendered
        // generators — so a disagreement means a generator was rendered, parsed or
        // truncated wrongly somewhere in between. The quotient stays sound either
        // way (canonicalization only ever uses the chain), so warn rather than fail.
        if bsgs.order() != sym_result.symmetry_group_order {
            warn!(
                "symmetry group order mismatch: graph symmetry detection reports |Sym(pbes)| = {}, \
                 but the BSGS built from its generators has order {}; the quotient will reduce by \
                 the smaller group",
                sym_result.symmetry_group_order,
                bsgs.order()
            );
        }
        Ok(Arc::new(Canonicaliser::Bsgs(bsgs)))
    }
}

/// Explore `pbes` into a parity game using `builder`, canonicalizing every
/// next-state via `canonicaliser`.
fn explore_with_symmetry<B: PGBuilder>(
    pbes: &Pbes,
    args: &ExploreArgs,
    canonicaliser: Arc<Canonicaliser>,
    timing: &Timing,
    builder: B,
) -> Result<B::PG, MercError> {
    // Both explorers unify with the same flags as `symmetry_parameter_basis`, so
    // they are expected to agree with it; SRF normalisation is the one that could
    // still add or reorder parameters on the way, since it introduces equations
    // of its own. Checking both keeps the guarantee where it can be seen.
    let basis = symmetry_parameter_basis(pbes)?;

    if args.srf {
        let mut srf = SrfPbes::from(pbes)?;
        // Must unify with exactly the flags `symmetry_parameter_basis` used above,
        // or the generators would index into a different vector than this SRF view
        // lays its state out by (see the module-level comment on this function).
        srf.unify_parameters(UNIFY_IGNORE_CE_EQUATIONS, !args.no_reset)?;
        let lps = PbesSrfLps::new(srf)?;
        check_parameter_basis(&basis, &lps.parameters(), "SRF")?;
        quotient_explore(&lps, args, args.caching, canonicaliser, timing, builder)
    } else {
        let lps = PbesLps::new(pbes.clone())?;
        check_parameter_basis(&basis, &lps.parameters(), "structure-graph")?;
        quotient_explore(&lps, args, CachingStrategy::None, canonicaliser, timing, builder)
    }
}

/// Explore `lps` into a parity game using `builder`, canonicalizing every
/// next-state via `canonicaliser`.
fn quotient_explore<P, B: PGBuilder>(
    lps: &P,
    args: &ExploreArgs,
    caching: CachingStrategy,
    canonicaliser: Arc<Canonicaliser>,
    timing: &Timing,
    builder: B,
) -> Result<B::PG, MercError>
where
    P: ParameterLayoutLPS<Value = usize, Label = (), StateInfo = PbesVertex> + Sync,
    <P::Summand as Summand>::Context: Send,
{
    match caching {
        CachingStrategy::None => {
            let qlps = QuotientLps::new(lps, canonicaliser, 1);
            if args.threads > 1 {
                explore_pbes_parallel_impl(&qlps, args.threads, args.pinned, timing, builder)
            } else {
                explore_pbes_impl(&qlps, args.strategy, timing, builder)
            }
        }
        caching => {
            // The cache sits *inside* the quotient (see [`QuotientLps`]) so the
            // keys stay the narrow read-position projections of the raw states
            // instead of covering every parameter touched by canonicalization.
            let cached = CacheLPS::new(lps, caching);
            let qlps = QuotientLps::new(&cached, canonicaliser, 1);
            let game = if args.threads > 1 {
                explore_pbes_parallel_impl(&qlps, args.threads, args.pinned, timing, builder)
            } else {
                explore_pbes_impl(&qlps, args.strategy, timing, builder)
            }?;
            debug!("{}", cached.metrics());
            Ok(game)
        }
    }
}

/// Handles the solve command, which explores a PBES into a parity game and
/// solves the game, printing the solution of the initial vertex.
fn handle_solve(args: &SolveArgs, timing: &Timing, preprocess: bool) -> Result<(), MercError> {
    args.explore.validate()?;
    // Solving always needs the actual game, unlike plain `explore-explicit`.
    let game = args.explore.explore(
        args.input.read(timing, preprocess)?,
        timing,
        ParityGameBuilder::new(VertexIndex::new(0)),
    )?;
    args.explore.write_output(&game)?;

    let (solution, strategy) = timing.measure("solve", || match args.solver {
        Solver::Zielonka => solve_zielonka(&game, args.verify_solution),
        Solver::PriorityPromotion => solve_priority_promotion(&game, args.verify_solution),
    });

    if let Some(strategy) = strategy
        && args.verify_solution
    {
        verify_solution(&game, &solution, &strategy);
    }

    let winner = if solution[Player::Even.to_index()][game.initial_vertex().value()] {
        Player::Even
    } else {
        Player::Odd
    };
    println!("{}", winner.solution());

    Ok(())
}

fn handle_graph_symmetry(args: &GraphSymmetryArgs, timing: &Timing, preprocess: bool) -> Result<(), MercError> {
    let pbes = args.input.read(timing, preprocess)?;

    let config = GapConfig {
        executable: args.gap_path.clone(),
        dump_script: args.dump_gap_script.as_deref().map(Path::new).map(|p| p.to_path_buf()),
    };

    let result = timing.measure("symmetry: detection", || graph_symmetries(&pbes, &config))?;

    if let Some(dot_path) = &args.dot {
        let mut f = File::create(dot_path)?;
        write_dot(&result.sdg, &mut f)?;
        log::info!("DOT file written to '{dot_path}'");
        if let Ok(dot_bin) = which::which("dot") {
            log::info!("Generating PDF using dot...");
            duct::cmd!(dot_bin, "-Tpdf", dot_path, "-O").run()?;
        }
    }

    for generator in &result.generators {
        if args.mapping_notation {
            println!("{:?}", generator);
        } else {
            println!("{}", generator);
        }
    }

    if result.generators.is_empty() {
        println!("No non-trivial symmetries found.");
    }

    Ok(())
}

fn handle_symmetry(args: &CfgSymmetryArgs, timing: &Timing, preprocess: bool) -> Result<(), MercError> {
    let pbes = args.input.read(timing, preprocess)?;
    let algorithm = SymmetryAlgorithm::new(&pbes, args.print_srf)?;
    if let Some(permutation) = &args.permutation {
        let pi = if permutation.trim_start().starts_with("[") {
            Permutation::from_mapping_notation(permutation)?
        } else {
            Permutation::from_cycle_notation(permutation)?
        };

        if let Err(x) = algorithm.is_valid_permutation(&pi) {
            return Err(format!("The given permutation is not valid: {x}").into());
        }

        info!("Checking permutation: {}", pi);
        if algorithm.check_symmetry(&pi) {
            println!("true");
        } else {
            println!("false");
        }
    } else {
        for candidate in algorithm.candidates(args.partition_data_sorts, args.partition_data_updates) {
            debug!("Found candidate: {}", candidate);

            if candidate.is_identity() {
                // Skip the identity permutation
                continue;
            }

            if algorithm.check_symmetry(&candidate) {
                if args.mapping_notation {
                    info!("Found symmetry: {:?}", candidate);
                } else {
                    info!("Found symmetry: {}", candidate);
                }

                if !args.all_symmetries {
                    // Only search for the first symmetry
                    info!("Stopping search after first non-trivial symmetry.");
                    break;
                }
            }
        }
    }

    Ok(())
}
