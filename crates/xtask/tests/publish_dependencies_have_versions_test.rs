//! Regression test for `cargo xtask publish` (`crates/xtask/src/publish.rs`).
//!
//! `publish_crates` runs `cargo publish --dry-run -p <crate>` for a hard-coded list of crates,
//! documented as "for every library crate, in dependency order". `cargo publish` requires every
//! non-dev dependency of a published crate - including optional ones - to carry an explicit
//! `version` in the workspace's dependency table (a bare `path` dependency is rejected: "all
//! dependencies must have a version requirement specified when publishing"). If a crate in the
//! list has an un-versioned workspace-local dependency, the dry run fails immediately, before it
//! verifies anything about that crate's own code.
//!
//! This test statically re-derives, from the actual `Cargo.toml` files, whether that precondition
//! holds for every crate `publish_crates` tries to publish - without shelling out to `cargo
//! publish` itself, which needs network access to crates.io and is therefore unsuitable for a
//! hermetic regression test. It currently fails for two crates that ARE dry-run: `merc_symbolic`
//! (depends on the un-versioned, optional `merc_tools`) and `merc_sabre` (depends on the
//! un-versioned `merc_typecheck`) - confirmed directly with
//! `cargo publish --dry-run -p merc_symbolic` / `-p merc_sabre`, both of which fail with exactly
//! that error on the current tree.
//!
//! It also flags `publish_crates`'s list containing a crate name with no matching package at all
//! (`merc_ldd` - there is no such crate anywhere in the workspace), which fails
//! `cargo publish --dry-run -p merc_ldd` with "package ID specification `merc_ldd` did not match
//! any packages" - confirmed directly the same way. Since the list is walked in order and
//! `merc_sabre` sits earlier in it than `merc_ldd`, `cargo xtask publish` currently cannot get
//! past its 14th entry (of 17) at all.
//!
//! The crate list below is a manual copy of `publish_crates`'s own list (that function is
//! `pub(crate)` and its list is a local variable, so it cannot be imported here without changing
//! production code) - keep the two in sync.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

/// Mirrors the crate list in `crates/xtask/src/publish.rs::publish_crates`.
const PUBLISH_CRATES: &[&str] = &[
    "merc_utilities",
    "merc_number",
    "merc_io",
    "merc_collections",
    "merc_unsafety",
    "merc_sharedmutex",
    "merc_macros",
    "merc_aterm",
    "merc_data",
    "merc_lts",
    "merc_reduction",
    "merc_refinement",
    "merc_syntax",
    "merc_sabre",
    "merc_symbolic",
    "merc_vpg",
];

fn workspace_root() -> PathBuf {
    // crates/xtask -> workspace root is two levels up.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Extracts the leading `merc_...` crate-name token from a TOML key/value line, e.g.
/// `merc_tools = { path = "crates/tools" }` -> `Some("merc_tools")`, or
/// `merc_typecheck.workspace = true` -> `Some("merc_typecheck")`.
fn merc_crate_name(line: &str) -> Option<&str> {
    let line = line.trim_start();
    if !line.starts_with("merc_") {
        return None;
    }
    let end = line.find(['.', '=', ' ', '\t']).unwrap_or(line.len());
    Some(&line[..end])
}

/// Returns the lines of `text` between a `[section]` header line and the next `[`-headed line (or
/// end of file), not including the header itself.
fn section_lines<'a>(text: &'a str, section: &str) -> Vec<&'a str> {
    let mut lines = text.lines();
    for line in lines.by_ref() {
        if line.trim() == section {
            break;
        }
    }
    lines.take_while(|line| !line.trim_start().starts_with('[')).collect()
}

/// Parses the root `Cargo.toml`'s `[workspace.dependencies]` table: for every `merc_*` entry,
/// whether its line contains a `version` key.
fn workspace_dependency_has_version() -> HashMap<String, bool> {
    let root_manifest = fs::read_to_string(workspace_root().join("Cargo.toml")).expect("root Cargo.toml must exist");

    let mut result = HashMap::new();
    for line in section_lines(&root_manifest, "[workspace.dependencies]") {
        if let Some(name) = merc_crate_name(line) {
            result.insert(name.to_string(), line.contains("version"));
        }
    }
    result
}

/// Discovers every `crates/*/Cargo.toml`, mapping each package's declared `name` to the set of
/// `merc_*` crates it depends on non-optionally-excluded, i.e. everything under `[dependencies]`
/// (which is what `cargo publish` must version-check; `[dev-dependencies]` is exempt).
fn package_dependencies() -> HashMap<String, Vec<String>> {
    let crates_dir = workspace_root().join("crates");
    let mut result = HashMap::new();

    for entry in fs::read_dir(&crates_dir).expect("crates/ directory must exist") {
        let entry = entry.expect("readable directory entry");
        let manifest_path = entry.path().join("Cargo.toml");
        let Ok(manifest) = fs::read_to_string(&manifest_path) else {
            continue;
        };

        let name = section_lines(&manifest, "[package]")
            .into_iter()
            .find_map(|line| {
                let line = line.trim();
                line.strip_prefix("name")
                    .map(|rest| rest.trim_start().trim_start_matches('='))
                    .map(|rest| rest.trim().trim_matches('"').to_string())
            })
            .unwrap_or_else(|| panic!("{manifest_path:?} has no [package] name"));

        let deps: Vec<String> = section_lines(&manifest, "[dependencies]")
            .into_iter()
            .filter_map(merc_crate_name)
            .map(str::to_string)
            .collect();

        result.insert(name, deps);
    }

    result
}

/// Every crate `publish_crates` dry-runs must have every one of its `[dependencies]` -
/// `cargo publish`'s manifest check does not exempt optional dependencies - carry a `version` in
/// the workspace dependency table, or the dry run fails before checking anything else about that
/// crate.
///
/// Fails on the current tree: `merc_symbolic` depends on `merc_tools` (no version) and
/// `merc_sabre` depends on `merc_typecheck` (no version), both reproduced directly with
/// `cargo publish --dry-run -p <crate>`.
#[test]
fn test_publish_crates_dependencies_all_have_versions() {
    let has_version = workspace_dependency_has_version();
    let deps = package_dependencies();

    let mut missing_versions: Vec<String> = Vec::new();

    for &krate in PUBLISH_CRATES {
        let Some(crate_deps) = deps.get(krate) else {
            // `cargo publish -p <krate>` fails outright ("package ID specification ... did not
            // match any packages") before it would ever reach a version check.
            missing_versions.push(format!(
                "{krate}: not a package in this workspace at all (no crates/*/Cargo.toml declares it)"
            ));
            continue;
        };

        for dep in crate_deps {
            match has_version.get(dep) {
                Some(true) => {}
                Some(false) => missing_versions.push(format!("{krate} -> {dep} (no version in workspace deps)")),
                None => missing_versions.push(format!("{krate} -> {dep} (not in [workspace.dependencies] at all)")),
            }
        }
    }

    assert!(
        missing_versions.is_empty(),
        "cargo publish --dry-run will fail its manifest check for these dependency edges \
         (every dependency of a crate `cargo xtask publish` publishes must have a `version` in \
         [workspace.dependencies]):\n{}",
        missing_versions.join("\n")
    );
}
