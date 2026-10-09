//! Authors: Maurice Laveaux and Sjef van Loo

use std::io::BufWriter;
use std::io::Read;
use std::io::Write;

use itertools::Itertools;
use log::info;
use regex::Regex;
use streaming_iterator::StreamingIterator;
use thiserror::Error;

use merc_io::LineIterator;
use merc_io::TimeProgress;
use merc_utilities::MercError;

use crate::PG;
use crate::ParityGame;
use crate::ParityGameBuilder;
use crate::Player;
use crate::Priority;
use crate::Set;
use crate::Strategy;
use crate::VertexIndex;

#[derive(Error, Debug)]
pub enum IOError {
    #[error("Invalid .pg header {0}")]
    InvalidHeader(&'static str),

    #[error("Invalid line {0}")]
    InvalidLine(&'static str),
}

/// Reads a parity game in textual PGSolver `.pg` format from the given reader.
///
/// # Details
///
/// The format starts with a header, followed by the vertices
///
/// `parity <num_of_vertices>;`
/// `<index> <priority> <owner> <outgoing_vertex>, <outgoing_vertex>, ...;`
pub fn read_pg<R: Read>(reader: R) -> Result<ParityGame, MercError> {
    info!("Reading parity game in .pg format...");

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
    let progress = TimeProgress::new(
        |(amount, total): (usize, usize)| info!("Read {} vertices ({}%)...", amount, amount * 100 / total),
        1,
    );

    // Collect the data in a builder and remove duplicate edges at the end.
    let mut builder = ParityGameBuilder::with_capacity(VertexIndex::new(0), num_of_vertices);
    if num_of_vertices > 0 {
        builder.add_vertex(VertexIndex::new(num_of_vertices - 1), Player::Even, Priority::new(0));
    }

    let mut vertex_count = 0;
    while let Some(line) = lines.next() {
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

        let vertex = VertexIndex::new(index);
        let owner =
            Player::try_from_index(vertex_owner).ok_or(IOError::InvalidLine("owner must be 0 (even) or 1 (odd)"))?;
        builder.add_vertex(vertex, owner, Priority::new(vertex_priority));

        for successors in parts {
            // An optional vertex name in quotes may follow the successors, which we ignore.
            if successors.starts_with('"') {
                break;
            }

            // Parse successors (remaining parts, removing trailing semicolon)
            for successor in successors
                .trim_end_matches(';')
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().parse())
            {
                let successor = successor?;
                builder.add_edge(vertex, VertexIndex::new(successor));
            }
        }

        progress.print((vertex_count + 1, num_of_vertices));
        vertex_count += 1;
    }

    Ok(builder.finish(true, true))
}

/// Writes the given parity game to the given writer in .pg format.
pub fn write_pg<W: Write, G: PG>(writer: W, game: &G) -> Result<(), MercError> {
    info!("Writing parity game to .pg format...");
    let mut writer = BufWriter::new(writer);

    let progress = TimeProgress::new(
        |(index, total): (usize, usize)| info!("Wrote {} vertices ({}%)...", index, index * 100 / total),
        1,
    );

    writeln!(writer, "parity {};", game.num_of_vertices())?;
    for v in game.iter_vertices() {
        let prio = game.priority(v);
        let owner = game.owner(v).to_index();

        write!(writer, "{} {} {} ", v.value(), prio.value(), owner)?;
        write!(
            writer,
            "{}",
            game.outgoing_edges(v).map(|edge| edge.to().value()).format(", ")
        )?;
        writeln!(writer, ";")?;
        progress.print((v.value() + 1, game.num_of_vertices()));
    }

    Ok(())
}

/// Writes a solution in the PGSolver solution format, as produced by
/// `pgsolver --printsolonly`: a header `paritysol <highest index>;` followed by
/// `<vertex> <winner> [<successor>];` for every vertex. The successor is the
/// winner's strategy and is only written for vertices owned by their winner
/// when a strategy is given.
pub fn write_pg_solution<W: Write, G: PG>(
    writer: W,
    game: &G,
    solution: &[Set; 2],
    strategy: Option<&[Strategy; 2]>,
) -> Result<(), MercError> {
    let mut writer = BufWriter::new(writer);

    writeln!(writer, "paritysol {};", game.num_of_vertices().saturating_sub(1))?;
    for v in game.iter_vertices() {
        let winner = if solution[Player::Even.to_index()][*v] {
            Player::Even
        } else {
            Player::Odd
        };
        write!(writer, "{} {}", v.value(), winner.to_index())?;
        if game.owner(v) == winner
            && let Some(successor) = strategy.and_then(|strategy| strategy[winner.to_index()].get(v))
        {
            write!(writer, " {}", successor.value())?;
        }
        writeln!(writer, ";")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use merc_utilities::random_test;

    use crate::Player;
    use crate::VertexIndex;
    use crate::random_parity_game;
    use crate::solve_zielonka;

    use super::Itertools;
    use super::PG;
    use super::read_pg;
    use super::write_pg;
    use super::write_pg_solution;

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_read_pg() {
        let parity_game = read_pg(include_bytes!("../../../../examples/vpg/example.pg") as &[u8]).unwrap();
        assert_eq!(parity_game.num_of_vertices(), 3002);
        assert_eq!(parity_game.num_of_edges(), 3968);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_io_pg() {
        random_test(100, |rng| {
            let game = random_parity_game(rng, true, 50, 10, 5);

            let mut buffer = Vec::new();
            write_pg(&mut buffer, &game).unwrap();

            let read_game = read_pg(buffer.as_slice()).unwrap();
            assert_eq!(game.num_of_vertices(), read_game.num_of_vertices());
            assert_eq!(game.num_of_edges(), read_game.num_of_edges());

            for vertex in game.iter_vertices() {
                let original_successors: Vec<_> = game.outgoing_edges(vertex).map(|e| e.to()).collect();
                let read_successors: Vec<_> = read_game.outgoing_edges(vertex).map(|e| e.to()).collect();
                assert_eq!(original_successors.len(), read_successors.len());
                assert_eq!(
                    original_successors.into_iter().sorted().collect::<Vec<_>>(),
                    read_successors.into_iter().sorted().collect::<Vec<_>>()
                );
            }
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_write_pg_solution() {
        random_test(50, |rng| {
            let game = random_parity_game(rng, true, 30, 6, 3);
            let (solution, strategy) = solve_zielonka(&game, true);

            let mut buffer = Vec::new();
            write_pg_solution(&mut buffer, &game, &solution, strategy.as_ref()).unwrap();
            let text = String::from_utf8(buffer).unwrap();

            let mut lines = text.lines();
            assert_eq!(
                lines.next(),
                Some(format!("paritysol {};", game.num_of_vertices() - 1).as_str())
            );
            for (line, v) in lines.zip(game.iter_vertices()) {
                let fields: Vec<usize> = line
                    .trim_end_matches(';')
                    .split_whitespace()
                    .map(|field| field.parse().unwrap())
                    .collect();
                assert_eq!(fields[0], v.value());
                let winner = Player::from_index(fields[1] as u8);
                assert!(
                    solution[winner.to_index()][*v],
                    "winner of {v} does not match the solution"
                );
                if let Some(&successor) = fields.get(2) {
                    assert_eq!(game.owner(v), winner);
                    assert!(
                        game.outgoing_edges(v)
                            .any(|edge| edge.to() == VertexIndex::new(successor))
                    );
                }
            }
        });
    }
}
