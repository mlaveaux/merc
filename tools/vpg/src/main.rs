use std::fs::File;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;

use merc_tools::VerbosityFlag;
use merc_tools::Version;
use merc_tools::VersionFlag;
use merc_utilities::MercError;
use merc_vpg::read_pg;
use merc_vpg::solve_zielonka;

#[derive(clap::Parser, Debug)]
#[command(name = "Maurice Laveaux", about = "A command line variability parity game tool")]
struct Cli {
    #[command(flatten)]
    version: VersionFlag,

    #[command(flatten)]
    verbosity: VerbosityFlag,
    
    #[command(subcommand)]
    commands: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Solve(SolveArgs),
}

#[derive(clap::Args, Debug)]
struct SolveArgs {    
    filename: String,
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

    if let Some(command) = cli.commands {
        match command {
            Commands::Solve(args) => {
                let mut file = File::open(&args.filename)?;
                let game = read_pg(&mut file)?;
                
                println!("{}", solve_zielonka(&game).solution())
            }
        }
    }


    Ok(ExitCode::SUCCESS)
}
