use clap::Args;
use log::LevelFilter;

#[derive(Args, Debug)]
pub struct VerbosityFlag {
    #[arg(short, long, global = true, help = "Set the verbosity to quiet")]
    quiet: bool,

    #[arg(short, long, global = true, help = "Set the verbosity to verbose")]
    verbose: bool,

    #[arg(short, long, global = true, help = "Set the verbosity to debug")]
    debug: bool,

    #[arg(short, long, global = true, help = "Set the verbosity to trace")]
    trace: bool,
}

impl VerbosityFlag {
    /// Returns the log level filter corresponding to the given verbosity flags.
    pub fn log_level_filter(&self) -> LevelFilter {
        let verbosity: Verbosity = self.into();
        verbosity.log_level_filter()
    }
}

#[derive(Debug, Clone)]
enum Verbosity {
    Quiet,
    Verbose,
    Debug,
    Trace,
}

impl std::fmt::Display for Verbosity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Verbosity::Quiet => write!(f, "quiet"),
            Verbosity::Verbose => write!(f, "verbose"),
            Verbosity::Debug => write!(f, "debug"),
            Verbosity::Trace => write!(f, "trace"),
        }
    }
}

impl Verbosity {
    /// Returns the log filter level corresponding to this verbosity.
    pub fn log_level_filter(&self) -> LevelFilter {
        match self {
            Verbosity::Quiet => LevelFilter::Off,
            Verbosity::Verbose => LevelFilter::Info,
            Verbosity::Debug => LevelFilter::Debug,
            Verbosity::Trace => LevelFilter::Trace,
        }
    }
}

impl From<&VerbosityFlag> for Verbosity {
    fn from(flag: &VerbosityFlag) -> Self {
        if flag.quiet {
            Verbosity::Quiet
        } else if flag.trace {
            Verbosity::Trace
        } else if flag.debug {
            Verbosity::Debug
        } else if flag.verbose {
            Verbosity::Verbose
        } else {
            Verbosity::Verbose
        }
    }
}
