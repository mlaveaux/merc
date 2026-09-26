//! Integration test for `merc-sym`'s default `--oxidd-capacity`/`--oxidd-cache-capacity`
//! handling, run as a black-box subprocess invocation since `tools/sym/src/main.rs` has no
//! library surface to unit test directly (same rationale as `tools/lts/tests/convert_cli.rs`).
//!
//! `OxiddArgs` (`crates/symbolic/src/args.rs`) documents `--oxidd-capacity`/
//! `--oxidd-cache-capacity` as being "in gigabytes", and converts the value with
//! `(gib as usize) << 30`. That expression is the *entry count* handed to
//! `oxidd::ldd::new_manager`/`oxidd::bdd::new_manager` (see
//! `oxidd/crates/oxidd/src/ldd.rs::new_manager`, whose first two parameters are
//! `inner_node_capacity`/`apply_cache_capacity`, both node counts, not byte counts), not a byte
//! count divided by the manager's entry size. So requesting the documented default of "1
//! gigabyte" actually asks for `1 << 30` (~1.07 billion) node-table slots and another `1 << 30`
//! apply-cache slots, each entry being tens of bytes wide: tens of gigabytes of real allocation
//! for the *smallest possible* non-zero setting, for every command that touches a decision
//! diagram manager (`info`, `reachability`, `reachability-bdd`, `convert`, `reduce` here; also
//! `merc-lps explore` and `merc-pbes explore-symbolic`/`solve-symbolic`, which flatten the same
//! `OxiddArgs`).
//!
//! On the machine this was found on (15 GiB RAM, no swap) this reliably aborts the process
//! (`memory allocation of 25769803776 bytes failed`) on the default settings, before the tool
//! reads a single byte of its input. There is no working value to pass instead: `--oxidd-capacity
//! 0` makes the manager reject the very first node it needs to create (`Error: OutOfMemory`), and
//! `--oxidd-capacity 1` (the default, and the smallest usable value the `u32`/"whole gigabytes"
//! CLI granularity can express) already asks for ~20-24 GiB. So there is no way, via the CLI
//! alone, to run any `merc-sym` command that needs a decision diagram manager to completion on a
//! machine with less than several tens of gigabytes of free memory (and GitHub Actions' own
//! `ubuntu-latest`/`macos-latest`/`windows-latest` runners have far less than that).
//!
//! This test does not depend on the *particular* amount of physical memory the CI machine
//! happens to have (which would make it flaky): it caps the process's own virtual address space
//! with `ulimit -v` to a generous-but-bounded allowance that a properly-sized default should
//! comfortably fit under for a file this small, well short of the tens of gigabytes the current
//! conversion actually requests. That keeps the test fast and deterministic (the allocation is
//! rejected immediately by the capped address space, so it never touches real memory) rather
//! than dependent on how much RAM+swap the runner happens to have.
//!
//! `ulimit` is POSIX shell functionality with no equivalent invoked this way on Windows, so this
//! is gated to Unix platforms; the underlying defect is platform-independent (it is pure integer
//! arithmetic), so a single platform is sufficient to demonstrate it.
#![cfg(unix)]

use std::process::Command;

/// Runs `merc-sym` with `args`, inside a shell that caps the process's virtual memory to
/// `virtual_memory_limit_kib` KiB via `ulimit -v` before exec'ing the binary. Returns
/// (stdout, stderr, success).
fn run_sym_with_memory_limit(virtual_memory_limit_kib: u64, args: &[&str]) -> (String, String, bool) {
    let bin = env!("CARGO_BIN_EXE_merc-sym");
    let joined_args = args.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ");
    let script = format!(
        "ulimit -v {virtual_memory_limit_kib}; exec {} {}",
        shell_quote(bin),
        joined_args
    );

    let output = Command::new("sh")
        .arg("-c")
        .arg(&script)
        .output()
        .expect("failed to run merc-sym under `sh -c 'ulimit -v ...; exec ...'`");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.success(),
    )
}

/// Minimal single-quote shell escaping, sufficient for the fixed binary path and literal
/// arguments this test passes (none of which contain a single quote).
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// `merc-sym info` on a tiny, valid `.sym` file, with default `--oxidd-capacity`/
/// `--oxidd-cache-capacity`, must not need tens of gigabytes of memory just to start up: capped
/// at a generous 4 GiB of virtual address space (any properly-sized default for a file this size
/// should need a small fraction of that), it must exit successfully and report the file's state
/// count, not abort while allocating the decision diagram manager.
///
/// On the reviewed tree this instead aborts inside `OxiddArgs::init_ldd_manager`
/// (`crates/symbolic/src/args.rs`) while allocating the manager's inner-node table or apply
/// cache -- both sized as `(capacity_gib as usize) << 30` *entries* despite the "gigabytes" name,
/// i.e. tens of gigabytes for the documented default of "1" -- before `handle_info`
/// (`tools/sym/src/main.rs`) ever opens the input file.
#[test]
fn info_with_default_capacity_fits_in_a_few_gib() {
    let example = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/lts/abp.sym");

    // 4 GiB of virtual address space: far more than a correctly-sized default should need to
    // read this small a file, and far less than the ~20-24 GiB the current `(gib as usize) <<
    // 30` conversion actually requests for `--oxidd-capacity`'s default value of 1.
    let (stdout, stderr, success) = run_sym_with_memory_limit(4 * 1024 * 1024, &["info", example]);

    assert!(
        success,
        "`merc-sym info` on a small .sym file with default capacity settings should succeed \
         within 4 GiB of virtual memory, not abort while sizing the decision diagram manager.\n\
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("Number of states"),
        "expected the usual `info` output, got:\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
}
