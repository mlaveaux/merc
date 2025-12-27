use duct::cmd;

/// Runs `cargo publish --dry-run` for all crates to verify they can be published.
pub fn publish_crates() {

    // The list of crates to publish, the order is important due to dependencies.
    let crates = ["merc_utilities", 
        "merc_unsafety",
        "merc_number",
        "merc_io",
        "merc_sharedmutex",];

    for library in &crates {

        // First do a dry run of the publish command to check that everything is fine.
        cmd!("cargo", "publish", "--dry-run", "-p", library)
            .run()
            .expect(&format!("Failed to publish crate {}", library));
    }
}
