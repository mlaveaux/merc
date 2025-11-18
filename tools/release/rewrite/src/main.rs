use std::fs::File;
use std::io::Write;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;

use merc_gui::verbosity::VerbosityFlag;
use merc_rec_tests::load_rec_from_file;
use merc_unsafety::print_allocator_metrics;
use merc_utilities::MercError;
use merc_version::Version;

use merc_rewrite::TrsFormatter;
use merc_rewrite::Rewriter;
use merc_rewrite::rewrite_rec;

mod trs_format;

#[derive(clap::Parser, Debug)]
#[command(name = "Maurice Laveaux", about = "A command line rewriting tool")]
struct Cli {
    #[arg(
        long,
        global = true,
        default_value_t = false,
        help = "Print the version of this tool"
    )]
    version: bool,

    #[command(flatten)]
    verbosity: VerbosityFlag,

    #[command(subcommand)]
    commands: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Rewrite(RewriteArgs),
    Convert(ConvertArgs),
}

#[derive(clap::Args, Debug)]
#[command(about = "Rewrite mCRL2 data specifications and REC files")]
struct RewriteArgs {
    rewriter: Rewriter,

    #[arg(value_name = "SPEC")]
    specification: String,

    #[arg(help = "File containing the terms to be rewritten.")]
    terms: Option<String>,

    #[arg(long = "output", default_value_t = false, help = "Print the rewritten term(s)")]
    output: bool,
}

#[derive(clap::Args, Debug)]
#[command(about = "Convert input rewrite system to the TRS format")]
struct ConvertArgs {
    #[arg(value_name = "SPEC")]
    specification: String,

    output: String,
}

fn main() -> Result<ExitCode, MercError> {
    let cli = Cli::parse();

    env_logger::Builder::new()
        .filter_level(cli.verbosity.log_level_filter())
        .parse_default_env()
        .init();

    if cli.version {
        eprintln!("{}", Version);
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(command) = cli.commands {
        match command {
            Commands::Rewrite(args) => {
                if args.specification.ends_with(".rec") {
                    assert!(args.terms.is_none());
                    rewrite_rec(args.rewriter, &args.specification, args.output)?;
                }
            }
            Commands::Convert(args) => {
                if args.specification.ends_with(".rec") {
                    // Read the data specification
                    let (spec_text, _) = load_rec_from_file(args.specification.into())?;
                    let spec = spec_text.to_rewrite_spec();

                    let mut output = File::create(args.output)?;
                    write!(output, "{}", TrsFormatter::new(&spec))?;
                }
            }
        }
    }

    print_allocator_metrics();
    Ok(ExitCode::SUCCESS)
}
