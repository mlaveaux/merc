use std::env;
use std::path::Path;
use std::path::PathBuf;

fn main() {

    #[cfg(feature = "merc_bcg_format")]
    if let Ok(directory) = env::var("CADP") {
        let bcg_user = Path::new(&directory).join("incl").join("bcg_user.h");
        if bcg_user.exists() {
            // The bindgen::Builder is the main entry point
            // to bindgen, and lets you build up options for
            // the resulting bindings.
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
        } else {
            panic!(
                "CADP environment variable is set, but the file {} does not exist.",
                bcg_user.display()
            );
        }
    } else {
        println!("cargo:error=The 'merc_bcg_format' feature is enabled, but the CADP environment variable is not set.");
    }
}
