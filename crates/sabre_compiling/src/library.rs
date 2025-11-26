use std::fs::File;
use std::fs::{self};
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use duct::Expression;
use duct::cmd;
use indoc::indoc;
use libloading::Library;
use log::info;
use toml::Table;
use toml::Value;
use toml::map::Map;

use merc_utilities::MercError;

/// Apply the value from compilation_toml for every given variable as an environment variable.
fn apply_env(
    builder: Expression,
    compilation_toml: &Map<String, Value>,
    variables: &[&'_ str],
) -> Result<Expression, MercError> {
    let mut result = builder;
    let env = compilation_toml.get("env").ok_or("Missing [env] table")?;

    for var in variables {
        let value = env.get(*var).ok_or("Missing var")?.as_str().ok_or("Not a string")?;

        info!("Setting environment variable {var} = {value}");
        result = result.env(var, value);
    }

    Ok(result)
}

/// A library that can be used to generate a crate on-the-fly and load it back
/// in after compiling at runtime.
pub struct RuntimeLibrary {
    source_dir: PathBuf,
    temp_dir: PathBuf,
}

impl RuntimeLibrary {
    /// Creates a new library that can be compiled at runtime.
    /// - depe
    pub fn new(temp_dir: &Path, dependencies: Vec<String>) -> Result<RuntimeLibrary, MercError> {
        info!("Creating library in directory {}", temp_dir.to_string_lossy());
        let source_dir = PathBuf::from(temp_dir).join("src");

        // Create the directory structure for a Cargo project
        if !temp_dir.exists() {
            fs::create_dir(temp_dir)?;
        }

        if !source_dir.exists() {
            fs::create_dir(&source_dir)?;
        }

        // Write the cargo configuration
        {
            let mut file = File::create(PathBuf::from(temp_dir).join("Cargo.toml"))?;
            writeln!(
                &mut file,
                indoc! {"
                [package]
                name = \"sabre-generated\"
                edition = \"2024\"
                rust-version = \"1.85.0\"
                version = \"1.0.0\"
                [workspace]
                
                [dependencies]
            "}
            )?;

            for dependency in &dependencies {
                writeln!(&mut file, "{dependency}")?;
            }

            writeln!(
                &mut file,
                indoc! {"
                
                [lib]
                crate-type = [\"cdylib\", \"rlib\"]            
            "}
            )?;
        }

        // Ignore the created package.
        {
            let mut file = File::create(PathBuf::from(temp_dir).join(".gitignore"))?;
            writeln!(&mut file, "*.*")?;
        }

        Ok(RuntimeLibrary {
            temp_dir: PathBuf::from(temp_dir),
            source_dir,
        })
    }

    /// Returns the directory in which the source files can be placed.
    pub fn source_dir(&self) -> &PathBuf {
        &self.source_dir
    }

    /// Compiles the library into
    pub fn compile(&mut self) -> Result<Library, MercError> {
        let compilation_toml = include_str!("../../../target/Compilation.toml").parse::<Table>()?;

        // Compile the dynamic object.
        info!("Compiling...");
        let mut expr = cmd("cargo", &["build", "--lib"]).dir(self.temp_dir.as_path());
        expr = apply_env(expr, &compilation_toml, &["RUSTFLAGS", "CFLAGS", "CXXFLAGS"])?;
        expr.run()?;

        info!("finished.");

        // Figure out the path to the library (it is based on platform: linux, windows and then macos)
        let mut path = self.temp_dir.clone().join("./target/debug/libsabre_generated.so");
        if !path.exists() {
            path = self.temp_dir.clone().join("./target/debug/sabre_generated.dll");
            if !path.exists() {
                path = self.temp_dir.clone().join("./target/debug/libsabre_generated.dylib");
                if !path.exists() {
                    return Err("Could not find the compiled library!".into());
                }
            }
        }

        // Load it back in and call the rewriter.
        unsafe { Ok(Library::new(&path)?) }
    }
}
