use std::fmt;

use mcrl2_sys::cxx::CxxVector;
use mcrl2_sys::cxx::UniquePtr;
use mcrl2_sys::pbes::ffi::assignment_pair;
use mcrl2_sys::pbes::ffi::local_control_flow_graph;
use mcrl2_sys::pbes::ffi::local_control_flow_graph_vertex;
use mcrl2_sys::pbes::ffi::mcrl2_load_pbes_from_pbes_file;
use mcrl2_sys::pbes::ffi::mcrl2_load_pbes_from_text;
use mcrl2_sys::pbes::ffi::mcrl2_load_pbes_from_text_file;
use mcrl2_sys::pbes::ffi::mcrl2_local_control_flow_graph_vertex_index;
use mcrl2_sys::pbes::ffi::mcrl2_local_control_flow_graph_vertex_name;
use mcrl2_sys::pbes::ffi::mcrl2_local_control_flow_graph_vertex_outgoing_edges;
use mcrl2_sys::pbes::ffi::mcrl2_local_control_flow_graph_vertex_value;
use mcrl2_sys::pbes::ffi::mcrl2_local_control_flow_graph_vertices;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_data_specification;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_expression_replace_propositional_variables;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_expression_replace_variables;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_expression_to_string;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_to_srf_pbes;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_to_string;
use mcrl2_sys::pbes::ffi::mcrl2_propositional_variable_name;
use mcrl2_sys::pbes::ffi::mcrl2_propositional_variable_parameters;
use mcrl2_sys::pbes::ffi::mcrl2_propositional_variable_to_string;
use mcrl2_sys::pbes::ffi::mcrl2_srf_equations_summands;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_equation_variable;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_equations;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_to_pbes;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_unify_parameters;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_equation_predicate_variables;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_equation_variable;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_local_algorithm_cfgs;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_local_algorithm_equations;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_local_algorithm_run;
use mcrl2_sys::pbes::ffi::pbes;
use mcrl2_sys::pbes::ffi::predicate_variable;
use mcrl2_sys::pbes::ffi::srf_equation;
use mcrl2_sys::pbes::ffi::srf_pbes;
use mcrl2_sys::pbes::ffi::srf_summand;
use mcrl2_sys::pbes::ffi::stategraph_algorithm;
use mcrl2_sys::pbes::ffi::stategraph_equation;
use merc_utilities::MercError;

use crate::Aterm;
use crate::AtermList;
use crate::AtermString;
use crate::DataExpression;
use crate::DataSpecification;
use crate::DataVariable;

/// mcrl2::pbes_system::pbes
pub struct Pbes {
    pbes: UniquePtr<pbes>,
}

impl Pbes {
    /// Load a PBES from a file.
    pub fn from_file(filename: &str) -> Result<Self, MercError> {
        Ok(Pbes {
            pbes: mcrl2_load_pbes_from_pbes_file(filename)?,
        })
    }

    /// Load a PBES from a textual pbes file.
    pub fn from_text_file(filename: &str) -> Result<Self, MercError> {
        Ok(Pbes {
            pbes: mcrl2_load_pbes_from_text_file(filename)?,
        })
    }

    /// Load a PBES from text.
    pub fn from_text(input: &str) -> Result<Self, MercError> {
        Ok(Pbes {
            pbes: mcrl2_load_pbes_from_text(input)?,
        })
    }

    /// Returns the data specification of the PBES.
    pub fn data_specification(&self) -> DataSpecification {
        DataSpecification::new(mcrl2_pbes_data_specification(&self.pbes))
    }

    pub(crate) fn new(pbes: UniquePtr<pbes>) -> Self {
        Pbes { pbes }
    }
}

impl fmt::Display for Pbes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", mcrl2_pbes_to_string(&self.pbes))
    }
}

/// mcrl2::pbes_system::stategraph_algorithm
pub struct PbesStategraph {
    control_flow_graphs: Vec<ControlFlowGraph>,
    equations: Vec<StategraphEquation>,

    _algorithm: UniquePtr<stategraph_algorithm>,
    _equations_ffi: UniquePtr<CxxVector<stategraph_equation>>,
    _control_flow_graphs_ffi: UniquePtr<CxxVector<local_control_flow_graph>>,
}

impl PbesStategraph {
    /// Run the state graph algorithm on the given PBES.
    pub fn run(pbes: &Pbes) -> Result<Self, MercError> {
        let algorithm = mcrl2_stategraph_local_algorithm_run(&pbes.pbes)?;

        // Obtain a copy of the control flow graphs.
        let mut control_flow_graphs_ffi = CxxVector::new();
        mcrl2_stategraph_local_algorithm_cfgs(control_flow_graphs_ffi.pin_mut(), &algorithm);

        // Obtain the original equations.
        let mut equations_ffi = CxxVector::new();
        mcrl2_stategraph_local_algorithm_equations(equations_ffi.pin_mut(), &algorithm);

        Ok(PbesStategraph {
            control_flow_graphs: control_flow_graphs_ffi
                .iter()
                .map(|cfg| ControlFlowGraph::new(cfg))
                .collect(),
            equations: equations_ffi.iter().map(|eq| StategraphEquation::new(eq)).collect(),
            _algorithm: algorithm,
            _control_flow_graphs_ffi: control_flow_graphs_ffi,
            _equations_ffi: equations_ffi,
        })
    }

    /// Returns the equations computed by the algorithm.
    pub fn equations(&self) -> &Vec<StategraphEquation> {
        &self.equations
    }

    /// Returns the control flow graphs identified by the algorithm.
    pub fn control_flow_graphs(&self) -> &Vec<ControlFlowGraph> {
        &self.control_flow_graphs
    }
}

/// mcrl2::pbes_system::detail::local_control_flow_graph
pub struct ControlFlowGraph {
    _cfg: *const local_control_flow_graph,
    vertices: Vec<ControlFlowGraphVertex>,
    _vertices_ffi: UniquePtr<CxxVector<local_control_flow_graph_vertex>>,
}

impl ControlFlowGraph {
    /// Returns the vertices of the control flow graph.
    pub fn vertices(&self) -> &Vec<ControlFlowGraphVertex> {
        &self.vertices
    }

    pub(crate) fn new(cfg: *const local_control_flow_graph) -> Self {
        // Obtain the vertices of the control flow graph.
        let mut vertices_ffi = CxxVector::new();
        mcrl2_local_control_flow_graph_vertices(vertices_ffi.pin_mut(), unsafe { &*cfg });
        let vertices = vertices_ffi.iter().map(|v| ControlFlowGraphVertex::new(v)).collect();

        ControlFlowGraph {
            _cfg: cfg,
            vertices,
            _vertices_ffi: vertices_ffi,
        }
    }
}

/// mcrl2::pbes_system::detail::control_flow_graph_vertex
pub struct ControlFlowGraphVertex {
    vertex: *const local_control_flow_graph_vertex,

    outgoing_edges: Vec<(*const local_control_flow_graph_vertex, Vec<usize>)>,
    incoming_edges: Vec<(*const local_control_flow_graph_vertex, Vec<usize>)>,
}

impl ControlFlowGraphVertex {
    pub fn get(&self) -> *const local_control_flow_graph_vertex {
        self.vertex
    }

    /// Returns the name of the variable associated with this vertex.
    pub fn name(&self) -> AtermString {
        AtermString::new(Aterm::new(unsafe {
            mcrl2_local_control_flow_graph_vertex_name(self.vertex.as_ref().expect("Pointer should be valid"))
        }))
    }

    pub fn value(&self) -> DataExpression {
        DataExpression::new(Aterm::new(unsafe {
            mcrl2_local_control_flow_graph_vertex_value(self.vertex.as_ref().expect("Pointer should be valid"))
        }))
    }

    /// Returns the index of the variable associated with this vertex.
    pub fn index(&self) -> usize {
        unsafe { mcrl2_local_control_flow_graph_vertex_index(self.vertex.as_ref().expect("Pointer should be valid")) }
    }

    /// Returns the outgoing edges of the vertex.
    pub fn outgoing_edges(&self) -> &Vec<(*const local_control_flow_graph_vertex, Vec<usize>)> {
        &self.outgoing_edges
    }

    /// Returns the outgoing edges of the vertex.
    pub fn incoming_edges(&self) -> &Vec<(*const local_control_flow_graph_vertex, Vec<usize>)> {
        &self.incoming_edges
    }

    /// Construct a new vertex and retrieve its edges as well.
    /// TODO: This should probably be private.
    pub fn new(vertex: *const local_control_flow_graph_vertex) -> Self {
        let mut outgoing_edges_ffi = CxxVector::new();
        unsafe {
            mcrl2_local_control_flow_graph_vertex_outgoing_edges(
                outgoing_edges_ffi.pin_mut(),
                vertex.as_ref().expect("Pointer should be valid"),
            );
        }

        let outgoing_edges = outgoing_edges_ffi
            .iter()
            .map(|pair| (pair.vertex, pair.edges.iter().map(|i| *i as usize).collect()))
            .collect();

        ControlFlowGraphVertex {
            vertex,
            outgoing_edges,
            incoming_edges: vec![],
        }
    }
}

impl fmt::Debug for ControlFlowGraphVertex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Vertex(name: {:?}, value: {:?})", self.name(), self.value())
    }
}

/// mcrl2::pbes_system::detail::predicate_variable
pub struct PredicateVariable {
    used: Vec<usize>,
    changed: Vec<usize>,

    _variable: *const predicate_variable,
}

impl PredicateVariable {
    /// Returns the used set of the predicate variable.
    pub fn used(&self) -> &Vec<usize> {
        &self.used
    }

    /// Returns the changed set of the predicate variable.
    pub fn changed(&self) -> &Vec<usize> {
        &self.changed
    }

    /// Creates a new `PredicateVariable` from the given FFI variable pointer.
    pub(crate) fn new(variable: *const predicate_variable) -> Self {
        PredicateVariable {
            _variable: variable,
            used: unsafe {
                mcrl2_sys::pbes::ffi::mcrl2_predicate_variable_used(variable.as_ref().expect("Pointer should be valid"))
            },
            changed: unsafe {
                mcrl2_sys::pbes::ffi::mcrl2_predicate_variable_changed(
                    variable.as_ref().expect("Pointer should be valid"),
                )
            },
        }
    }
}

/// mcrl2::pbes_system::detail::stategraph_equation
pub struct StategraphEquation {
    predicate_variables: Vec<PredicateVariable>,

    equation: *const stategraph_equation,
}

impl StategraphEquation {
    /// Returns the predicate variables of the equation.
    pub fn predicate_variables(&self) -> &Vec<PredicateVariable> {
        &self.predicate_variables
    }

    /// Returns the variable of the equation.
    pub fn variable(&self) -> PropositionalVariable {
        PropositionalVariable::new(Aterm::new(unsafe {
            mcrl2_stategraph_equation_variable(self.equation.as_ref().expect("Pointer should be valid"))
        }))
    }

    pub(crate) fn new(equation: *const stategraph_equation) -> Self {
        let mut predicate_variables = CxxVector::new();
        mcrl2_stategraph_equation_predicate_variables(predicate_variables.pin_mut(), unsafe {
            equation.as_ref().expect("Pointer should be valid")
        });
        let predicate_variables = predicate_variables.iter().map(|v| PredicateVariable::new(v)).collect();

        StategraphEquation {
            predicate_variables,
            equation,
        }
    }
}

/// mcrl2::pbes_system::srf_pbes
pub struct SrfPbes {
    srf_pbes: UniquePtr<srf_pbes>,
    equations: Vec<SrfEquation>,
    _ffi_equations: UniquePtr<CxxVector<srf_equation>>,
}

impl SrfPbes {
    /// Convert a PBES to an SRF PBES.
    pub fn from(pbes: &Pbes) -> Result<Self, MercError> {
        let srf_pbes = mcrl2_pbes_to_srf_pbes(&pbes.pbes)?;

        let mut ffi_equations = CxxVector::new();
        mcrl2_srf_pbes_equations(ffi_equations.pin_mut(), &srf_pbes);

        Ok(SrfPbes {
            srf_pbes,
            equations: ffi_equations.iter().map(|eq| SrfEquation::new(eq)).collect(),
            _ffi_equations: ffi_equations,
        })
    }

    /// Convert the SRF PBES back to a PBES.
    pub fn to_pbes(&self) -> Pbes {
        Pbes::new(mcrl2_srf_pbes_to_pbes(self.srf_pbes.as_ref().unwrap()))
    }

    /// Unify all parameters of the equations.
    pub fn unify_parameters(&mut self, ignore_ce_equations: bool, reset: bool) -> Result<(), MercError> {
        mcrl2_srf_pbes_unify_parameters(self.srf_pbes.pin_mut(), ignore_ce_equations, reset);
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

    summands: Vec<SrfSummand>,
    _summands_ffi: UniquePtr<CxxVector<srf_summand>>,
}

impl SrfEquation {
    /// Returns the parameters of the equation.
    pub fn variable(&self) -> PropositionalVariable {
        PropositionalVariable::new(Aterm::new(unsafe {
            mcrl2_srf_pbes_equation_variable(self.equation.as_ref().expect("Pointer should be valid"))
        }))
    }

    /// Returns the summands of the equation.
    pub fn summands(&self) -> &Vec<SrfSummand> {
        &self.summands
    }

    /// Creates a new [`SrfEquation`] from the given FFI equation pointer.
    pub(crate) fn new(equation: *const srf_equation) -> Self {
        let mut summands_ffi = CxxVector::new();
        mcrl2_srf_equations_summands(summands_ffi.pin_mut(), unsafe {
            equation.as_ref().expect("Pointer should be valid")
        });
        let summands = summands_ffi.iter().map(|s| SrfSummand::new(s)).collect();

        SrfEquation {
            equation,
            _summands_ffi: summands_ffi,
            summands,
        }
    }
}

/// mcrl2::pbes_system::srf_summand
pub struct SrfSummand {
    summand: *const srf_summand,
}

impl SrfSummand {
    /// Returns the condition of the summand.
    pub fn condition(&self) -> PbesExpression {
        PbesExpression::new(Aterm::new(unsafe {
            mcrl2_sys::pbes::ffi::mcrl2_srf_summand_condition(self.summand.as_ref().expect("Pointer should be valid"))
        }))
    }

    /// Returns the variable of the summand.
    pub fn variable(&self) -> PbesExpression {
        PbesExpression::new(Aterm::new(unsafe {
            mcrl2_sys::pbes::ffi::mcrl2_srf_summand_variable(self.summand.as_ref().expect("Pointer should be valid"))
        }))
    }

    /// Creates a new [`SrfSummand`] from the given FFI summand pointer.
    pub(crate) fn new(summand: *const srf_summand) -> Self {
        SrfSummand { summand }
    }
}

impl fmt::Debug for SrfSummand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Summand(condition: {:?}, variable: {:?})",
            self.condition(),
            self.variable()
        )
    }
}

/// mcrl2::pbes_system::propositional_variable
pub struct PropositionalVariable {
    term: Aterm,
}

impl PropositionalVariable {
    /// Returns the name of the propositional variable.
    pub fn name(&self) -> AtermString {
        AtermString::new(Aterm::new(mcrl2_propositional_variable_name(self.term.get())))
    }

    /// Returns the parameters of the propositional variable.
    pub fn parameters(&self) -> AtermList<DataVariable> {
        let term = mcrl2_propositional_variable_parameters(self.term.get());
        AtermList::new(Aterm::new(term))
    }

    /// Creates a new `PbesPropositionalVariable` from the given term.
    pub(crate) fn new(term: Aterm) -> Self {
        PropositionalVariable { term }
    }
}

impl fmt::Debug for PropositionalVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", mcrl2_propositional_variable_to_string(self.term.get()))
    }
}

/// mcrl2::pbes_system::pbes_expression
#[derive(Clone, Eq, PartialEq)]
pub struct PbesExpression {
    term: Aterm,
}

impl PbesExpression {
    /// Creates a new [PbesExpression] from the given term.
    pub(crate) fn new(term: Aterm) -> Self {
        PbesExpression { term }
    }
}

impl fmt::Debug for PbesExpression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", mcrl2_pbes_expression_to_string(&self.term.get()))
    }
}

/// Replace variables in the given PBES expression according to the given substitution sigma.
pub fn replace_variables(expr: &PbesExpression, sigma: Vec<(DataExpression, DataExpression)>) -> PbesExpression {
    // Do not into_iter here, as we need to keep sigma alive for the call.
    let sigma: Vec<assignment_pair> = sigma
        .iter()
        .map(|(lhs, rhs)| assignment_pair {
            lhs: lhs.get().get(),
            rhs: rhs.get().get(),
        })
        .collect();

    PbesExpression::new(Aterm::new(mcrl2_pbes_expression_replace_variables(
        expr.term.get(),
        &sigma,
    )))
}

/// Replaces propositional variables in the given PBES expression according to the given substitution sigma.
pub fn replace_propositional_variables(expr: &PbesExpression, pi: &Vec<usize>) -> PbesExpression {
    PbesExpression::new(Aterm::new(mcrl2_pbes_expression_replace_propositional_variables(
        expr.term.get(),
        pi,
    )))
}
