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

    println!("=== Creating package directory ===");

    // Create package directory for distribution artifacts
    let package_dir = workspace_root.join("package");
    create_dir_all(&package_dir)?;

    println!("=== Building and copying release binaries ===");

    // Mapping from workspace paths to their binaries
    let workspace_binaries = [
        (workspace_root.clone(), vec!["merc-lts", "merc-rewrite", "merc-vpg"]),
        (workspace_root.join("tools/gui"), vec!["merc-ltsgraph"]),
        (workspace_root.join("tools/mcrl2"), vec!["merc-pbes"]),
    ];

    // Build all workspaces in release mode
    // Using release profile for optimized performance in distribution
    for (workspace_path, binaries) in &workspace_binaries {
        cmd!("cargo", "build", "--release").dir(workspace_path).run()?;
        
        let target_release_dir = workspace_path.join("target").join("release");

        for binary_name in binaries {
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

            // Copy debug symbols if they exist
            #[cfg(target_os = "linux")]
            {
                let dwp_source = target_release_dir.join(format!("{binary_name}.dwp"));
                if dwp_source.exists() {
                    let dwp_dest = package_dir.join(format!("{binary_name}.dwp"));
                    copy(&dwp_source, &dwp_dest)?;
                    println!("Copied {binary_name}.dwp to package directory");
                }
            }

            #[cfg(target_os = "macos")]
            {
                let dsym_source = target_release_dir.join(format!("{binary_name}.dSYM"));
                if dsym_source.exists() {
                    let dsym_dest = package_dir.join(format!("{binary_name}.dSYM"));
                    // .dSYM is a directory, so we need to copy recursively
                    std::fs::create_dir_all(&dsym_dest)?;
                    copy_dir_all(&dsym_source, &dsym_dest)?;
                    println!("Copied {binary_name}.dSYM to package directory");
                }
            }
        }
    }

    println!("=== Package creation completed ===");
    println!("Package directory: {}", package_dir.display());

    // Postcondition: All required binaries should be in package directory
    let all_binaries: Vec<&str> = workspace_binaries
        .iter()
        .flat_map(|(_, bins)| bins.iter().copied())
        .collect();

    debug_assert!(
        all_binaries.iter().all(|name| {
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

#[cfg(target_os = "macos")]
fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> Result<(), Box<dyn Error>> {
    use std::fs;
    
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        
        if ty.is_dir() {
            fs::create_dir_all(&dst_path)?;
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}
