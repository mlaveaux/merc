use std::fmt;

use oxidd::bdd::BDDFunction;
use oxidd::util::OptBool;

use crate::CubeIter;
use crate::Player;
use crate::VariabilityParityGame;
use crate::parity_game::PG;

/// Helper to render a parity game in Graphviz DOT format.
pub struct PgDot<'a, G: PG> {
    pub game: &'a G,
}

impl<'a, G: PG> PgDot<'a, G> {
    /// Creates a new PgDot Display for the given parity game.
    pub fn new(game: &'a G) -> Self {
        Self { game }
    }
}

impl<'a, G: PG> fmt::Display for PgDot<'a, G> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_dot_header(f)?;
        write_dot_style(f)?;

        let initial = self.game.initial_vertex();

        write_vertices(f, self.game)?;

        // Display edges
        for v in self.game.iter_vertices() {
            for to in self.game.outgoing_edges(v) {
                writeln!(f, "  v{} -> v{};", v, to)?;
            }
        }

        write_init_arrow(f, initial)?;

        write_footer(f)
    }
}

/// Helper to render a parity game in Graphviz DOT format.
pub struct VpgDot<'a> {
    pub game: &'a VariabilityParityGame,
}

impl<'a> VpgDot<'a> {
    /// Creates a new PgDot Display for the given parity game.
    pub fn new(game: &'a VariabilityParityGame) -> Self {
        Self { game }
    }
}

impl<'a> fmt::Display for VpgDot<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_dot_header(f)?;
        write_dot_style(f)?;

        let initial = self.game.initial_vertex();

        write_vertices(f, self.game)?;

        // Display edges
        for v in self.game.iter_vertices() {
            for edge in self.game.outgoing_conf_edges(v) {
                writeln!(
                    f,
                    "  v{} -> v{} [label=\"{}\"];",
                    v,
                    edge.to(),
                    DisplayConfig(edge.configuration())
                )?;
            }
        }

        write_init_arrow(f, initial)?;

        write_footer(f)
    }
}

// Shared free functions for DOT rendering
fn write_dot_header(f: &mut fmt::Formatter<'_>) -> fmt::Result {
    writeln!(f, "digraph parity_game {{")
}

fn write_dot_style(f: &mut fmt::Formatter<'_>) -> fmt::Result {
    writeln!(f, "  rankdir=LR;")?;
    writeln!(f, "  graph [fontname=\"DejaVu Sans\", splines=true];")?;
    writeln!(f, "  node [fontname=\"DejaVu Sans\"];")?;
    writeln!(
        f,
        "  edge [fontname=\"DejaVu Sans\", color=\"#444444\", arrowsize=0.9, penwidth=1.2];"
    )
}

fn write_init_arrow(f: &mut fmt::Formatter<'_>, initial: impl fmt::Display) -> fmt::Result {
    writeln!(f, "  init [shape=point, width=0.05, label=\"\"];")?;
    writeln!(f, "  init -> v{} [arrowsize=0.6];", initial)
}

fn write_footer(f: &mut fmt::Formatter<'_>) -> fmt::Result { writeln!(f, "}}") }

fn write_vertices<G: PG>(f: &mut fmt::Formatter<'_>, game: &G) -> fmt::Result {
    for v in game.iter_vertices() {
        let orientation = match game.owner(v) {
            Player::Odd => "0",
            Player::Even => "45",
        };

        writeln!(
            f,
            "  v{} [label=\"{}\", shape=square, orientation={}, xlabel=< <FONT POINT-SIZE=\"9\">v{}</FONT> >];",
            v,
            game.priority(v),
            orientation,
            v
        )?;
    }
    Ok(())
}

// TODO: With information from the feature diagram we could actually put the variable names.
struct DisplayConfig<'a>(&'a BDDFunction);

impl fmt::Display for DisplayConfig<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for cube in CubeIter::new(self.0) {
            for bit in cube {
                match bit {
                    OptBool::True => {
                        write!(f, "1")?;
                    }
                    OptBool::False => {
                        write!(f, "0")?;
                    }
                    OptBool::None => {
                        write!(f, "-")?;
                    }
                };
            }
        }

        Ok(())
    }
}
