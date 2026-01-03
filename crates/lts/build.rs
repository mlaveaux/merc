#[cfg(feature = "cadp")]
use std::env;
#[cfg(feature = "cadp")]
use std::path::Path;
#[cfg(feature = "cadp")]
use std::path::PathBuf;

fn main() {
    #[cfg(feature = "cadp")]
    if let Ok(directory) = env::var("CADP") {
        let bcg_user = Path::new(&directory).join("incl").join("bcg_user.h");
        if bcg_user.exists() {
            // The bindgen::Builder is the main entry point
            // to bindgen, and lets you build up options for
            // the resulting bindings.

            use std::process::Command;
            let bindings = bindgen::Builder::default()
                // The input header we would like to generate
                // bindings for.
                .header(bcg_user.to_string_lossy())
                // Tell cargo to invalidate the built crate whenever any of the
                // included header files changed.
                .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
                // Finish the builder and generate the bindings.
                .generate()
                // Unwrap the Result and panic on failure.
                .expect("Unable to generate bindings");

            // Write the bindings to the $OUT_DIR/bindings.rs file.
            let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
            bindings
                .write_to_file(out_path.join("bindings.rs"))
                .expect("Couldn't write bindings!");

            // Link to the BCG library.
            let arch = Command::new("sh")
                .arg("-c")
                .arg(Path::new(&directory).join("com").join("arch"))
                .output()
                .expect("Failed to get system architecture");

            if !arch.status.success() {
                panic!("Cannot determine system architecture for linking BCG libraries.");
            }

            let bcg_libraries =
                Path::new(&directory).join(format!("bin.{}", String::from_utf8_lossy(&arch.stdout).trim()));
            cargo_emit::rustc_link_search!(bcg_libraries.to_string_lossy());

            cargo_emit::rustc_link_lib!("BCG");
            cargo_emit::rustc_link_lib!("BCG_IO");
        } else {
            panic!(
                "CADP environment variable is set, but the file {} does not exist.",
                bcg_user.display()
            );
        }
    } else {
        panic!("The 'cadp' feature is enabled, but the CADP environment variable is not set.");
    }
}
