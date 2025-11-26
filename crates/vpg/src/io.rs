use std::io::Read;

use log::{info, trace};
use regex::Regex;
use streaming_iterator::StreamingIterator;
use thiserror::Error;

use merc_io::{LineIterator, Progress};
use merc_utilities::MercError;

use crate::{ParityGame, Player, Priority, VertexIndex};

#[derive(Error, Debug)]
pub enum IOError {
    #[error("Invalid .pg header {0}")]
    InvalidHeader(&'static str),

    #[error("Invalid line {0}")]
    InvalidLine(&'static str),
}

/// Reads a parity game from the given reader.
///
/// # Details
///
/// The format starts with a header, followed by the vertices
///
/// parity <num_of_vertices>;
/// <index> <priority> <owner> <outgoing_vertex>, <outgoing_vertex>, ...;
pub fn read_pg(reader: impl Read) -> Result<ParityGame, MercError> {
    let mut lines = LineIterator::new(reader);
    lines.advance();
    let header = lines
        .get()
        .ok_or(IOError::InvalidHeader("The first line should be the header"))?;

    // Read the header
    let header_regex = Regex::new(r#"parity\s+([0-9]+)\s*;"#).expect("Regex compilation should not fail");

    let (_, [num_of_vertices_txt]) = header_regex
        .captures(header)
        .ok_or(IOError::InvalidHeader("does not match parity <num_of_vertices>;"))?
        .extract();

    let num_of_vertices: usize = num_of_vertices_txt.parse()?;
    let mut progress = Progress::new(
        |value, increment| info!("Reading vertices {}%...", value / increment),
        num_of_vertices,
    );

    // Collect that data into the parity game structure
    let mut owner: Vec<Player> = vec![Player::Even; num_of_vertices];
    let mut priority: Vec<Priority> = vec![Priority::new(0); num_of_vertices];

    let mut vertices: Vec<usize> = Vec::with_capacity(num_of_vertices + 1);
    let mut transitions_to: Vec<VertexIndex> = Vec::with_capacity(num_of_vertices);

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
        vertices.push(transitions_to.len());

        if let Some(succesors) = parts.next() 
        {
            // Parse successors (remaining parts, removing trailing semicolon)
            for successor in 
                succesors
                .trim_end_matches(';')
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().parse())
            {
                let successor = successor?;
                transitions_to.push(VertexIndex::new(successor));
            }
        }


        progress.add(1);
    }

    Ok(ParityGame::new(
        VertexIndex::new(0),
        owner,
        priority,
        vertices,
        transitions_to,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_pg() {
        let _parity_game = read_pg(include_bytes!("../../../examples/vpg/example.pg") as &[u8]).unwrap();
    }
}
