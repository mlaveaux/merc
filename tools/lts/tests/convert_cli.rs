//! Integration tests for the `merc-lts` binary's `convert` subcommand, run as
//! black-box subprocess invocations since `tools/lts/src/main.rs` has no
//! library surface to unit test directly.

use std::io::Write;
use std::process::Command;

/// Runs the `merc-lts` binary built for this test with the given arguments,
/// returning (stdout, stderr, success).
fn run_lts(args: &[&str]) -> (String, String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_merc-lts"))
        .args(args)
        .output()
        .expect("failed to run merc-lts binary");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

fn write_temp(name: &str, contents: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    // Unique-ish per test process/thread to avoid collisions when tests run
    // in parallel.
    path.push(format!("merc_lts_test_{}_{}_{}", std::process::id(), name, line!()));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(contents.as_bytes()).unwrap();
    path
}

/// `convert --format aut-mcrl2 ... --output-format aut-mcrl2` on an input
/// with a `tau`-labelled (hidden) transition must round-trip the hidden
/// transition's label as `tau`, the mCRL2 dialect's internal-action label --
/// not silently rewrite it to `i`, the plain Aldebaran dialect's label.
///
/// `handle_convert` (tools/lts/src/main.rs) matches `LtsFormat::Aut |
/// LtsFormat::AutMcrl2 => write_aut(...)` for both the `AutMcrl2` and `Lts`
/// source variants, unconditionally calling `write_aut` (which always uses
/// the Aldebaran `"i"` tau label) instead of dispatching to `write_mcrl2_aut`
/// (which uses `"tau"`) when the *output* format is `AutMcrl2`. This corrupts
/// any conversion that keeps (or moves into) the AutMcrl2 dialect.
#[test]
fn convert_aut_mcrl2_to_aut_mcrl2_preserves_tau_label() {
    let input = write_temp("in.aut", "des (0,1,2)\n(0,\"tau\",1)\n");
    let output = write_temp("out.aut", "");

    let (_, stderr, success) = run_lts(&[
        "convert",
        "--format",
        "aut-mcrl2",
        input.to_str().unwrap(),
        "--output-format",
        "aut-mcrl2",
        output.to_str().unwrap(),
    ]);
    assert!(success, "convert command failed: {stderr}");

    let contents = std::fs::read_to_string(&output).unwrap();
    assert!(
        contents.contains("\"tau\""),
        "AutMcrl2 -> AutMcrl2 conversion must keep the mCRL2 'tau' hidden-action \
         label, got:\n{contents}"
    );
    assert!(
        !contents.contains("\"i\""),
        "AutMcrl2 -> AutMcrl2 conversion must not rewrite the hidden transition \
         to the plain Aldebaran 'i' label, got:\n{contents}"
    );

    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&output);
}

/// Sanity check on the companion direction: converting an `AutMcrl2` source
/// down to plain `Aut` output legitimately uses `"i"` -- this confirms the
/// bug above is specifically about the *output* format being `AutMcrl2`, not
/// that `write_aut`/`"i"` is always wrong.
#[test]
fn convert_aut_mcrl2_to_aut_uses_i_label() {
    let input = write_temp("in2.aut", "des (0,1,2)\n(0,\"tau\",1)\n");
    let output = write_temp("out2.aut", "");

    let (_, stderr, success) = run_lts(&[
        "convert",
        "--format",
        "aut-mcrl2",
        input.to_str().unwrap(),
        "--output-format",
        "aut",
        output.to_str().unwrap(),
    ]);
    assert!(success, "convert command failed: {stderr}");

    let contents = std::fs::read_to_string(&output).unwrap();
    assert!(
        contents.contains("\"i\""),
        "AutMcrl2 -> Aut must use the plain Aldebaran 'i' label, got:\n{contents}"
    );

    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&output);
}

/// `convert` with neither `--output-format` nor a positional output path
/// must fail with a clear error, not panic.
#[test]
fn convert_without_output_or_output_format_errors_cleanly() {
    let input = write_temp("in3.aut", "des (0,1,2)\n(0,\"a\",1)\n");

    let (_, stderr, success) = run_lts(&["convert", "--format", "aut", input.to_str().unwrap()]);
    assert!(!success, "convert with no output path/format should fail");
    assert!(
        stderr.contains("Either output path or output file format must be specified"),
        "unexpected stderr: {stderr}"
    );

    let _ = std::fs::remove_file(&input);
}

/// `info` on a nonexistent file must return a clean error, not panic.
#[test]
fn info_on_missing_file_errors_cleanly() {
    let (_, _, success) = run_lts(&["info", "/nonexistent/path/does_not_exist.aut"]);
    assert!(!success, "info on a missing file should fail, not succeed");
}

/// Running with no subcommand at all (arg_required_else_help) must not panic
/// and must exit non-zero.
#[test]
fn no_subcommand_prints_help_and_exits_nonzero() {
    let (_, _, success) = run_lts(&[]);
    assert!(!success, "running with no subcommand should not exit successfully");
}
