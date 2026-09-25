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

/// Sets each of `variables` as an environment variable on `builder`, reading
/// its value from `compilation_toml`'s `[env]` table.
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
pub(crate) struct RuntimeLibrary {
    source_dir: PathBuf,
    temp_dir: PathBuf,
}

impl RuntimeLibrary {
    /// Creates a new library that can be compiled at runtime.
    ///
    /// `dependencies` are extra lines appended verbatim to the generated
    /// `[dependencies]` table of the on-the-fly crate.
    pub(crate) fn new(temp_dir: &Path, dependencies: Vec<String>) -> Result<RuntimeLibrary, MercError> {
        info!("Creating library in directory {}", temp_dir.to_string_lossy());
        let source_dir = PathBuf::from(temp_dir).join("src");

        // Create the directory structure for a Cargo project. `create_dir_all`
        // tolerates missing parents and an already-existing directory.
        fs::create_dir_all(&source_dir)?;

        // Write the cargo configuration
        {
            let mut file = File::create(PathBuf::from(temp_dir).join("Cargo.toml"))?;
            // `rust-version` is templated from the host crate (workspace-inherited)
            // so the generated crate never drifts from the toolchain in use.
            writeln!(
                &mut file,
                indoc! {"
                [package]
                name = \"sabre-generated\"
                edition = \"2024\"
                rust-version = \"{}\"
                version = \"1.0.0\"
                [workspace]

                [dependencies]"},
                env!("CARGO_PKG_RUST_VERSION")
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
            writeln!(&mut file, "*")?;
        }

        Ok(RuntimeLibrary {
            temp_dir: PathBuf::from(temp_dir),
            source_dir,
        })
    }

    /// Returns the directory in which the source files can be placed.
    pub(crate) fn source_dir(&self) -> &PathBuf {
        &self.source_dir
    }

    /// Compiles the generated crate into a dynamic library and loads it back in.
    pub(crate) fn compile(&mut self) -> Result<Library, MercError> {
        let compilation_toml = include_str!(concat!(env!("OUT_DIR"), "/Compilation.toml")).parse::<Table>()?;

        // Compile the dynamic object.
        info!("Compiling...");
        let release = !cfg!(debug_assertions);
        let build_args: &[&str] = if release {
            &["build", "--lib", "--release"]
        } else {
            &["build", "--lib"]
        };
        let mut expr = cmd("cargo", build_args).dir(self.temp_dir.as_path());
        expr = apply_env(expr, &compilation_toml, &["RUSTFLAGS", "CFLAGS", "CXXFLAGS"])?;
        expr.run()?;

        info!("finished.");

        // Figure out the path to the library (it is based on platform: linux, windows and then macos)
        let profile = if release { "release" } else { "debug" };
        let mut path = self
            .temp_dir
            .clone()
            .join(format!("./target/{profile}/libsabre_generated.so"));
        if !path.exists() {
            path = self
                .temp_dir
                .clone()
                .join(format!("./target/{profile}/sabre_generated.dll"));
            if !path.exists() {
                path = self
                    .temp_dir
                    .clone()
                    .join(format!("./target/{profile}/libsabre_generated.dylib"));
                if !path.exists() {
                    return Err("Could not find the compiled library!".into());
                }
            }
        }

        // Load it back in and call the rewriter.
        //
        // Safety: `libloading::Library::new` is unsafe because loading an arbitrary
        // dynamic library can run arbitrary initialisation code and offers no
        // guarantee that any symbol it exports has the signature the caller expects.
        // Here `path` names a library this same process just built, moments ago, from
        // source this crate generated, from the on-the-fly `Cargo.toml` written by
        // `RuntimeLibrary::new` (which pins the generated crate's `rust-version` to
        // `CARGO_PKG_RUST_VERSION`, the host's own toolchain version). There is no
        // library-side initialisation to run at load time beyond static linking; the
        // `initialise`/`rewrite` symbols are only *called* later, in
        // `SabreCompilingRewriter::new`, whose own `# Safety` note is what actually
        // justifies their signatures matching.
        unsafe { Ok(Library::new(&path)?) }
    }
}
