use std::env;
use std::error::Error;
use std::fs;
use std::fs::File;

use std::io::Write;

/// Write every environment variable in the variables array.
fn write_env(writer: &mut impl Write, variables: &[&'static str]) -> Result<(), Box<dyn Error>> {
    for var in variables {
        writeln!(writer, "{} = '{}'", var, env::var(var).unwrap_or_default())?;
    }

    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    for (from, to) in env::vars() {
        println!("{from} to {to}");
    }

    let mut file = File::create("../../target/Compilation.toml")?;

    // Write the development location.
    writeln!(file, "[sabrec]")?;
    writeln!(file, "path = '{}'", fs::canonicalize(".")?.to_string_lossy())?;

    // Write compilation related environment variables to the configuration file.
    writeln!(file, "[env]")?;
    write_env(&mut file, &["RUSTFLAGS", "CFLAGS", "CXXFLAGS"])?;

    Ok(())
}
