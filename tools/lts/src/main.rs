use std::fs::File;
use std::io::Write;
use std::io::stdout;
use std::path::Path;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;
use log::info;
use log::warn;

use merc_data::Mcrl2DataSpecification;
use merc_explore::combine_lts;
use merc_io::LargeFormatter;
use merc_lts::GenericLts;
use merc_lts::LTS;
use merc_lts::LtsBuilderMem;
use merc_lts::LtsFormat;
use merc_lts::LtsMultiAction;
use merc_lts::SimpleAction;
use merc_lts::StateIndex;
use merc_lts::apply_lts;
use merc_lts::apply_lts_pair;
use merc_lts::guess_lts_format_from_extension;
use merc_lts::guess_lts_output_format;
use merc_lts::read_explicit_lts;
use merc_lts::read_lts;
use merc_lts::read_mcrl2_aut;
use merc_lts::write_aut;
use merc_lts::write_bcg;
use merc_lts::write_lts;
use merc_lts::write_mcrl2_aut;
use merc_reduction::Equivalence;
use merc_reduction::reduce_lts;
use merc_refinement::ExplorationStrategy;
use merc_refinement::RefinementType;
use merc_refinement::refines;
use merc_syntax::generate_distinguishing_formula;
use merc_syntax::generate_refinement_formula;
use merc_syntax::parse_action_names;
use merc_syntax::parse_allow_action_names;
use merc_syntax::parse_comm_expr_set;
use merc_tools::VerbosityFlag;
use merc_tools::Version;
use merc_tools::VersionFlag;
use merc_tools::format_key_values_json;
use merc_tools::report_error;
use merc_unsafety::print_allocator_metrics;
use merc_utilities::MercError;
use merc_utilities::Timing;

/// Only the mCRL2 binary .lts format carries a real data specification (read from the file
/// itself). The other formats have no notion of one, so a plain default specification would
/// not actually describe the action arguments; reject conversion to LTS format from those
/// until we can construct a proper data specification for them.
const NO_DATA_SPEC_FOR_LTS: &str = "Conversion to LTS format requires a data specification, which is not available when reading this format. This is not yet supported.";

/// A command line tool for labelled transition systems.
#[derive(clap::Parser, Debug)]
#[command(arg_required_else_help = true)]
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
    /// Prints information related to the given LTS.
    Info(InfoArgs),
    /// Reduces the given LTS modulo an equivalent relation.
    Reduce(ReduceArgs),
    /// Compares two LTS modulo an equivalent relation.
    Compare(CompareArgs),
    /// Checks whether the given implementation LTS refines the given specification LTS modulo various refinement relations.
    Refines(RefinesArgs),
    /// Converts an LTS from one format to another format.
    Convert(ConvertArgs),
    /// Computes the parallel composition hide(allow(comm(L1 || ... || Ln))).
    Combine(CombineArgs),
}

#[derive(clap::Args, Debug)]
struct InfoArgs {
    /// Specify the input LTS.
    filename: String,

    /// Explicitly specify the LTS file format.
    #[arg(long)]
    format: Option<LtsFormat>,
}

#[derive(clap::Args, Debug)]
struct ReduceArgs {
    /// Selects the equivalence to reduce the LTS modulo.
    equivalence: Equivalence,

    /// Specify the input LTS.
    filename: PathBuf,

    /// Explicitly specify the LTS file format.
    #[arg(long)]
    format: Option<LtsFormat>,

    /// Specify the output LTS, if not given, output to stdout.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Explicitly specify the output LTS file format; guessed from `--output`'s extension
    /// when not given, defaulting to the AUT format.
    #[arg(long)]
    output_format: Option<LtsFormat>,

    /// Disables preprocessing of the LTS before reducing.
    #[arg(long)]
    no_preprocess: bool,
}

#[derive(clap::Args, Debug)]
struct CompareArgs {
    /// Selects the equivalence to compare the LTSs modulo.
    equivalence: Equivalence,

    /// Specify the input LTS.
    left_filename: PathBuf,

    /// Specify the input LTS.
    right_filename: PathBuf,

    /// If set, outputs a distinguishing formula when the LTSs are not
    /// equivalent. Only supported for naive strong bisimulation.
    #[arg(short = 'c', long)]
    counter_example: Option<PathBuf>,

    /// Explicitly specify the LTS file format.
    #[arg(long)]
    format: Option<LtsFormat>,

    /// Disables preprocessing of the LTSs before checking equivalence.
    #[arg(long)]
    no_preprocess: bool,
}

#[derive(clap::Args, Debug)]
struct ConvertArgs {
    /// Explicitly specify the LTS input file format.
    #[arg(long)]
    format: Option<LtsFormat>,

    /// Specify the input LTS.
    filename: PathBuf,

    /// Explicitly specify the LTS output file format.
    #[arg(long)]
    output_format: Option<LtsFormat>,

    /// Specify the output LTS.
    output: Option<PathBuf>,
}

#[derive(clap::Args, Debug)]
struct RefinesArgs {
    /// Selects the preorder to check for refinement.
    refinement: RefinementType,

    /// Specify the implementation LTS.
    implementation_filename: PathBuf,

    /// Specify the specification LTS.
    specification_filename: PathBuf,

    /// If set, outputs a counter-example when refinement does not hold.
    #[arg(short = 'c', long)]
    counter_example: Option<PathBuf>,

    /// Explicitly specify the LTS file format.
    #[arg(long)]
    format: Option<LtsFormat>,

    /// Disables preprocessing of the LTSs before checking refinement.
    #[arg(long)]
    no_preprocess: bool,
}

#[derive(clap::Args, Debug)]
struct CombineArgs {
    /// The input LTSs for which the parallel composition should be computed.
    #[arg(required=true, num_args=2..)]
    lts: Vec<PathBuf>,

    /// Specify the output LTS, if not given, output to stdout.
    #[arg(long)]
    output: Option<PathBuf>,

    /// Determines the outermost hide operator.
    #[arg(long)]
    hide: Option<String>,

    /// Reads the hide operator from a file.
    #[arg(long)]
    hide_file: Option<PathBuf>,

    /// Determines the action names for the allow operator.
    #[arg(long)]
    allow: Option<String>,

    /// Reads the allow operator from a file.
    #[arg(long)]
    allow_file: Option<PathBuf>,

    /// Determines the communication expressions for the comm operator.
    #[arg(long)]
    comm: Option<String>,

    /// Reads the comm operator from a file.
    #[arg(long)]
    comm_file: Option<PathBuf>,

    /// Determines the number of threads to use for the parallel composition, defaults to one.
    #[arg(long, default_value_t = 1)]
    threads: usize,

    /// Explicitly specify the LTS file format.
    #[arg(long)]
    format: Option<LtsFormat>,

    /// Explicitly specify the output LTS file format; guessed from `--output`'s extension
    /// when not given, defaulting to the mCRL2 AUT dialect.
    #[arg(long)]
    output_format: Option<LtsFormat>,
}

/// Logs the state/transition counts of a reduction result.
fn log_reduced_stats<L: LTS>(lts: &L) {
    info!(
        "Reduced LTS has {} states and {} transitions.",
        LargeFormatter(lts.num_of_states()),
        LargeFormatter(lts.num_of_transitions())
    );
}

/// Writes an untyped (string-labelled) LTS to `output` (or stdout) in `format`. Used for
/// results that have no [`Mcrl2DataSpecification`] to describe their action arguments, so
/// [`LtsFormat::Lts`] is rejected.
fn write_untyped_lts(
    lts: &merc_lts::LabelledTransitionSystem<String>,
    format: LtsFormat,
    output: Option<&Path>,
) -> Result<(), MercError> {
    match format {
        LtsFormat::Aut => match output {
            Some(path) => write_aut(&mut File::create(path)?, lts),
            None => write_aut(&mut stdout(), lts),
        },
        LtsFormat::AutMcrl2 => match output {
            Some(path) => write_mcrl2_aut(&mut File::create(path)?, lts),
            None => write_mcrl2_aut(&mut stdout(), lts),
        },
        LtsFormat::Bcg => {
            let path = output.ok_or("Output path must be specified when writing BCG files.")?;
            write_bcg(lts, path)
        }
        LtsFormat::Lts => Err(NO_DATA_SPEC_FOR_LTS.into()),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    env_logger::Builder::new()
        .filter_level(cli.verbosity.log_level_filter())
        .format_key_values(|formatter, source| format_key_values_json(formatter, source))
        .parse_default_env()
        .init();

    if cli.version.into() {
        eprintln!("{}", Version);
        return ExitCode::SUCCESS;
    }

    let mut timing = Timing::new();
    let result = handle_command(cli.commands, &mut timing);

    if cli.timings {
        timing.print();
    }

    print_allocator_metrics();
    report_error(result)
}

fn handle_command(commands: Option<Commands>, timing: &mut Timing) -> Result<(), MercError> {
    if let Some(command) = &commands {
        match command {
            Commands::Info(args) => {
                handle_info(args, timing)?;
            }
            Commands::Reduce(args) => {
                handle_reduce(args, timing)?;
            }
            Commands::Compare(args) => {
                handle_compare(args, timing)?;
            }
            Commands::Refines(args) => {
                handle_refinement(args, timing)?;
            }
            Commands::Convert(args) => {
                handle_convert(args, timing)?;
            }
            Commands::Combine(args) => {
                handle_combine(args, timing)?;
            }
        }
    }

    Ok(())
}

/// Display information about the given LTS.
fn handle_info(args: &InfoArgs, timing: &mut Timing) -> Result<(), MercError> {
    let path = Path::new(&args.filename);

    let format = guess_lts_format_from_extension(path, args.format).ok_or("Unknown LTS file format.")?;
    let lts = read_explicit_lts(path, format, timing)?;
    println!(
        "LTS has {} states and {} transitions.",
        LargeFormatter(lts.num_of_states()),
        LargeFormatter(lts.num_of_transitions())
    );

    apply_lts!(lts, (), |lts, _data_spec, _| {
        println!("Labels:");
        for label in lts.labels() {
            println!("  {}", label);
        }

        let num_of_silent_transitions = lts.iter_states().fold(0, |acc, s| {
            acc + lts
                .outgoing_transitions(s)
                .filter(|t| lts.is_hidden_label(t.label))
                .count()
        });

        // Count the number of silent transitions.
        println!("Silent transitions: {}", num_of_silent_transitions);

        // Structured output.
        println!("{{\"num_of_silent_transitions\": {}}}", num_of_silent_transitions);
    });

    Ok(())
}

/// Reduce the given LTS into another LTS modulo any of the supported equivalences.
fn handle_reduce(args: &ReduceArgs, timing: &mut Timing) -> Result<(), MercError> {
    let path = Path::new(&args.filename);
    let format = guess_lts_format_from_extension(path, args.format).ok_or("Unknown LTS file format.")?;

    let lts = read_explicit_lts(path, format, timing)?;
    info!(
        "LTS has {} states and {} transitions.",
        LargeFormatter(lts.num_of_states()),
        LargeFormatter(lts.num_of_transitions())
    );

    let output_format = guess_lts_output_format(args.output.as_deref(), args.output_format, LtsFormat::Aut);

    // Only the `Lts` variant carries a data specification, so only it can be written back out
    // in the binary LTS output format; the other variants are relabelled to plain strings.
    match lts {
        GenericLts::Aut(lts) | GenericLts::Bcg(lts) => {
            let reduced_lts = reduce_lts(lts, args.equivalence, !args.no_preprocess, timing);
            log_reduced_stats(&reduced_lts);
            write_untyped_lts(&reduced_lts, output_format, args.output.as_deref())?;
        }
        GenericLts::AutMcrl2(lts) => {
            let reduced_lts = reduce_lts(lts, args.equivalence, !args.no_preprocess, timing);
            log_reduced_stats(&reduced_lts);
            let reduced_lts = reduced_lts.relabel(|label| Ok(label.to_string()))?;
            write_untyped_lts(&reduced_lts, output_format, args.output.as_deref())?;
        }
        GenericLts::Lts(lts, data_spec) => {
            let reduced_lts = reduce_lts(lts, args.equivalence, !args.no_preprocess, timing);
            log_reduced_stats(&reduced_lts);

            if output_format == LtsFormat::Lts {
                match &args.output {
                    Some(path) => write_lts(&mut File::create(path)?, &reduced_lts, &data_spec)?,
                    None => write_lts(&mut stdout(), &reduced_lts, &data_spec)?,
                }
            } else {
                let reduced_lts = reduced_lts.relabel(|label| Ok(label.to_string()))?;
                write_untyped_lts(&reduced_lts, output_format, args.output.as_deref())?;
            }
        }
    }

    Ok(())
}

/// Handles the refinement checking between two LTSs.
fn handle_refinement(args: &RefinesArgs, timing: &mut Timing) -> Result<(), MercError> {
    let impl_path = Path::new(&args.implementation_filename);
    let spec_path = Path::new(&args.specification_filename);
    let format = guess_lts_format_from_extension(impl_path, args.format).ok_or("Unknown LTS file format.")?;

    let impl_lts = read_explicit_lts(impl_path, format, timing)?;
    let spec_lts = read_explicit_lts(spec_path, format, timing)?;

    info!(
        "Implementation LTS has {} states and {} transitions.",
        LargeFormatter(impl_lts.num_of_states()),
        LargeFormatter(impl_lts.num_of_transitions())
    );
    info!(
        "Specification LTS has {} states and {} transitions.",
        LargeFormatter(spec_lts.num_of_states()),
        LargeFormatter(spec_lts.num_of_transitions())
    );

    apply_lts_pair!(impl_lts, spec_lts, timing, |left,
                                                 right,
                                                 _data_specs,
                                                 timing|
     -> Result<(), MercError> {
        let (result, counter_example) = refines(
            left,
            right,
            args.refinement,
            ExplorationStrategy::BFS,
            !args.no_preprocess,
            args.counter_example.is_some(),
            timing,
        );

        if result {
            println!("true");
        } else {
            if let Some(counter_example) = counter_example {
                if let Some(path) = &args.counter_example {
                    // Generate a counterexample formula and output it to the given path.
                    let mut writer = File::create(path)?;
                    writeln!(&mut writer, "{}", generate_refinement_formula(&counter_example))?;
                } else {
                    return Err("Counter example path not provided.".into());
                }
            }

            println!("false");
        }

        Ok(())
    })?;

    Ok(())
}

/// Compares two LTSs for equivalence modulo any of the available equivalences.
fn handle_compare(args: &CompareArgs, timing: &mut Timing) -> Result<(), MercError> {
    if args.counter_example.is_some() && !matches!(args.equivalence, Equivalence::StrongBisimNaive) {
        return Err("Distinguishing formulas are only supported for naive strong bisimulation.".into());
    }

    let format = guess_lts_format_from_extension(&args.left_filename, args.format).ok_or("Unknown LTS file format.")?;

    info!("Assuming format {:?} for both LTSs.", format);
    let left_lts = read_explicit_lts(&args.left_filename, format, timing)?;
    let right_lts = read_explicit_lts(&args.right_filename, format, timing)?;

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

    apply_lts_pair!(left_lts, right_lts, timing, |left,
                                                  right,
                                                  _data_specs,
                                                  timing|
     -> Result<(), MercError> {
        let (equivalent, counter_example) = merc_reduction::compare_lts(
            args.equivalence,
            left,
            right,
            !args.no_preprocess,
            args.counter_example.is_some(),
            timing,
        );

        if equivalent {
            println!("true");
        } else {
            if let Some(formula) = counter_example {
                if let Some(path) = &args.counter_example {
                    // Generate a distinguishing formula and output it to the given path.
                    let mut writer = File::create(path)?;
                    writeln!(&mut writer, "{}", generate_distinguishing_formula(&formula))?;
                } else {
                    return Err("Counter example path not provided.".into());
                }
            }

            println!("false");
        }

        Ok(())
    })?;

    Ok(())
}

/// Converts an LTS from one format to another, does not do any reduction, see [handle_reduce] for that.
fn handle_convert(args: &ConvertArgs, timing: &mut Timing) -> Result<(), MercError> {
    let format = guess_lts_format_from_extension(&args.filename, args.format).ok_or("Unknown LTS file format.")?;
    let input_lts = read_explicit_lts(&args.filename, format, timing)?;

    let output_format = if let Some(output) = &args.output {
        guess_lts_format_from_extension(output, args.output_format).ok_or("Unknown LTS file format.")?
    } else if let Some(format) = args.output_format {
        format
    } else {
        return Err("Either output path or output file format must be specified.".into());
    };

    match input_lts {
        GenericLts::Aut(lts) => match output_format {
            LtsFormat::Bcg => {
                if let Some(path) = &args.output {
                    write_bcg(&lts, path)?;
                } else {
                    return Err("Output path must be specified when writing BCG files.".into());
                }
            }
            LtsFormat::Aut => {
                return Err("Conversion from AUT to AUT is not useful.".into());
            }
            LtsFormat::AutMcrl2 => {
                return Err(format!("Conversion to {output_format:?} format is not yet implemented.").into());
            }
            LtsFormat::Lts => {
                return Err(NO_DATA_SPEC_FOR_LTS.into());
            }
        },
        GenericLts::AutMcrl2(lts) => match output_format {
            LtsFormat::Aut => {
                if let Some(path) = &args.output {
                    write_aut(&mut File::create(path)?, &lts.relabel(|label| Ok(label.to_string()))?)?;
                } else {
                    write_aut(&mut stdout(), &lts.relabel(|label| Ok(label.to_string()))?)?;
                }
            }
            LtsFormat::AutMcrl2 => {
                if let Some(path) = &args.output {
                    write_mcrl2_aut(&mut File::create(path)?, &lts.relabel(|label| Ok(label.to_string()))?)?;
                } else {
                    write_mcrl2_aut(&mut stdout(), &lts.relabel(|label| Ok(label.to_string()))?)?;
                }
            }
            LtsFormat::Bcg => {
                if let Some(path) = &args.output {
                    write_bcg(&lts.relabel(|label| Ok(label.to_string()))?, path)?;
                } else {
                    return Err("Output path must be specified when writing BCG files.".into());
                }
            }
            LtsFormat::Lts => {
                return Err(NO_DATA_SPEC_FOR_LTS.into());
            }
        },
        GenericLts::Lts(lts, data_spec) => match output_format {
            LtsFormat::Aut => {
                if let Some(path) = &args.output {
                    write_aut(&mut File::create(path)?, &lts.relabel(|label| Ok(label.to_string()))?)?;
                } else {
                    write_aut(&mut stdout(), &lts.relabel(|label| Ok(label.to_string()))?)?;
                }
            }
            LtsFormat::AutMcrl2 => {
                if let Some(path) = &args.output {
                    write_mcrl2_aut(&mut File::create(path)?, &lts.relabel(|label| Ok(label.to_string()))?)?;
                } else {
                    write_mcrl2_aut(&mut stdout(), &lts.relabel(|label| Ok(label.to_string()))?)?;
                }
            }
            LtsFormat::Bcg => {
                if let Some(path) = &args.output {
                    write_bcg(&lts.relabel(|label| Ok(label.to_string()))?, path)?;
                } else {
                    return Err("Output path must be specified when writing BCG files.".into());
                }
            }
            LtsFormat::Lts => {
                if let Some(path) = &args.output {
                    write_lts(&mut File::create(path)?, &lts, &data_spec)?;
                } else {
                    write_lts(&mut stdout(), &lts, &data_spec)?;
                }
            }
        },
        GenericLts::Bcg(lts) => match output_format {
            LtsFormat::Aut => {
                if let Some(path) = &args.output {
                    write_aut(&mut File::create(path)?, &lts)?;
                } else {
                    write_aut(&mut stdout(), &lts)?;
                }
            }
            LtsFormat::Lts => {
                return Err(NO_DATA_SPEC_FOR_LTS.into());
            }
            _ => {
                return Err(format!("Conversion to {output_format:?}LTS format is not yet implemented.").into());
            }
        },
    }

    Ok(())
}

fn handle_combine(args: &CombineArgs, timing: &mut Timing) -> Result<(), MercError> {
    let format = guess_lts_format_from_extension(&args.lts[0], args.format).ok_or("Unknown LTS file format.")?;

    // Parse the hide, allow and comm arguments, if they are provided.
    let mut hide = match &args.hide {
        Some(arg) => parse_action_names(arg).map_err(|e| format!("Failed to parse --hide argument:\n{e}"))?,
        None => Vec::new(),
    };

    if let Some(hide_file) = &args.hide_file {
        if !hide_file.exists() {
            return Err(format!("--hide-file path does not exist: {}", hide_file.display()).into());
        }
        if !hide_file.is_file() {
            return Err(format!("--hide-file path is not a file: {}", hide_file.display()).into());
        }
        let contents = std::fs::read_to_string(hide_file)?;
        hide.extend(parse_action_names(&contents).map_err(|e| format!("Failed to parse --hide-file argument:\n{e}"))?);
    }

    let mut allow = match &args.allow {
        Some(arg) => parse_allow_action_names(arg).map_err(|e| format!("Failed to parse --allow argument:\n{e}"))?,
        None => Vec::new(),
    };

    if let Some(allow_file) = &args.allow_file {
        if !allow_file.exists() {
            return Err(format!("--allow-file path does not exist: {}", allow_file.display()).into());
        }
        if !allow_file.is_file() {
            return Err(format!("--allow-file path is not a file: {}", allow_file.display()).into());
        }
        let contents = std::fs::read_to_string(allow_file)?;
        allow.extend(
            parse_allow_action_names(&contents).map_err(|e| format!("Failed to parse --allow-file argument:\n{e}"))?,
        );
    }

    let mut comm = match &args.comm {
        Some(arg) => parse_comm_expr_set(arg).map_err(|e| format!("Failed to parse --comm argument:\n{e}"))?,
        None => Vec::new(),
    };

    if let Some(comm_file) = &args.comm_file {
        if !comm_file.exists() {
            return Err(format!("--comm-file path does not exist: {}", comm_file.display()).into());
        }
        if !comm_file.is_file() {
            return Err(format!("--comm-file path is not a file: {}", comm_file.display()).into());
        }
        let contents = std::fs::read_to_string(comm_file)?;
        comm.extend(parse_comm_expr_set(&contents).map_err(|e| format!("Failed to parse --comm-file argument:\n{e}"))?);
    }

    let output_format = guess_lts_output_format(args.output.as_deref(), args.output_format, LtsFormat::AutMcrl2);

    match format {
        LtsFormat::AutMcrl2 => {
            // The result has no data specification (AutMcrl2 labels are untyped strings), so
            // it cannot be written back out in the binary LTS format.
            if output_format == LtsFormat::Lts {
                return Err(NO_DATA_SPEC_FOR_LTS.into());
            }

            let lts_list = args
                .lts
                .iter()
                .map(|path| -> Result<_, MercError> {
                    let file = File::open(path)?;
                    read_mcrl2_aut(&file)?.relabel(|label| LtsMultiAction::<SimpleAction>::from_string(&label))
                })
                .collect::<Result<Vec<_>, _>>()?;

            let mut builder = LtsBuilderMem::new(Vec::new(), Vec::new());
            combine_lts(&mut builder, lts_list, &hide, &allow, &comm, timing)?;
            let result = builder.finish(StateIndex::new(0), false);

            let result = result.relabel(|label| Ok(label.to_string()))?;
            write_untyped_lts(&result, output_format, args.output.as_deref())?;
        }
        LtsFormat::Aut | LtsFormat::Bcg => {
            return Err(format!("Combining LTSs in {format:?} format is not yet implemented, please convert the LTSs to AutMcrl2 format first.").into());
        }
        LtsFormat::Lts => {
            // Combining itself only needs the LTSs and does not use the data specifications.
            // They are only read here to write the result back out in the binary LTS format
            // below, which requires exactly one; if the inputs' specifications actually differ
            // (distinct sorts/constructors, not just distinct action names), only the first
            // input's is used, so actions originating from the others may come out mistyped.
            let mut data_specs = Vec::with_capacity(args.lts.len());
            let lts_list = args
                .lts
                .iter()
                .map(|path| -> Result<_, MercError> {
                    let file = File::open(path)?;
                    let (lts, spec) = read_lts(&file, false)?;
                    data_specs.push(spec);
                    Ok(lts)
                })
                .collect::<Result<Vec<_>, _>>()?;

            let mut builder = LtsBuilderMem::new(Vec::new(), Vec::new());
            combine_lts(&mut builder, lts_list, &hide, &allow, &comm, timing)?;
            let result = builder.finish(StateIndex::new(0), false);

            if output_format == LtsFormat::Lts {
                if data_specs.len() > 1 {
                    warn!(
                        "Combining {} LTSs into .lts output: only the first input's data specification is kept, so actions from the others may come out mistyped if the specifications differ.",
                        data_specs.len()
                    );
                }
                let data_spec: Mcrl2DataSpecification = data_specs
                    .into_iter()
                    .next()
                    .expect("combine_lts rejects an empty input list");
                match &args.output {
                    Some(output) => write_lts(&mut File::create(output)?, &result, &data_spec)?,
                    None => write_lts(&mut stdout(), &result, &data_spec)?,
                }
            } else {
                let result = result.relabel(|label| Ok(label.to_string()))?;
                write_untyped_lts(&result, output_format, args.output.as_deref())?;
            }
        }
    }

    Ok(())
}
