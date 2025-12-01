//! Authors: Maurice Laveaux and Sjef van Loo

use std::io::Read;
use std::io::Write;

use log::info;
use log::trace;
use oxidd::BooleanFunction;
use oxidd::Manager;
use oxidd::ManagerRef;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;
use regex::Regex;
use streaming_iterator::StreamingIterator;

use merc_io::LineIterator;
use merc_io::TimeProgress;
use merc_utilities::MercError;

use crate::IOError;
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
/// <index> <priority> <owner> <outgoing_vertex>, <outgoing_vertex>, ...;
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
    let (variables, configurations) = parse_configuration(manager, &configurations_txt)?;

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
    let mut progress = TimeProgress::new(|percentage: usize| info!("Reading vertices {}%...", percentage), 1);
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
        let vertex_owner: u8 = parts
            .next()
            .ok_or(IOError::InvalidLine(
                "Expected at least <index> <priority> <owner> ...;",
            ))?
            .parse()?;

        owner[index] = Player::from_index(vertex_owner as u8);
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
                    edges_configuration.push(manager.with_manager_shared(|m| BDDFunction::t(&m)));
                }
            }
        }

        progress.print(vertex_count / num_of_vertices);
        vertex_count += 1;
    }

    // Add the sentinel state.
    vertices.push(edges_configuration.len());

    Ok(VariabilityParityGame::new(
        VertexIndex::new(0),
        configurations,
        owner,
        priority,
        vertices,
        edges_configuration,
        edges_to,
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
    manager: &BDDManagerRef,
    variables: &Vec<BDDFunction>,
    config: &str,
) -> Result<BDDFunction, MercError> {
    manager.with_manager_shared(|manager| {
        let mut result = BDDFunction::f(&manager);

        for part in config.split('+') {
            let mut conjunction = BDDFunction::t(&manager);

            for (i, c) in part.chars().enumerate() {
                let var = &variables[i];
                match c {
                    '1' => conjunction = conjunction.and(&var)?,
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

/// Writes the given parity game to the given writer in .pg format.
pub fn write_vpg(
    _manager: &BDDManagerRef,
    _writer: &mut impl Write,
    _game: &VariabilityParityGame,
) -> Result<(), MercError> {
    // How to iterate over the satisfying assignments and write them as a string.
    unimplemented!();
}

/// Write a configuration set to its string representation.
fn write_configuration_set(
    _manager: &BDDManagerRef,
    _variables: &Vec<BDDFunction>,
    _config: &BDDFunction,
) -> Result<String, MercError> {
    // How to iterate over the satisfying assignments and write them as a string.
    unimplemented!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_pg() {
        let manager = oxidd::bdd::new_manager(2048, 1024, 8);

        let parity_game = read_vpg(&manager, include_bytes!("../../../examples/vpg/example.vpg") as &[u8]).unwrap();

        assert_eq!(parity_game.num_of_vertices(), 61014);
    }
}
