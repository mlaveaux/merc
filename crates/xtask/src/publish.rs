use duct::cmd;

/// Runs `cargo publish --dry-run` for all crates to verify they can be published.
pub fn publish_crates() {
    // The list of crates to publish, they must be published in order of dependencies, i.e., downstream first.
    let crates = [
        "merc_utilities",
        "merc_unsafety",
        "merc_number",
        "merc_io",
        "merc_sharedmutex",
        "merc_macros",
        "merc_data",
        "merc_sabre",
        "merc_lts",
        "merc_reduction",
        "merc_vpg",
    ];

    for library in &crates {
        // First do a dry run of the publish command to check that everything is fine.
        cmd!("cargo", "publish", "--dry-run", "-p", library)
            .run()
            .unwrap_or_else(|err| panic!("Failed to publish crate {}: {}", library, err));
    }
}
