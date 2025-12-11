#[cxx::bridge(namespace = "mcrl2::log")]
pub mod ffi {
    unsafe extern "C++" {
        include!("mcrl2-sys/cpp/log.h");
        include!("mcrl2-sys/cpp/exception.h");

        /// Sets the reporting level for mCRL2 utilities logging.
        fn mcrl2_set_reporting_level(level: usize);
    }
}
