use duct::cmd;

/// Runs `cargo publish --dry-run` for all crates to verify they can be published.
pub fn publish_crates() {
    let crates = ["utilities", "unsafety"];

    for library in &crates {
        cmd!("cargo", "publish", "--dry-run", format!("-p {}", library))
            .run()
            .expect(&format!("Failed to publish crate {}", library));
    }
}
