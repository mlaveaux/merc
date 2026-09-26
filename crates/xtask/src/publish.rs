use std::error::Error;

use duct::cmd;

/// Runs `cargo publish --dry-run` for all crates to verify they can be published.
pub(crate) fn publish_crates() -> Result<(), Box<dyn Error>> {
    // The list of crates to publish, they must be published in order of dependencies, i.e., downstream first.
    let crates = [
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

    for library in &crates {
        // First do a dry run of the publish command to check that everything is fine.
        cmd!("cargo", "publish", "--dry-run", "-p", library)
            .run()
            .map_err(|err| format!("Failed to publish crate {library}: {err}"))?;
    }

    Ok(())
}
