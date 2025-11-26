use std::fs::File;
use std::process::ExitCode;

use clap::Parser;
use merc_tools::VerbosityFlag;
use merc_tools::Version;
use merc_tools::VersionFlag;
use merc_utilities::MercError;
use merc_vpg::read_pg;

#[derive(clap::Parser, Debug)]
#[command(name = "Maurice Laveaux", about = "A command line variability parity game tool")]
struct Cli {
    #[command(flatten)]
    version: VersionFlag,

    #[command(flatten)]
    verbosity: VerbosityFlag,

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

    let file = File::open(cli.filename)?;
    let vpg = read_pg(file)?;

    println!("Read VPG with {} vertices", vpg.num_of_vertices());

    Ok(ExitCode::SUCCESS)
}
