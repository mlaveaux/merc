//!
//! Package command for creating release distributions.
//!

use duct::cmd;
use std::env;
use std::error::Error;
use std::fs::copy;
use std::fs::create_dir_all;

/// Builds the project in release mode and packages specified binaries.
///
/// This function performs the following operations:
/// 1. Runs `cargo build --release` to build optimized binaries
/// 2. Creates a 'package' directory in the workspace root
/// 3. Copies the binaries ltsgraph, ltsinfo, and mercrewrite to the package directory
///
/// The design choice to use a dedicated package directory ensures clean separation
/// of packaged artifacts from build artifacts, making distribution easier.
pub fn package() -> Result<(), Box<dyn Error>> {
    // Get the workspace root directory
    let workspace_root = env::current_dir()?;

    // Precondition: Ensure we're in a valid Rust workspace
    debug_assert!(
        workspace_root.join("Cargo.toml").exists(),
        "Must be run from workspace root containing Cargo.toml"
    );

    println!("=== Building release binaries ===");

    // Build all binaries in release mode
    // Using release profile for optimized performance in distribution
    cmd!("cargo", "build", "--release").dir(&workspace_root).run()?;

    println!("=== Creating package directory ===");

    // Create package directory for distribution artifacts
    let package_dir = workspace_root.join("package");
    create_dir_all(&package_dir)?;

    println!("=== Copying binaries to package directory ===");

    // Binary names to package - these are the core tools for distribution
    let binary_names = ["ltsgraph", "ltsinfo", "mercrewrite"];
    let target_release_dir = workspace_root.join("target").join("release");

    // Copy each binary with appropriate extension for Windows
    for binary_name in &binary_names {
        let source_path = if cfg!(windows) {
            target_release_dir.join(format!("{binary_name}.exe"))
        } else {
            target_release_dir.join(binary_name)
        };

        let dest_path = if cfg!(windows) {
            package_dir.join(format!("{binary_name}.exe"))
        } else {
            package_dir.join(binary_name)
        };

        // Precondition: Binary must exist after successful build
        debug_assert!(
            source_path.exists(),
            "Binary {binary_name} should exist after cargo build --release"
        );

        copy(&source_path, &dest_path)?;
        println!("Copied {binary_name} to package directory");
    }

    println!("=== Package creation completed ===");
    println!("Package directory: {}", package_dir.display());

    // Postcondition: All required binaries should be in package directory
    debug_assert!(
        binary_names.iter().all(|name| {
            let expected_path = if cfg!(windows) {
                package_dir.join(format!("{name}.exe"))
            } else {
                package_dir.join(name)
            };
            expected_path.exists()
        }),
        "All binaries should be copied to package directory"
    );

    Ok(())
}
