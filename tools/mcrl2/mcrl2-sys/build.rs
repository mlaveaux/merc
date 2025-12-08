use cargo_emit::rerun_if_changed;
use cc::Build;

fn main() {
    // Path to the mCRL2 location
    let mcrl2_path = String::from("../../../3rd-party/mCRL2/");
    let mcrl2_workarounds_path = String::from("../../../3rd-party/mCRL2-workarounds/");

    #[cfg(feature = "mcrl2_cpptrace")]
    {
        // The debug flags must be set on all the standard libraries used.
        let mut debug_build = Build::new();
        add_debug_defines(&mut debug_build);
        add_compile_flags(&mut debug_build, mcrl2_workarounds_path.clone());

        // Use the `cmake` crate to build cpptrace.
        let mut dst = cmake::Config::new("../../../3rd-party/cpptrace")
            .define("BUILD_SHARED_LIBS", "OFF") // Build a static library.
            .define("CPPTRACE_USE_EXTERNAL_LIBDWARF", "OFF") // Compile libdwarf as part of cpptrace.
            .init_cxx_cfg(debug_build)
            .build();
        dst.push("lib");

        cargo_emit::rustc_link_search!(dst.display() => "native");
        // Link the required libraries for cpptrace (Can this be derived from the cmake somehow?)
        cargo_emit::rustc_link_lib!("cpptrace" => "static");

        // If /usr/lib/x86_64-linux-gnu/libz.a exists, link it statically. (This is not yet portable)
        #[cfg(target_os = "linux")]
        {
            cargo_emit::rustc_link_lib!("dwarf" => "static");
            cargo_emit::rustc_link_lib!("zstd" => "static");

            if std::path::Path::new("/usr/lib/x86_64-linux-gnu/libz.a").exists() {
                cargo_emit::rustc_link_lib!("z" => "static");
                cargo_emit::rustc_link_search!("/usr/lib/x86_64-linux-gnu/" => "native");
            }
        }
    }

    // The mCRL2 source files that we need to build for our Rust wrapper.
    let atermpp_source_files = [
        "aterm_implementation.cpp",
        "aterm_io_binary.cpp",
        "aterm_io_text.cpp",
        "function_symbol.cpp",
        "function_symbol_pool.cpp",
    ];

    let core_source_files = ["dparser.cpp", "core.cpp"];

    let data_source_files = [
        "data.cpp",
        "data_io.cpp",
        "data_specification.cpp",
        "machine_word.cpp",
        "typecheck.cpp",
        "detail/prover/smt_lib_solver.cpp",
        "detail/rewrite/jitty.cpp",
        "detail/rewrite/rewrite.cpp",
        "detail/rewrite/strategy.cpp",
    ];

    let dparser_source_files = [
        "arg.c",
        "parse.c",
        "scan.c",
        "dsymtab.c",
        "util.c",
        "read_binary.c",
        "dparse_tree.c",
    ];

    let lps_source_files = [
        "lps.cpp",
        "lps_io.cpp",
        //"linearise.cpp",
        //"lpsparunfoldlib.cpp",
        //"next_state_generator.cpp",
        //"symbolic_lts_io.cpp",
    ];

    let utilities_source_files = [
        "bitstream.cpp",
        "cache_metric.cpp",
        "logger.cpp",
        //"command_line_interface.cpp",
        "text_utility.cpp",
        "toolset_version.cpp",
    ];

    let pbes_sources_files = [
        "algorithms.cpp",
        "io.cpp",
        "pbes.cpp",
        "pbes_explorer.cpp",
        "pgsolver.cpp",
    ];

    let process_source_files = ["process.cpp"];

    // Build dparser separately since it's a C library.
    let mut build_dparser = cc::Build::new();
    build_dparser
        .include(mcrl2_path.clone() + "3rd-party/dparser/")
        .files(add_prefix(
            mcrl2_path.clone() + "3rd-party/dparser/",
            &dparser_source_files,
        ));

    add_compile_flags(&mut build_dparser, mcrl2_path.clone());
    build_dparser.compile("dparser");

    // These are the files for which we need to call cxxbuild to produce the bridge code.
    let mut build = cxx_build::bridges(["src/atermpp.rs", "src/data.rs", "src/pbes.rs", "src/log.rs"]);

    // Additional files needed to compile the bridge, basically to build mCRL2 itself.
    build
        .cpp(true)
        .std("c++20")
        .define("MCRL2_NO_RECURSIVE_SOUNDNESS_CHECKS", "1") // These checks overflow the stack, and are extremely slow.
        .define("LPS_NO_RECURSIVE_SOUNDNESS_CHECKS", "1")
        .define("MERC_MCRL2_VERSION", "\"internal_merc_build\"") // Sets the mCRL2 version to something recognized as our internal build.
        .includes(add_prefix(
            mcrl2_path.clone(),
            &[
                "3rd-party/dparser/",
                "libraries/atermpp/include",
                "libraries/core/include",
                "libraries/data/include",
                // "libraries/gui/include",
                "libraries/lps/include",
                // "libraries/lts/include",
                // "libraries/modal_formula/include",
                "libraries/pbes/include",
                // "libraries/pg/include",
                // "libraries/pres/include",
                "libraries/process/include",
                // "libraries/smt/include",
                // "libraries/symbolic/include",
                "libraries/utilities/include",
            ],
        ))
        .include(mcrl2_workarounds_path.clone() + "include/")
        .include("../../../3rd-party/boost-include-only/")
        .include("dparser")
        .include(std::env::var("OUT_DIR").unwrap() + "/include/") // This is where cmake generates the headers for cpptrace.
        .files(add_prefix(
            mcrl2_path.clone() + "libraries/atermpp/source/",
            &atermpp_source_files,
        ))
        .files(add_prefix(
            mcrl2_path.clone() + "libraries/core/source/",
            &core_source_files,
        ))
        .files(add_prefix(
            mcrl2_path.clone() + "libraries/data/source/",
            &data_source_files,
        ))
        .files(add_prefix(
            mcrl2_path.clone() + "libraries/lps/source/",
            &lps_source_files,
        ))
        .files(add_prefix(
            mcrl2_path.clone() + "libraries/pbes/source/",
            &pbes_sources_files,
        ))
        .files(add_prefix(
            mcrl2_path.clone() + "libraries/process/source/",
            &process_source_files,
        ))
        .files(add_prefix(
            mcrl2_path.clone() + "libraries/utilities/source/",
            &utilities_source_files,
        ))
        .file("cpp/pbes.cpp")
        .file(mcrl2_workarounds_path.clone() + "mcrl2_syntax.c"); // This is to avoid generating the dparser grammer.

    #[cfg(feature = "mcrl2_jittyc")]
    build.files(add_prefix(
        mcrl2_path.clone() + "libraries/data/source/",
        &["detail/rewrite/jittyc.cpp"],
    ));

    #[cfg(feature = "mcrl2_jittyc")]
    build.define("MCRL2_ENABLE_JITTYC", "1");

    #[cfg(feature = "mcrl2_cpptrace")]
    build.define("MCRL2_ENABLE_CPPTRACE", "1");

    // Enable thread safety since Rust executes its tests at least by default, and allow threading in general.
    build.define("MCRL2_ENABLE_MULTITHREADING", "1");

    // Disable machine numbers since their changes are not compatible with Sabre yet
    build.define("MCRL2_ENABLE_MACHINENUMBERS", "1");

    add_compile_flags(&mut build, mcrl2_path);
    add_debug_defines(&mut build);

    build.compile("mcrl2-sys");

    // These files should trigger a rebuild.
    rerun_if_changed!("cpp/assert.h");
    rerun_if_changed!("cpp/atermpp.h");
    rerun_if_changed!("cpp/exception.h");
    rerun_if_changed!("cpp/data.h");
    rerun_if_changed!("cpp/pbes.h");
    rerun_if_changed!("cpp/pbes.cpp");
    rerun_if_changed!("cpp/log.h");
}

// Enable various additional debug defines based on the current profile.
fn add_debug_defines(build: &mut Build) {
    // Disable assertions and other checks in release mode.
    let profile = std::env::var("PROFILE").expect("cargo should always set this variable");
    match profile.as_str() {
        "debug" => {
            // Debug mode for libc++ (the LLVM standard library)
            build.define("_LIBCPP_DEBUG", "1");
            build.define("_LIBCPP_ENABLE_THREAD_SAFETY_ANNOTATIONS", "1");
            build.define("_LIBCPP_HARDENING_MODE", "_LIBCPP_HARDENING_MODE_DEBUG");
            build.define("_LIBCPP_ABI_BOUNDED_ITERATORS", "1");
            build.define("_LIBCPP_ABI_BOUNDED_ITERATORS_IN_STRING", "1");
            build.define("_LIBCPP_ABI_BOUNDED_ITERATORS_IN_VECTOR", "1");
            build.define("_LIBCPP_ABI_BOUNDED_UNIQUE_PTR", "1");
            build.define("_LIBCPP_ABI_BOUNDED_ITERATORS_IN_STD_ARRAY", "1");

            // // Debug mode for libstdc++ (the GNU standard library)
            build.define("_GLIBCXX_DEBUG", "1");
            build.define("_GLIBCXX_DEBUG_PEDANTIC", "1");
            build.define("_GLIBCXX_ASSERTIONS", "1");
        }
        "release" => {
            build.define("NDEBUG", "1");
        }
        _ => {
            panic!("Unsupported profile {}", profile);
        }
    }
}

/// Add platform specific compile flags and definitions.
#[allow(unused_variables)]
fn add_compile_flags(build: &mut Build, mcrl2_path: String) {
    #[cfg(unix)]
    build
        .flag_if_supported("-Wno-unused-parameter") // I don't care about unused parameters in mCRL2 code.
        .flag_if_supported("-pipe")
        .flag_if_supported("-pedantic")
        .flag_if_supported("c++");

    #[cfg(windows)]
    build
        .include(mcrl2_path + "build/workarounds/msvc") // These are MSVC workarounds that mCRL2 relies on for compilation.
        .flag_if_supported("/EHsc")
        .flag_if_supported("/bigobj")
        .flag_if_supported("/MP")
        .flag_if_supported("/Zc:inline")
        .flag_if_supported("/permissive-")
        .flag_if_supported("/wd4267") // Disable implicit conversion warnings.
        .define("WIN32", "1")
        .define("WIN32_LEAN_AND_MEAN", "1")
        .define("NOMINMAX", "1")
        .define("_USE_MATH_DEFINES", "1")
        .define("_CRT_SECURE_CPP_OVERLOAD_STANDARD_NAMES", "1")
        .define("_CRT_SECURE_NO_WARNINGS", "1")
        .define("BOOST_ALL_NO_LIB", "1");
}

/// \returns A vector of strings where prefix is prepended to every string slice in paths.
fn add_prefix(prefix: String, paths: &[&str]) -> Vec<String> {
    let mut result: Vec<String> = vec![];

    for path in paths {
        result.push(prefix.clone() + path);
    }

    result
}
