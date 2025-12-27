use core::fmt;
use std::collections::HashMap;
use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::fs::File;
use std::fs::{self};
use std::io::BufRead;
use std::io::Write;
use std::path::Path;

use duct::cmd;
use regex::Regex;
use serde::Deserialize;
use serde::Serialize;

#[derive(Deserialize, Serialize)]
struct MeasurementEntry {
    rewriter: String,
    benchmark_name: String,
    timings: Vec<f32>,
}

#[derive(PartialEq)]
pub enum Rewriter {
    Innermost,
    Sabre,
}

impl Rewriter {
    pub fn from_str(s: &str) -> Result<Self, &'static str> {
        match s {
            "innermost" => Ok(Rewriter::Innermost),
            "sabre" => Ok(Rewriter::Sabre),
            _ => Err("Invalid rewriter"),
        }
    }
}

impl fmt::Display for Rewriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rewriter::Innermost => write!(f, "innermost"),
            Rewriter::Sabre => write!(f, "sabre"),
        }
    }
}

/// Benchmarks all the REC specifications in the example folder.
pub fn benchmark(output_path: impl AsRef<Path>, rewriter: Rewriter) -> Result<(), Box<dyn Error>> {
    // Find the tool based on which rewriter we want to benchmark
    let cwd = env::current_dir()?;

    let mcrl2_rewrite_path = if rewriter == Rewriter::Innermost || rewriter == Rewriter::Sabre {
        // Build the tool with the correct settings
        cmd!("cargo", "build", "--profile", "bench", "--bin", "merc-rewrite").run()?;

        // Using which is a bit unnecessary, but it deals nicely with .exe on Windows and can also be used to do other searching.
        which::which_in("merc-rewrite", Some("target/release/"), cwd)?
    } else {
        which::which("merc-rewrite")?
    };

    let mcrl2_rewrite_timing = match rewriter {
        Rewriter::Innermost => Regex::new(r#"Innermost rewrite took ([0-9]*) ms"#)?,
        Rewriter::Sabre => Regex::new(r#"Sabre rewrite took ([0-9]*) ms"#)?,
    };

    // Create the output directory before creating the file.
    if let Some(parent) = output_path.as_ref().parent() {
        fs::create_dir_all(parent)?;
    }

    let mut result_file = File::create(output_path)?;

    // Consider all the specifications in the example directory.
    for file in fs::read_dir("examples/REC/rec")? {
        let path = file?.path();

        let benchmark_name = path.file_stem().unwrap().to_string_lossy();
        println!("Benchmarking {benchmark_name}");

        let mut arguments = vec!["600".to_string(), mcrl2_rewrite_path.to_string_lossy().to_string()];

        match rewriter {
            Rewriter::Innermost => {
                arguments.push("rewrite".to_string());
                arguments.push("innermost".to_string());
            }
            Rewriter::Sabre => {
                arguments.push("rewrite".to_string());
                arguments.push("sabre".to_string());
            }
        }

        arguments.push(path.to_string_lossy().to_string());

        let mut measurements = MeasurementEntry {
            rewriter: rewriter.to_string(),
            benchmark_name: benchmark_name.to_string(),
            timings: Vec::new(),
        };

        // Run the benchmarks several times until one of them fails
        for _ in 0..5 {
            match cmd("timeout", &arguments).stdout_capture().stderr_capture().run() {
                Ok(result) => {
                    // Parse the standard output to read the rewriting time and insert it into results.
                    for line in result.stdout.lines().chain(result.stderr.lines()) {
                        let line = line?;

                        if let Some(result) = mcrl2_rewrite_timing.captures(&line) {
                            let (_, [grp1]) = result.extract();
                            let timing: f32 = grp1.parse()?;

                            println!("Benchmark {benchmark_name} timing {timing} milliseconds");

                            // Write the output to the file and include a newline.
                            measurements.timings.push(timing / 1000.0);
                        }
                    }
                }
                Err(err) => {
                    println!("Benchmark {benchmark_name} timed out or crashed");
                    println!("Command failed {err:?}");
                    break;
                }
            };
        }

        serde_json::to_writer(&mut result_file, &measurements)?;

        writeln!(&result_file)?;
    }

    Ok(())
}

/// Computes the average of the given values.
fn average(values: &[f32]) -> f32 {
    values.iter().sum::<f32>() / values.len() as f32
}

/// Prints a float with two decimals, since format specifiers cannot be stacked.
fn print_float(value: f32) -> String {
    format!("{value:.1}")
}

/// Prints a table with the results of the benchmark.
pub fn create_table(json_path: impl AsRef<Path>) -> Result<(), Box<dyn Error>> {
    let output = fs::read_to_string(json_path)?;

    // Keep track of all the read results.
    let mut results: HashMap<String, HashMap<String, f32>> = HashMap::new();

    // Figure out the list of rewriters used to print '-' values.
    let mut rewriters: HashSet<String> = HashSet::new();

    for line in output.lines() {
        let timing = serde_json::from_str::<MeasurementEntry>(line)?;

        rewriters.insert(timing.rewriter.clone());

        results
            .entry(timing.benchmark_name)
            .and_modify(|e| {
                e.insert(timing.rewriter.clone(), average(&timing.timings));
            })
            .or_insert_with(|| {
                let mut table = HashMap::new();
                table.insert(timing.rewriter.clone(), average(&timing.timings));
                table
            });
    }

    // Print the header of the table.
    let mut first = true;
    for rewriter in &rewriters {
        if first {
            print!("{rewriter: >30}");
            first = false;
        } else {
            print!("{rewriter: >10} |");
        }
    }

    // Print the entries in the table.
    for (benchmark, result) in &results {
        print!("{benchmark: >30}");

        for rewriter in &rewriters {
            if let Some(timing) = result.get(rewriter) {
                print!("| {: >10}", print_float(*timing));
            } else {
                print!("| {: >10}", "-");
            }
        }

        println!();
    }

    Ok(())
}
