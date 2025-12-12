use std::fmt;

use crate::Player;
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
        // Header
        writeln!(f, "digraph parity_game {{")?;

        // Global defaults and improved styling
        writeln!(f, "  rankdir=LR;")?;
        writeln!(f, "  graph [fontname=\"DejaVu Sans\", splines=true];")?;
        writeln!(f, "  node [fontname=\"DejaVu Sans\"];")?;
        writeln!(f, "  edge [fontname=\"DejaVu Sans\", color=\"#444444\", arrowsize=0.9, penwidth=1.2];")?;

        let initial = self.game.initial_vertex();

        // Emit vertices with labels and styling based on owner/priority
        for v in self.game.iter_vertices() {
            // Shape based on owner: Odd -> square, Even -> diamond. However, for the diamond
            // we use a rotated square since it has even sides.
            let orientation = match self.game.owner(v) {
                Player::Odd => "0",
                Player::Even => "45",
            };

            // Primary label: priority value only; external index via smaller-font xlabel.
            writeln!(
                f,
                "  v{} [label=\"{}\", shape=square, orientation={}, xlabel=< <FONT POINT-SIZE=\"9\">v{}</FONT> >];",
                v,
                self.game.priority(v),
                orientation,
                v
            )?;
        }

        // Emit edges
        for v in self.game.iter_vertices() {
            for to in self.game.outgoing_edges(v) {
                writeln!(f, "  v{} -> v{};", v, to)?;
            }
        }

        // Emit a small incoming arrow to the initial vertex
        writeln!(f, "  init [shape=point, width=0.05, label=\"\"];")?;
        writeln!(f, "  init -> v{} [arrowsize=0.6];", initial)?;

        // Footer
        writeln!(f, "}}")
    }
}
