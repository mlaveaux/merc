use glob::glob;
use std::env;
use std::error::Error;
use std::fs::create_dir_all;
use std::fs::remove_dir_all;
use std::fs::remove_file;
use std::path::PathBuf;

use duct::cmd;

/// Removes every file matching `pattern`.
///
/// # Errors
///
/// Returns an error if listing or removing a matched file fails.
fn clean_files(pattern: &str) -> Result<(), Box<dyn Error>> {
    let files: Result<Vec<PathBuf>, _> = glob(pattern)?.collect();
    files?.iter().try_for_each(|path| {
        remove_file(path)?;
        Ok(())
    })
}

/// Runs `cargo <arguments>` instrumented for coverage and renders an HTML report with `grcov`.
///
/// Requires the nightly toolchain and `grcov` on `PATH`, and must be run from the workspace
/// root. Deletes and recreates `target/coverage` first, discarding any previous report there;
/// the intermediate `.profraw` files are removed again once the report is generated.
///
/// # Errors
///
/// Returns an error if no arguments are given, if `grcov` or `llvm-profdata` is missing, if the
/// instrumented `cargo` run, `grcov`, or a filesystem operation fails, or if the run produced no
/// profiles or an empty report.
pub(crate) fn coverage(arguments: Vec<String>) -> Result<(), Box<dyn Error>> {
    if arguments.is_empty() {
        return Err("expected a cargo command to run with coverage, e.g. `cargo xtask coverage nextest run`".into());
    }

    // Check this before the (long) instrumented run, instead of failing with a bare `NotFound` afterwards.
    if cmd!("grcov", "--version").stdout_null().stderr_null().run().is_err() {
        return Err("grcov was not found on PATH; install it with `cargo install grcov`".into());
    }

    // grcov only logs an error and writes an empty report when it cannot find `llvm-profdata`, so locate it explicitly.
    let sysroot = cmd!("rustc", "--print", "sysroot")
        .env("RUSTUP_TOOLCHAIN", &toolchain)
        .read()?;
    let llvm_path = glob(&format!("{}/lib/rustlib/*/bin/llvm-profdata*", sysroot.trim()))?
        .next()
        .transpose()?
        .and_then(|path| path.parent().map(PathBuf::from))
        .ok_or_else(|| {
            format!(
                "llvm-profdata was not found in toolchain {toolchain}; install it with \
                 `rustup component add llvm-tools-preview --toolchain {toolchain}`"
            )
        })?;

    // Ignore errors about missing directory.
    let _ = remove_dir_all("target/coverage");
    create_dir_all("target/coverage")?;

    println!("=== running coverage ===");

    // The path from which cargo is called.
    let mut base_directory = env::current_dir()?;
    base_directory.push("target");
    base_directory.push("coverage");

    let mut prof_directory = base_directory.clone();
    prof_directory.push("cargo-test-%p-%m.profraw");

    cmd("cargo", arguments)
        .env("RUSTUP_TOOLCHAIN", &toolchain)
        .env("CARGO_INCREMENTAL", "0")
        .env("RUSTFLAGS", "-C instrument-coverage -Z coverage-options=condition")
        .env("LLVM_PROFILE_FILE", prof_directory)
        .run()?;
    println!("ok.");

    if glob("target/coverage/*.profraw")?.next().is_none() {
        return Err("the cargo command did not write any .profraw files; did it run any tests?".into());
    }

    println!("=== generating report ===");
    let report_dir = "target/coverage/html";
    cmd!(
        "grcov",
        base_directory,
        "--binary-path",
        "./target/debug/deps",
        "-s",
        ".",
        "-t",
        "html",
        "--branch",
        "--llvm-path",
        &llvm_path,
        "--ignore-not-existing",
        "--ignore",
        "**/target/*",
        "-o",
        report_dir,
    )
    .env("RUSTUP_TOOLCHAIN", &toolchain)
    .run()?;

    // grcov exits successfully on most failures, but a report that covers any source file contains a subdirectory for it.
    let is_empty = std::fs::read_dir(report_dir)?
        .filter_map(Result::ok)
        .all(|entry| !entry.path().is_dir() || entry.file_name() == "badges");
    if is_empty {
        return Err("grcov generated an empty report; see its output above".into());
    }
    println!("ok.");

    println!("=== cleaning up ===");
    // The `.profraw` files are written under target/coverage; only clean those.
    clean_files("target/coverage/**/*.profraw")?;
    println!("ok.");

    println!("report location: {report_dir}");

    Ok(())
}
