use log::info;
use mcrl2::Pbes;
use mcrl2::PbesSrf;
use mcrl2::PbesStategraph;
use mcrl2::PbesStategraphControlFlowGraph;
use merc_utilities::MercError;

/// Implements symmetry detection for PBESs.
pub struct SymmetryAlgorithm {
    state_graph: PbesStategraph, // Needs to be kept alive while the control flow graphs are used.
    control_flow_graphs: Vec<PbesStategraphControlFlowGraph>,
}

impl SymmetryAlgorithm {
    /// Does the required preprocessing to analyse symmetries in the given PBES.
    pub fn new(pbes: &Pbes) -> Result<Self, MercError> {
        // Apply various preproecessing necessary for symmetry detection
        let mut srf = PbesSrf::from(pbes)?;
        srf.unify_parameters(false, false)?;

        let parameters = if let Some(equation) = srf.equations().first() {
            equation.variable().parameters().to_vec()
        } else {
            // There are no equations, so no parameters.
            Vec::new()
        };

        let state_graph = PbesStategraph::run(&Pbes::from(srf))?;
        unimplemented!();
        // let control_flow_graphs = state_graph.control_flow_graphs();

        // Self {
        //     state_graph,
        //     control_flow_graphs,
        // }
    }

    /// Runs the symmetry detection algorithm.
    pub fn run(&self) {
        let cliques = self.cliques();

        for clique in cliques {
            info!("Found clique: {:?}", clique);
        }
    }

    /// Determine the cliques in the given control flow graphs.
    fn cliques(&self) ->Vec<Vec<usize>> {
        let mut cal_I = Vec::new();

        for (i, cfg) in self.control_flow_graphs.iter().enumerate() {
            if cal_I.iter().any(|clique: &Vec<usize>| clique.contains(&i)) {
                // Skip every graph that already belongs to a clique.
                continue;
            }

            // For every other control flow graph check if it is compatible, and start a new clique
            let mut clique = vec![i];
            for j in (i + 1)..self.control_flow_graphs.len() {
                if self.compatible(cfg, &self.control_flow_graphs[j]) {
                    clique.push(j);
                }
            }

            if clique.len() > 1 {
                cal_I.push(clique);
            }
        }

        cal_I
    }

    /// Returns true iff the two control flow graphs are compatible.
    fn compatible(&self, left: &PbesStategraphControlFlowGraph, right: &PbesStategraphControlFlowGraph) -> bool {
        unimplemented!()
    }
}
