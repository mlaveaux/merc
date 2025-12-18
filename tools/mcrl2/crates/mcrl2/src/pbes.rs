use std::fmt;
use std::rc::Rc;

use mcrl2_sys::cxx::CxxVector;
use mcrl2_sys::cxx::UniquePtr;
use mcrl2_sys::pbes::ffi::assignment_pair;
use mcrl2_sys::pbes::ffi::local_control_flow_graph_vertex;
use mcrl2_sys::pbes::ffi::mcrl2_load_pbes_from_pbes_file;
use mcrl2_sys::pbes::ffi::mcrl2_load_pbes_from_text;
use mcrl2_sys::pbes::ffi::mcrl2_load_pbes_from_text_file;
use mcrl2_sys::pbes::ffi::mcrl2_local_control_flow_graph_vertex;
use mcrl2_sys::pbes::ffi::mcrl2_local_control_flow_graph_vertex_index;
use mcrl2_sys::pbes::ffi::mcrl2_local_control_flow_graph_vertex_name;
use mcrl2_sys::pbes::ffi::mcrl2_local_control_flow_graph_vertex_outgoing_edges;
use mcrl2_sys::pbes::ffi::mcrl2_local_control_flow_graph_vertex_value;
use mcrl2_sys::pbes::ffi::mcrl2_local_control_flow_graph_vertices;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_data_specification;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_expression_replace_propositional_variables;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_expression_replace_variables;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_is_propositional_variable;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_to_srf_pbes;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_to_string;
use mcrl2_sys::pbes::ffi::mcrl2_srf_equations_summands;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_equation_variable;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_equations;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_to_pbes;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_unify_parameters;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_equation_predicate_variables;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_equation_variable;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_local_algorithm_cfg;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_local_algorithm_cfgs;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_local_algorithm_equation;
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

use crate::ATerm;
use crate::ATermList;
use crate::ATermString;
use crate::DataExpression;
use crate::DataSpecification;
use crate::DataVariable;
use crate::lock_global;

/// mcrl2::pbes_system::pbes
pub struct Pbes {
    pbes: UniquePtr<pbes>,
}

impl Pbes {
    /// Load a PBES from a file.
    pub fn from_file(filename: &str) -> Result<Self, MercError> {
        let _guard = lock_global();
        Ok(Pbes {
            pbes: mcrl2_load_pbes_from_pbes_file(filename)?,
        })
    }

    /// Load a PBES from a textual pbes file.
    pub fn from_text_file(filename: &str) -> Result<Self, MercError> {
        let _guard = lock_global();
        Ok(Pbes {
            pbes: mcrl2_load_pbes_from_text_file(filename)?,
        })
    }

    /// Load a PBES from text.
    pub fn from_text(input: &str) -> Result<Self, MercError> {
        let _guard = lock_global();
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

    _algorithm: Rc<UniquePtr<stategraph_algorithm>>,
}

impl PbesStategraph {
    /// Run the state graph algorithm on the given PBES.
    pub fn run(pbes: &Pbes) -> Result<Self, MercError> {
        let algorithm = Rc::new(mcrl2_stategraph_local_algorithm_run(&pbes.pbes)?);

        // Obtain a copy of the control flow graphs.
        let control_flow_graphs = (0..mcrl2_stategraph_local_algorithm_cfgs(&algorithm))
            .map(|index| ControlFlowGraph::new(algorithm.clone(), index))
            .collect::<Vec<_>>();

        let equations = (0..mcrl2_stategraph_local_algorithm_equations(&algorithm))
            .map(|index| StategraphEquation::new(algorithm.clone(), index))
            .collect::<Vec<_>>();

        Ok(PbesStategraph {
            control_flow_graphs,
            equations,
            _algorithm: algorithm,
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
    vertices: Vec<ControlFlowGraphVertex>,
}

impl ControlFlowGraph {
    /// Returns the vertices of the control flow graph.
    pub fn vertices(&self) -> &Vec<ControlFlowGraphVertex> {
        &self.vertices
    }

    /// Finds a vertex by its pointer.
    pub fn find_by_ptr(&self, ptr: *const local_control_flow_graph_vertex) -> &ControlFlowGraphVertex {
        self.vertices
            .iter()
            .find(|v| v.get() == ptr)
            .expect("Vertex should exist")
    }

    pub(crate) fn new(algorithm: Rc<UniquePtr<stategraph_algorithm>>, index: usize) -> Self {
        let cfg = mcrl2_stategraph_local_algorithm_cfg(&algorithm, index);
        let vertices = (0..mcrl2_local_control_flow_graph_vertices(cfg))
            .map(|vertex_index| ControlFlowGraphVertex::new(algorithm.clone(), index, vertex_index))
            .collect::<Vec<_>>();

        ControlFlowGraph { vertices }
    }
}

/// mcrl2::pbes_system::detail::control_flow_graph_vertex
pub struct ControlFlowGraphVertex {
    vertex: *const local_control_flow_graph_vertex,

    outgoing_edges: Vec<(*const local_control_flow_graph_vertex, Vec<usize>)>,
}

impl ControlFlowGraphVertex {
    pub fn get(&self) -> *const local_control_flow_graph_vertex {
        self.vertex
    }

    /// Returns the name of the variable associated with this vertex.
    pub fn name(&self) -> ATermString {
        ATermString::new(ATerm::from_ptr(mcrl2_local_control_flow_graph_vertex_name(
            self.as_ref(),
        )))
    }

    pub fn value(&self) -> DataExpression {
        DataExpression::new(ATerm::from_ptr(mcrl2_local_control_flow_graph_vertex_value(
            self.as_ref(),
        )))
    }

    /// Returns the index of the variable associated with this vertex.
    pub fn index(&self) -> usize {
        mcrl2_local_control_flow_graph_vertex_index(self.as_ref())
    }

    /// Returns the outgoing edges of the vertex.
    pub fn outgoing_edges(&self) -> &Vec<(*const local_control_flow_graph_vertex, Vec<usize>)> {
        &self.outgoing_edges
    }

    /// Construct a new vertex and retrieve its edges as well.
    /// TODO: This should probably be private.
    pub fn new(algorithm: Rc<UniquePtr<stategraph_algorithm>>, cfg: usize, vertex: usize) -> Self {
        let cfg = mcrl2_stategraph_local_algorithm_cfg(&algorithm, cfg);
        let vertex = mcrl2_local_control_flow_graph_vertex(cfg, vertex);
        let outgoing_edges_ffi = mcrl2_local_control_flow_graph_vertex_outgoing_edges(vertex);

        let outgoing_edges = outgoing_edges_ffi
            .iter()
            .map(|pair| (pair.vertex, pair.edges.iter().copied().collect()))
            .collect();

        ControlFlowGraphVertex { vertex, outgoing_edges }
    }

    fn as_ref(&self) -> &local_control_flow_graph_vertex {
        // Safety
        //
        // Vertex is never modified, and there is a unique owner of the underlying
        // pointer that ensures its validity.
        unsafe { self.vertex.as_ref().expect("Pointer should be valid") }
    }
}

impl fmt::Debug for ControlFlowGraphVertex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}({})", self.name(), self.value().pretty_print())
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
    index: usize,
    algorithm: Rc<UniquePtr<stategraph_algorithm>>,
    predicate_variables: Vec<PredicateVariable>,
}

impl StategraphEquation {
    /// Returns the predicate variables of the equation.
    pub fn predicate_variables(&self) -> &Vec<PredicateVariable> {
        &self.predicate_variables
    }

    /// Returns the variable of the equation.
    pub fn variable(&self) -> PropositionalVariable {
        PropositionalVariable::new(ATerm::from_ptr(mcrl2_stategraph_equation_variable(self.as_ref())))
    }

    pub(crate) fn new(algorithm: Rc<UniquePtr<stategraph_algorithm>>, index: usize) -> Self {
        let equation = mcrl2_stategraph_local_algorithm_equation(&algorithm, index);
        let predicate_variables = mcrl2_stategraph_equation_predicate_variables(equation);
        let predicate_variables = predicate_variables.iter().map(|v| PredicateVariable::new(v)).collect();

        StategraphEquation {
            predicate_variables,
            index,
            algorithm,
        }
    }

    /// Returns a reference to the underlying FFI equation.
    fn as_ref(&self) -> &stategraph_equation {
        mcrl2_stategraph_local_algorithm_equation(&self.algorithm, self.index)
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
        PropositionalVariable::new(ATerm::from_ptr(unsafe {
            mcrl2_srf_pbes_equation_variable(self.as_ref())
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

    /// Returns a reference to the underlying FFI equation.
    fn as_ref(&self) -> &srf_equation {
        unsafe { self.equation.as_ref().expect("Pointer should be valid") }
    }
}

/// mcrl2::pbes_system::srf_summand
pub struct SrfSummand {
    summand: *const srf_summand,
}

impl SrfSummand {
    /// Returns the condition of the summand.
    pub fn condition(&self) -> PbesExpression {
        PbesExpression::new(ATerm::from_ptr(unsafe {
            mcrl2_sys::pbes::ffi::mcrl2_srf_summand_condition(self.summand.as_ref().expect("Pointer should be valid"))
        }))
    }

    /// Returns the variable of the summand.
    pub fn variable(&self) -> PbesExpression {
        PbesExpression::new(ATerm::from_ptr(unsafe {
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
    term: ATerm,
}

impl PropositionalVariable {
    /// Creates a new `PbesPropositionalVariable` from the given term.
    pub fn new(term: ATerm) -> Self {
        debug_assert!(
            mcrl2_pbes_is_propositional_variable(term.get()),
            "Term {:?} is not a propositional variable",
            term
        );
        PropositionalVariable { term }
    }

    /// Returns the name of the propositional variable.
    pub fn name(&self) -> ATermString {
        ATermString::new(self.term.arg(0).protect())
    }

    /// Returns the parameters of the propositional variable.
    pub fn parameters(&self) -> ATermList<DataVariable> {
        ATermList::new(self.term.arg(1).protect())
    }
}

impl fmt::Debug for PropositionalVariable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.term)
    }
}

/// mcrl2::pbes_system::pbes_expression
#[derive(Clone, Eq, PartialEq)]
pub struct PbesExpression {
    term: ATerm,
}

impl PbesExpression {
    /// Creates a new pbes expression from the given term.
    fn new(term: ATerm) -> Self {
        PbesExpression { term }
    }
}

impl fmt::Debug for PbesExpression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.term)
    }
}

/// Replace variables in the given PBES expression according to the given substitution sigma.
pub fn replace_variables(expr: &PbesExpression, sigma: Vec<(DataExpression, DataExpression)>) -> PbesExpression {
    // Do not into_iter here, as we need to keep sigma alive for the call.
    let sigma: Vec<assignment_pair> = sigma
        .iter()
        .map(|(lhs, rhs)| assignment_pair {
            lhs: lhs.address(),
            rhs: rhs.address(),
        })
        .collect();

    PbesExpression::new(ATerm::from_unique_ptr(mcrl2_pbes_expression_replace_variables(
        expr.term.get(),
        &sigma,
    )))
}

/// Replaces propositional variables in the given PBES expression according to the given substitution sigma.
pub fn replace_propositional_variables(expr: &PbesExpression, pi: &Vec<usize>) -> PbesExpression {
    PbesExpression::new(ATerm::from_unique_ptr(
        mcrl2_pbes_expression_replace_propositional_variables(expr.term.get(), pi),
    ))
}
