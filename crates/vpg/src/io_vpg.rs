//! Authors: Maurice Laveaux and Sjef van Loo

use std::fmt;
use std::io::BufWriter;
use std::io::Read;
use std::io::Write;

use itertools::Itertools;
use log::info;
use log::trace;
use oxidd::BooleanFunction;
use oxidd::Manager;
use oxidd::ManagerRef;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;
use oxidd::util::OptBool;
use regex::Regex;
use streaming_iterator::StreamingIterator;

use merc_io::LineIterator;
use merc_io::TimeProgress;
use merc_utilities::MercError;

use crate::IOError;
use crate::ParityGame;
use crate::Player;
use crate::Priority;
use crate::VariabilityParityGame;
use crate::VertexIndex;

/// Reads a variability parity game from the given reader.
/// Note that the reader is buffered internally using a `BufReader`.
///
/// # Details
///
/// The format starts with a header, followed by the vertices
///
/// parity <num_of_vertices>;
/// `\<index\> \<priority\> \<owner\> \<outgoing_vertex\>,\<outgoing_vertex\>,...;`
/// Each outgoing edge is represented as `\<\to>|\<configuration_set\>`. For the
/// format of the configuration set see [parse_configuration_set]
pub fn read_vpg(manager: &BDDManagerRef, reader: impl Read) -> Result<VariabilityParityGame, MercError> {
    let mut lines = LineIterator::new(reader);
    lines.advance();
    let header = lines
        .get()
        .ok_or(IOError::InvalidHeader("The first line should be the confs header"))?;

    // Read the confs <configurations> line
    let confs_regex = Regex::new(r#"confs\s+([+-01]*)\s*;"#).expect("Regex compilation should not fail");
    let (_, [configurations_txt]) = confs_regex
        .captures(header)
        .ok_or(IOError::InvalidHeader("header does not match confs <configurations>;"))?
        .extract();
    let (variables, configurations) = parse_configuration(manager, configurations_txt)?;

    // Read the parity header
    let header_regex = Regex::new(r#"parity\s+([0-9]+)\s*;"#).expect("Regex compilation should not fail");
    let header = lines
        .next()
        .ok_or(IOError::InvalidHeader("The second line should be the parity header"))?;

    let (_, [num_of_vertices_txt]) = header_regex
        .captures(header)
        .ok_or(IOError::InvalidHeader(
            "header does not match parity <num_of_vertices>;",
        ))?
        .extract();

    let num_of_vertices: usize = num_of_vertices_txt.parse()?;

    // Collect that data into the parity game structure
    let mut owner: Vec<Player> = vec![Player::Even; num_of_vertices];
    let mut priority: Vec<Priority> = vec![Priority::new(0); num_of_vertices];

    let mut vertices: Vec<usize> = Vec::with_capacity(num_of_vertices + 1);
    let mut edges_to: Vec<VertexIndex> = Vec::with_capacity(num_of_vertices);
    let mut edges_configuration: Vec<BDDFunction> = Vec::with_capacity(num_of_vertices);

    // Print progress messages
    let progress = TimeProgress::new(|percentage: usize| info!("Reading vertices {}%...", percentage), 1);
    let mut vertex_count = 0;
    while let Some(line) = lines.next() {
        trace!("{line}");

        // Parse the line: <index> <priority> <owner> <outgoing_vertex>, <outgoing_vertex>, ...;
        let mut parts = line.split_whitespace();

        let index: usize = parts
            .next()
            .ok_or(IOError::InvalidLine("Expected at least <index> ...;"))?
            .parse()?;
        let vertex_priority: usize = parts
            .next()
            .ok_or(IOError::InvalidLine("Expected at least <index> <priority> ...;"))?
            .parse()?;
        let vertex_owner = Player::from_index(
            parts
                .next()
                .ok_or(IOError::InvalidLine(
                    "Expected at least <index> <priority> <owner> ...;",
                ))?
                .parse()?,
        );

        owner[index] = vertex_owner;
        priority[index] = Priority::new(vertex_priority);

        // Store the offset for the vertex
        vertices.push(edges_configuration.len());

        if let Some(succesors) = parts.next() {
            // Parse successors (remaining parts, removing trailing semicolon)
            for successor in succesors
                .trim_end_matches(';')
                .split(',')
                .filter(|s| !s.trim().is_empty())
            {
                let parts: Vec<&str> = successor.trim().split('|').collect();
                let successor_index: usize = parts[0].trim().parse()?;
                edges_to.push(VertexIndex::new(successor_index));

                if parts.len() > 1 {
                    let config = parse_configuration_set(manager, &variables, parts[1].trim())?;
                    edges_configuration.push(config);
                } else {
                    // No configuration specified, use true (all configurations)
                    edges_configuration.push(manager.with_manager_shared(|m| BDDFunction::t(m)));
                }
            }
        }

        progress.print(vertex_count / num_of_vertices);
        vertex_count += 1;
    }

    // Add the sentinel state.
    vertices.push(edges_configuration.len());

    Ok(VariabilityParityGame::new(
        ParityGame::new(VertexIndex::new(0), owner, priority, vertices, edges_to),
        configurations,
        edges_configuration,
    ))
}

/// Parses a configuration set from a string representation into a BDD function, but also creates the necessary variables.
/// based on the length of the configurations.
fn parse_configuration(manager: &BDDManagerRef, config: &str) -> Result<(Vec<BDDFunction>, BDDFunction), MercError> {
    if let Some(first_part) = config.split('+').next() {
        let variables = manager.with_manager_exclusive(|manager| {
            manager
                .add_vars(first_part.len() as u32)
                .map(|i| BDDFunction::var(manager, i))
                .collect::<Result<Vec<_>, _>>()
        })?;

        let configuration = parse_configuration_set(manager, &variables, config)?;
        return Ok((variables, configuration));
    };

    Err(MercError::from(IOError::InvalidHeader("Empty configuration string")))
}

/// Parses a configuration from a string representation into a BDD function.
///
/// # Details
///
/// A configuration is represented as a string <entry>+<entry>+..., where each entry is either
/// a sequence consisting of '-', '0', and '1', representing don't care, false, and true respectively.
/// The length of the sequence determines the number of boolean variables. So `-1--` represents a boolean
/// function over 4 variables.
///
/// The variables must be defined beforehand and are assumed to be in order, i.e., the first character
/// corresponds to variable 0, the second to variable 1, and so on.
fn parse_configuration_set(
    manager_ref: &BDDManagerRef,
    variables: &[BDDFunction],
    config: &str,
) -> Result<BDDFunction, MercError> {
    manager_ref.with_manager_shared(|manager| -> Result<BDDFunction, MercError> {
        let mut result = BDDFunction::f(manager);

        for part in config.split('+') {
            let mut conjunction = BDDFunction::t(manager);

            for (i, c) in part.chars().enumerate() {
                let var = &variables[i];
                match c {
                    '1' => conjunction = conjunction.and(var)?,
                    '0' => conjunction = conjunction.and(&var.not()?)?,
                    '-' => {} // don't care
                    _ => {
                        return Err(MercError::from(IOError::InvalidHeader(
                            "Invalid character in configuration",
                        )));
                    }
                }
            }

            result = result.or(&conjunction)?;
        }

        Ok(result)
    })
}

/// Writes the given parity game to the given writer in .vpg format.
/// Note that the reader is buffered internally using a `BufWriter`.
pub fn write_vpg(writer: &mut impl Write, game: &VariabilityParityGame) -> Result<(), MercError> {
    let mut writer = BufWriter::new(writer);

    writeln!(writer, "confs {};", FormatConfigSet(game.configuration()))?;
    writeln!(writer, "parity {};", game.num_of_vertices())?;

    for v in game.iter_vertices() {
        let prio = game.priority(v);
        let owner = game.owner(v).to_index();

        write!(writer, "{} {} {} ", v.value(), prio.value(), owner)?;
        write!(
            writer,
            "{}",
            game.outgoing_edges(v).format_with(",", |edge, fmt| {
                fmt(&format_args!("{}|{}", edge.to(), FormatConfigSet(edge.configuration())))
            })
        )?;

        writeln!(writer, ";")?;
    }

    Ok(())
}

/// A helper structure to format configuration sets for output.
struct FormatConfigSet<'a>(&'a BDDFunction);

impl fmt::Display for FormatConfigSet<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut choices: Vec<OptBool> = Vec::new();
        let mut last_index = 0;
        let mut first = true;
        let mut stop_condition = false;

        // Use pick_cube to iterate over all cubes in the BDD
        while !stop_condition
            && let Some(cube) = self.0.pick_cube(|_manager, _edge, index| {
                // Ensure that the choices vector is large enough, initialize with don't care
                let mut resized = false;
                if index as usize >= choices.len() {
                    resized = true;
                    choices.resize(index as usize + 1, OptBool::None);
                }

                // If we have skipped levels then the intermediate variables should be don't care
                for i in (last_index as usize + 1)..(index as usize) {
                    choices[i] = OptBool::None;
                }

                if index <= last_index {
                    // Set all ones to zero, and initialize the next index on true
                    let mut had_false = false;
                    for i in 0..choices.len() {
                        if choices[i] == OptBool::True {
                            choices[i] = OptBool::False;
                        } else if choices[i] == OptBool::False {
                            choices[i] = OptBool::True;
                            had_false = true;
                            break; // Skip updating further indices
                        }
                    }

                    if !had_false && !resized {
                        // All choices with 1 have been taken, so abort.
                        stop_condition = true;
                    }
                }

                // Update the choice for the current index
                last_index = index;

                if choices[index as usize] == OptBool::None {
                    // First time setting this index, it should be false
                    choices[index as usize] = OptBool::False;
                }

                match choices[index as usize] {
                    OptBool::False => true,
                    OptBool::True => false,
                    OptBool::None => unreachable!("Proper choice should have been set"),
                }
            })
        {
            if !first {
                write!(f, "+")?;
            }

            if !stop_condition {}

            for value in cube {
                match value {
                    OptBool::True => write!(f, "1")?,
                    OptBool::False => write!(f, "0")?,
                    OptBool::None => write!(f, "-")?,
                }
            }
            first = false;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not support miri (specifically the crossbeam-epoch dependency)
    fn test_read_vpg() {
        let manager = oxidd::bdd::new_manager(2048, 1024, 8);

        let parity_game = read_vpg(&manager, include_bytes!("../../../examples/vpg/example.vpg") as &[u8]).unwrap();

        assert_eq!(parity_game.num_of_vertices(), 61014);
    }
}
