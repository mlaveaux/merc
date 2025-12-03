use std::fmt;

use mcrl2_sys::cxx::CxxVector;
use mcrl2_sys::cxx::UniquePtr;
use mcrl2_sys::pbes::ffi::local_control_flow_graph;
use mcrl2_sys::pbes::ffi::local_control_flow_graph_vertex;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_local_control_flow_graph_vertices;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_stategraph_local_algorithm_cfgs;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_stategraph_local_algorithm_run;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_to_string;
use mcrl2_sys::pbes::ffi::mcrl2_propositional_variable_parameters;
use mcrl2_sys::pbes::ffi::mcrl2_propositional_variable_to_string;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_equation_variable;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_equations;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_to_pbes;
use mcrl2_sys::pbes::ffi::mcrl2_unify_parameters;
use mcrl2_sys::pbes::ffi::pbes;
use mcrl2_sys::pbes::ffi::propositional_variable;
use mcrl2_sys::pbes::ffi::srf_equation;
use mcrl2_sys::pbes::ffi::srf_pbes;
use mcrl2_sys::pbes::ffi::stategraph_algorithm;
use merc_utilities::MercError;

use crate::ATerm;
use crate::AtermList;

/// mcrl2::pbes_system::pbes
pub struct Pbes {
    pbes: UniquePtr<pbes>,
}

impl Pbes {
    /// Load a PBES from a file.
    pub fn from_file(filename: &str) -> Result<Self, MercError> {
        Ok(Pbes {
            pbes: mcrl2_sys::pbes::ffi::mcrl2_load_pbes_from_file(filename)?,
        })
    }

    pub(crate) fn new(pbes: UniquePtr<pbes>) -> Self {
        Pbes { pbes }
    }
}

impl fmt::Display for Pbes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", mcrl2_pbes_to_string(&self.pbes).unwrap())
    }
}

pub struct PbesStategraph {
    algorithm: UniquePtr<stategraph_algorithm>,
    control_flow_graphs: Vec<PbesStategraphControlFlowGraph>,
}

impl PbesStategraph {
    /// Run the state graph algorithm on the given PBES.
    pub fn run(pbes: &Pbes) -> Self {
        let algorithm = mcrl2_pbes_stategraph_local_algorithm_run(&pbes.pbes).unwrap();

        // Obtain a copy of the control flow graphs.
        let mut cfgs = CxxVector::new();
        mcrl2_pbes_stategraph_local_algorithm_cfgs(cfgs.pin_mut(), &algorithm).unwrap();

        PbesStategraph {
            algorithm,
            control_flow_graphs: cfgs
                .iter()
                .map(|cfg| PbesStategraphControlFlowGraph::new(cfg))
                .collect(),
        }
    }

    /// Returns the control flow graphs identified by the algorithm.
    pub fn control_flow_graphs(&self) -> &Vec<PbesStategraphControlFlowGraph> {
        &self.control_flow_graphs
    }
}

/// Represents a local control flow graph identified by the PBES state graph algorithm.
/// mcrl2::pbes_system::detail::local_control_flow_graph
pub struct PbesStategraphControlFlowGraph {
    cfg: *const local_control_flow_graph,
    vertices: Vec<ControlFlowGraphVertex>,
}

impl PbesStategraphControlFlowGraph {

    /// Returns the vertices of the control flow graph.
    pub fn vertices(&self) -> &Vec<ControlFlowGraphVertex> {
        &self.vertices
    }

    pub(crate) fn new(cfg: *const local_control_flow_graph) -> Self {

        // Obtain the vertices of the control flow graph.
        let mut ffi_vertices = CxxVector::new();
        mcrl2_pbes_local_control_flow_graph_vertices(ffi_vertices.pin_mut(), unsafe { &*cfg }).unwrap();
        let vertices = ffi_vertices
            .iter()
            .map(|v| ControlFlowGraphVertex { vertex: v })
            .collect();

        PbesStategraphControlFlowGraph { cfg, vertices }
    }
}

/// mcrl2::pbes_system::detail::control_flow_graph_vertex
pub struct ControlFlowGraphVertex {
    vertex: *const local_control_flow_graph_vertex,
}

/// mcrl2::pbes_system::srf_pbes
pub struct SrfPbes {
    srf_pbes: UniquePtr<srf_pbes>,
    equations: Vec<SrfEquation>,
}

impl SrfPbes {
    /// Convert a PBES to an SRF PBES.
    pub fn from(pbes: &Pbes) -> Result<Self, MercError> {
        let srf_pbes = mcrl2_sys::pbes::ffi::mcrl2_pbes_to_srf_pbes(&pbes.pbes)?;

        let mut ffi_equations = CxxVector::new();
        mcrl2_srf_pbes_equations(ffi_equations.pin_mut(), &srf_pbes).unwrap();

        Ok(SrfPbes {
            srf_pbes,
            equations: ffi_equations.iter().map(|eq| SrfEquation::new(eq)).collect(),
        })
    }

    /// Convert the SRF PBES back to a PBES.
    pub fn to_pbes(&self) -> Pbes {
        Pbes::new(mcrl2_srf_pbes_to_pbes(self.srf_pbes.as_ref().unwrap()).unwrap())
    }

    /// Unify all parameters of the equations.
    pub fn unify_parameters(&mut self, ignore_ce_equations: bool, reset: bool) -> Result<(), MercError> {
        mcrl2_unify_parameters(self.srf_pbes.pin_mut(), ignore_ce_equations, reset)?;
        Ok(())
    }

    /// Returns the srf equations of the SRF pbes.
    pub fn equations(&self) -> &Vec<SrfEquation> {
        &self.equations
    }
}

/// mcrl2::pbes_system::srf_equation
pub struct SrfEquation {
    equation: *const srf_equation,
}

impl SrfEquation {
    /// Returns the parameters of the equation.
    pub fn variable(&self) -> PropositionalVariable {
        PropositionalVariable::new(unsafe { mcrl2_srf_pbes_equation_variable(self.equation).unwrap() })
    }

    /// Creates a new `SrfEquation` from the given FFI equation pointer.
    pub(crate) fn new(equation: *const srf_equation) -> Self {
        SrfEquation { equation }
    }
}

/// mcrl2::pbes_system::propositional_variable
pub struct PropositionalVariable {
    variable: UniquePtr<propositional_variable>,
}

impl PropositionalVariable {
    /// Returns the parameters of the propositional variable.
    pub fn parameters(&self) -> AtermList<ATerm> {
        let term = mcrl2_propositional_variable_parameters(&self.variable).unwrap();
        AtermList::new(ATerm::new(term))
    }

    /// Creates a new `PbesPropositionalVariable` from the given term.
    pub(crate) fn new(variable: UniquePtr<propositional_variable>) -> Self {
        PropositionalVariable { variable }
    }
}

impl fmt::Debug for PropositionalVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", mcrl2_propositional_variable_to_string(&self.variable).unwrap())
    }
}
