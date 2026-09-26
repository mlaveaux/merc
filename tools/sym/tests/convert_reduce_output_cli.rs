//! Integration tests for `merc-sym`'s `convert`/`reduce` `--output` handling, run as black-box
//! subprocess invocations since `tools/sym/src/main.rs` has no library surface to unit test
//! directly (same rationale as `tools/sym/tests/oxidd_default_capacity_cli.rs` and
//! `tools/lts/tests/convert_cli.rs`).
//!
//! Before this fix, `handle_convert`/`handle_reduce` wrapped *all* of their writing logic in
//! `if let Some(output) = &args.output { .. }` with nothing outside that block, so omitting
//! `--output` silently did nothing: the input was read (and, for `reduce`, the reduction was
//! computed) and then thrown away with exit code 0 and no diagnostic. Both subcommands now
//! require `--output` up front and error out clearly instead.

use std::process::Command;

/// Runs the `merc-sym` binary built for this test with the given arguments, returning
/// (stdout, stderr, success).
fn run_sym(args: &[&str]) -> (String, String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_merc-sym"))
        .args(args)
        .output()
        .expect("failed to run merc-sym binary");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

fn abp_sym_path() -> String {
    concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/lts/abp.sym").to_string()
}

/// Builds a unique-ish temporary path ending in `.aut`, so `guess_lts_format_from_extension` can
/// infer the output format from the extension the same way a real invocation would.
fn temp_aut_path(name: &str) -> std::path::PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("merc_sym_test_{}_{}_{}.aut", std::process::id(), name, line!()));
    path
}

/// `convert` with no `--output` must fail with a clear error, not silently do nothing.
#[test]
fn convert_without_output_errors_cleanly() {
    let example = abp_sym_path();

    let (stdout, stderr, success) = run_sym(&["convert", &example]);
    assert!(
        !success,
        "convert with no --output should fail, not silently succeed.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("--output"),
        "expected an error mentioning --output, got:\nstderr:\n{stderr}"
    );
}

/// Sanity check on the companion direction: `convert` with `--output` actually writes the
/// converted LTS.
#[test]
fn convert_with_output_writes_file() {
    let example = abp_sym_path();
    let output = temp_aut_path("convert_out");

    let (stdout, stderr, success) = run_sym(&["convert", &example, "--output", output.to_str().unwrap()]);
    assert!(
        success,
        "convert with --output should succeed.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let contents = std::fs::read_to_string(&output).expect("output file should have been written");
    assert!(
        contents.starts_with("des ("),
        "expected an .aut header, got:\n{contents}"
    );

    let _ = std::fs::remove_file(&output);
}

/// `reduce` with no `--output` must fail with a clear error, not compute the reduction and throw
/// it away silently.
#[test]
fn reduce_without_output_errors_cleanly() {
    let example = abp_sym_path();

    let (stdout, stderr, success) = run_sym(&["reduce", "strong-bisim-sigref", &example]);
    assert!(
        !success,
        "reduce with no --output should fail, not silently succeed.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("--output"),
        "expected an error mentioning --output, got:\nstderr:\n{stderr}"
    );
}

/// Sanity check on the companion direction: `reduce strong-bisim-sigref` with `--output` actually
/// writes the quotient.
#[test]
fn reduce_strong_bisim_sigref_with_output_writes_file() {
    let example = abp_sym_path();
    let output = temp_aut_path("reduce_out");

    let (stdout, stderr, success) = run_sym(&[
        "reduce",
        "strong-bisim-sigref",
        &example,
        "--output",
        output.to_str().unwrap(),
    ]);
    assert!(
        success,
        "reduce strong-bisim-sigref with --output should succeed.\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );

    let contents = std::fs::read_to_string(&output).expect("output file should have been written");
    assert!(
        contents.starts_with("des ("),
        "expected an .aut header, got:\n{contents}"
    );

    let _ = std::fs::remove_file(&output);
}
