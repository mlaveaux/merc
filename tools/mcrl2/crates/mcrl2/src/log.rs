use merc_tools::Verbosity;

/// Sets the reporting level for mCRL2 logging.
pub fn set_reporting_level(level: usize) {
    mcrl2_sys::log::ffi::mcrl2_set_reporting_level(level);
}

/// Convert a verbosity to a log level understood by mCRL2
pub fn verbosity_to_log_level_t(verbosity: Verbosity) -> usize {
    match verbosity {
        Verbosity::Quiet => 0,
        Verbosity::Verbose => 5,
        Verbosity::Debug => 6,
        Verbosity::Trace => 7,
    }
}
