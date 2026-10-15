use std::cell::RefCell;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use mcrl2_sys::cxx::CxxVector;
use mcrl2_sys::cxx::UniquePtr;
use mcrl2_sys::data::ffi::mcrl2_pbes_expression_replace_variables;
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
use mcrl2_sys::pbes::ffi::mcrl2_make_data_assignment_list;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_clone;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_create_rewrite_context;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_data_specification;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_equation_formula;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_equation_is_mu;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_equation_variable;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_equations;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_expression_replace_propositional_variables;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_initial_state;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_instantiate_global_variables;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_is_propositional_variable;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_one_point_rule;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_order_quantified_variables;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_rewrite_formula;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_rewrite_set_assignments;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_simplify_quantifiers;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_to_srf_pbes;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_to_string;
use mcrl2_sys::pbes::ffi::mcrl2_pbes_unify_parameters;
use mcrl2_sys::pbes::ffi::mcrl2_srf_equation_is_conjunctive;
use mcrl2_sys::pbes::ffi::mcrl2_srf_equation_is_mu;
use mcrl2_sys::pbes::ffi::mcrl2_srf_equations_summands;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_data_specification;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_equation_variable;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_equations;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_resolve_summand_variable_name_clashes;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_to_pbes;
use mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_unify_parameters;
use mcrl2_sys::pbes::ffi::mcrl2_srf_summand_parameters;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_equation_predicate_variables;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_equation_variable;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_local_algorithm_cfg;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_local_algorithm_cfgs;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_local_algorithm_equation;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_local_algorithm_equations;
use mcrl2_sys::pbes::ffi::mcrl2_stategraph_local_algorithm_run;
use mcrl2_sys::pbes::ffi::pbes;
use mcrl2_sys::pbes::ffi::pbes_equation;
use mcrl2_sys::pbes::ffi::predicate_variable;
use mcrl2_sys::pbes::ffi::srf_equation;
use mcrl2_sys::pbes::ffi::srf_pbes;
use mcrl2_sys::pbes::ffi::srf_summand;
use mcrl2_sys::pbes::ffi::stategraph_algorithm;
use mcrl2_sys::pbes::ffi::stategraph_equation;
use merc_utilities::MercError;
use merc_utilities::Timing;

use crate::ATerm;
use crate::ATermList;
use crate::ATermString;
use crate::DataExpression;
use crate::DataSpecification;
use crate::DataVariable;
use crate::PbesExpression;
use crate::PbesPropositionalVariableInstantiation;
use crate::data_expression::to_assignment_pairs;
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

    /// Normalizes the PBES.
    pub fn normalize(&mut self) {
        mcrl2_sys::pbes::ffi::mcrl2_pbes_normalize(self.pbes.pin_mut());
    }

    /// Checks whether the PBES is well-typed.
    pub fn is_well_typed(&self) -> bool {
        mcrl2_sys::pbes::ffi::mcrl2_pbes_is_well_typed(&self.pbes)
    }

    /// Creates a new PBES from the given FFI PBES pointer.
    pub(crate) fn new(pbes: UniquePtr<pbes>) -> Self {
        Pbes { pbes }
    }

    /// Returns the initial state (propositional variable instantiation) of the PBES.
    pub fn initial_state(&self) -> PbesPropositionalVariableInstantiation {
        // SAFETY: the FFI returns the live initial-state term of this PBES.
        PbesPropositionalVariableInstantiation::new(unsafe {
            ATerm::from_ptr(mcrl2_pbes_initial_state(
                self.pbes.as_ref().expect("pbes UniquePtr should not be null"),
            ))
        })
    }

    /// Unifies the parameter vectors of all equations in-place.
    ///
    /// After this call every equation shares the same parameter vector (same names and sorts).
    /// Unlike `SrfPbes::unify_parameters`, this operates directly on the PBES without
    /// converting to standard recursive form, so formula structure is preserved.
    pub fn unify_parameters(&mut self, ignore_ce_equations: bool, reset: bool) -> Result<(), MercError> {
        mcrl2_pbes_unify_parameters(self.pbes.pin_mut(), ignore_ce_equations, reset)?;
        Ok(())
    }

    /// Substitutes a value for every global variable of the PBES, in-place.
    ///
    /// Returns an error when a global variable cannot be instantiated.
    pub fn instantiate_global_variables(&mut self) -> Result<(), MercError> {
        mcrl2_pbes_instantiate_global_variables(self.pbes.pin_mut())?;
        Ok(())
    }

    /// Simplifies every equation body in-place, evaluating data subterms and
    /// eliminating quantifiers that range over nothing.
    pub fn simplify_quantifiers(&mut self) -> Result<(), MercError> {
        mcrl2_pbes_simplify_quantifiers(self.pbes.pin_mut())?;
        Ok(())
    }

    /// Applies the one point rule to every equation body in-place, replacing a
    /// quantifier that pins its variable to a single value by that instance.
    pub fn one_point_rule(&mut self) -> Result<(), MercError> {
        mcrl2_pbes_one_point_rule(self.pbes.pin_mut())?;
        Ok(())
    }

    /// Orders the quantified variables of every equation body in-place, so that
    /// quantifiers differing only in the order of their variables become the
    /// same term.
    pub fn order_quantified_variables(&mut self) -> Result<(), MercError> {
        mcrl2_pbes_order_quantified_variables(self.pbes.pin_mut())?;
        Ok(())
    }

    /// Applies the preprocessing before instantiating a PBES, in-place,
    /// reporting every step it runs.
    ///
    /// Only the equation bodies change, so the parameter vector that
    /// [`Pbes::unify_parameters`] produces is unaffected and the symmetry
    /// generators keep indexing into the same vector.
    ///
    /// Every step is registered on `timing`, so a tool that prints its timings
    /// reports them alongside the rest of its phases.
    ///
    /// Returns an error when a global variable cannot be instantiated.
    pub fn preprocess(&mut self, timing: &Timing) -> Result<(), MercError> {
        // Named steps rather than four inlined calls so that the reporting
        // cannot drift out of sync with what is actually run.
        let steps: [(&str, fn(&mut Pbes) -> Result<(), MercError>); 4] = [
            ("instantiate global variables", Pbes::instantiate_global_variables),
            ("simplify quantifiers", Pbes::simplify_quantifiers),
            ("one point rule", Pbes::one_point_rule),
            ("order quantified variables", Pbes::order_quantified_variables),
        ];

        for (name, step) in steps {
            timing.measure(&format!("preprocess: {name}"), || step(self))?;
        }

        Ok(())
    }

    /// Applies the preprocessing for symbolic instantiation, in-place,
    /// reporting every step it runs.
    ///
    /// Every step is registered on `timing`, so a tool that prints its timings
    /// reports them alongside the rest of its phases.
    ///
    /// Returns an error when a global variable cannot be instantiated.
    pub fn preprocess_symbolic(&mut self, timing: &Timing) -> Result<(), MercError> {
        let steps: [(&str, fn(&mut Pbes) -> Result<(), MercError>); 3] = [
            ("instantiate global variables", Pbes::instantiate_global_variables),
            ("normalize", |p| {
                p.normalize();
                Ok(())
            }),
            ("one point rule", Pbes::one_point_rule),
        ];

        for (name, step) in steps {
            timing.measure(&format!("preprocess: {name}"), || step(self))?;
        }

        Ok(())
    }

    /// Returns the equations of the PBES, in declaration order.
    ///
    /// Unlike [`SrfPbes::equations`], this returns the equations exactly as
    /// they appear in the PBES: no conversion to standard recursive form, and
    /// equations are not required to share a single unified parameter vector.
    pub fn equations(&self) -> PbesEquations {
        let mut ffi_equations = CxxVector::new();
        mcrl2_pbes_equations(
            ffi_equations.pin_mut(),
            self.pbes.as_ref().expect("pbes UniquePtr should not be null"),
        );

        let equations = ffi_equations.iter().map(|eq| PbesEquation::new(eq)).collect();
        PbesEquations {
            equations,
            _ffi_equations: ffi_equations,
        }
    }
}

/// Build a `data::assignment_list` from two parallel ATerm lists: a `variable_list` and a
/// `data_expression_list`. The returned term can be passed to `LearnSuccessorsContext` as the
/// `assignments` argument when enumerating PBES SRF summands.
pub fn make_data_assignment_list(variables: &ATerm, values: &ATerm) -> ATerm {
    ATerm::from_unique_ptr(mcrl2_make_data_assignment_list(variables.get(), values.get()))
}

impl fmt::Display for Pbes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", mcrl2_pbes_to_string(&self.pbes))
    }
}

impl Clone for Pbes {
    fn clone(&self) -> Self {
        let _guard = lock_global();
        Pbes {
            pbes: mcrl2_pbes_clone(&self.pbes),
        }
    }
}

/// Wraps an `enumerate_quantifiers_rewriter` together with a substitution σ.
///
/// Not `Send`: the underlying C++ rewriter is single-threaded. Clone the PBES
/// and construct a separate context per thread.
pub struct PbesRewriteContext {
    ctx: RefCell<UniquePtr<mcrl2_sys::pbes::ffi::pbes_rewrite_context>>,
    _not_send: PhantomData<*const ()>,
}

impl PbesRewriteContext {
    pub fn from_data_spec(data_spec: &DataSpecification) -> Result<Self, MercError> {
        let ctx = mcrl2_pbes_create_rewrite_context(
            data_spec
                .get()
                .as_ref()
                .expect("data_specification UniquePtr should not be null"),
        )?;
        Ok(PbesRewriteContext {
            ctx: RefCell::new(ctx),
            _not_send: PhantomData,
        })
    }

    /// Sets σ := { `variables[i]` ↦ `values[i]` } for the next rewrite call.
    ///
    /// # Safety
    /// Every pointer in `variables` must be a live `data::variable` term, and
    /// every pointer in `values` must be a live `data::data_expression` term.
    pub unsafe fn set_assignments(
        &self,
        variables: &[*const mcrl2_sys::atermpp::ffi::_aterm],
        values: &[*const mcrl2_sys::atermpp::ffi::_aterm],
    ) {
        assert_eq!(
            variables.len(),
            values.len(),
            "Variables and values must have equal length"
        );

        // The only error the FFI reports is the length mismatch asserted above.
        mcrl2_pbes_rewrite_set_assignments(self.ctx.borrow_mut().pin_mut(), variables, values)
            .expect("variables and values have equal length");
    }

    /// Rewrites `formula` under the current σ.
    ///
    /// The result is a protected `PbesExpression` and remains valid
    /// independently of subsequent calls.
    ///
    /// Returns an error when the rewriter cannot evaluate the formula, which
    /// happens for quantifiers that it fails to enumerate (for instance over a
    /// function sort, or over an infinite sort).
    ///
    /// # Safety
    /// `formula` must be a live `pbes_expression` term.
    pub unsafe fn rewrite_formula(&self, formula: &PbesExpression) -> Result<PbesExpression, MercError> {
        let ptr = unsafe { mcrl2_pbes_rewrite_formula(self.ctx.borrow_mut().pin_mut(), formula.get()) }?;
        Ok(PbesExpression::new(unsafe { ATerm::from_ptr(ptr) }))
    }
}

/// The equations of a [`Pbes`], in declaration order.
///
/// Owns the underlying FFI equation vector, so the [`PbesEquation`]s (each a
/// pointer into it) stay valid for as long as this value is alive.
pub struct PbesEquations {
    equations: Vec<PbesEquation>,
    _ffi_equations: UniquePtr<CxxVector<pbes_equation>>,
}

impl std::ops::Deref for PbesEquations {
    type Target = [PbesEquation];

    fn deref(&self) -> &[PbesEquation] {
        &self.equations
    }
}

/// mcrl2::pbes_system::pbes_equation
pub struct PbesEquation {
    equation: *const pbes_equation,
}

impl PbesEquation {
    /// Creates a new [`PbesEquation`] from the given FFI equation pointer.
    pub(crate) fn new(equation: *const pbes_equation) -> Self {
        PbesEquation { equation }
    }

    /// Returns a reference to the underlying FFI equation.
    fn as_ref(&self) -> &pbes_equation {
        unsafe { self.equation.as_ref().expect("Pointer should be valid") }
    }

    /// Returns true when the equation has a least fixed-point (μ) symbol, false for greatest (ν).
    pub fn is_mu(&self) -> bool {
        mcrl2_pbes_equation_is_mu(self.as_ref())
    }

    /// Returns the bound predicate variable (name and parameters) of the equation.
    pub fn variable(&self) -> PropositionalVariable {
        // SAFETY: `self.equation` is a live equation pointer (kept alive by the
        // owning `PbesEquations`), and the FFI returns the live variable term
        // wrapped immediately by `from_ptr`.
        PropositionalVariable::new(unsafe { ATerm::from_ptr(mcrl2_pbes_equation_variable(self.as_ref())) })
    }

    /// Returns the right-hand side predicate formula of the equation.
    pub fn formula(&self) -> PbesExpression {
        // SAFETY: `self.equation` is a live equation pointer (kept alive by the
        // owning `PbesEquations`), and the FFI returns the live formula term
        // wrapped immediately by `from_ptr`.
        PbesExpression::new(unsafe { ATerm::from_ptr(mcrl2_pbes_equation_formula(self.as_ref())) })
    }
}

impl fmt::Debug for PbesEquation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {:?} = {}",
            if self.is_mu() { "mu" } else { "nu" },
            self.variable(),
            self.formula()
        )
    }
}

/// mcrl2::pbes_system::stategraph_algorithm
pub struct PbesStategraph {
    control_flow_graphs: Vec<Mcrl2ControlFlowGraph>,
    equations: Vec<StategraphEquation>,

    _algorithm: Rc<UniquePtr<stategraph_algorithm>>,
}

impl PbesStategraph {
    /// Run the state graph algorithm on the given PBES.
    pub fn run(pbes: &Pbes) -> Result<Self, MercError> {
        let algorithm = Rc::new(mcrl2_stategraph_local_algorithm_run(&pbes.pbes)?);

        // Obtain a copy of the control flow graphs.
        let control_flow_graphs = (0..mcrl2_stategraph_local_algorithm_cfgs(&algorithm))
            .map(|index| Mcrl2ControlFlowGraph::new(algorithm.clone(), index))
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
    pub fn equations(&self) -> &[StategraphEquation] {
        &self.equations
    }

    /// Returns the control flow graphs identified by the algorithm.
    pub fn control_flow_graphs(&self) -> &[Mcrl2ControlFlowGraph] {
        &self.control_flow_graphs
    }
}

/// mcrl2::pbes_system::detail::local_control_flow_graph
pub struct Mcrl2ControlFlowGraph {
    vertices: Vec<Mcrl2ControlFlowGraphVertex>,
}

impl Mcrl2ControlFlowGraph {
    /// Returns the vertices of the control flow graph.
    pub fn vertices(&self) -> &[Mcrl2ControlFlowGraphVertex] {
        &self.vertices
    }

    /// Finds a vertex by its pointer.
    pub fn find_by_ptr(&self, ptr: *const local_control_flow_graph_vertex) -> &Mcrl2ControlFlowGraphVertex {
        self.vertices
            .iter()
            .find(|v| v.get() == ptr)
            .expect("Vertex should exist")
    }

    pub(crate) fn new(algorithm: Rc<UniquePtr<stategraph_algorithm>>, index: usize) -> Self {
        // Only ever called with an index from `0..mcrl2_stategraph_local_algorithm_cfgs`.
        let cfg = mcrl2_stategraph_local_algorithm_cfg(&algorithm, index).expect("cfg index is in range");
        let vertices = (0..mcrl2_local_control_flow_graph_vertices(cfg))
            .map(|vertex_index| Mcrl2ControlFlowGraphVertex::new(algorithm.clone(), index, vertex_index))
            .collect::<Vec<_>>();

        Mcrl2ControlFlowGraph { vertices }
    }
}

/// mcrl2::pbes_system::detail::control_flow_graph_vertex
pub struct Mcrl2ControlFlowGraphVertex {
    vertex: *const local_control_flow_graph_vertex,

    outgoing_edges: Vec<(*const local_control_flow_graph_vertex, Vec<usize>)>,

    /// Retains ownership of the algorithm that owns the underlying vertex, and
    /// outgoing edges.
    _algorithm: Rc<UniquePtr<stategraph_algorithm>>,
}

impl Mcrl2ControlFlowGraphVertex {
    pub fn get(&self) -> *const local_control_flow_graph_vertex {
        self.vertex
    }

    /// Returns the name of the variable associated with this vertex.
    pub fn name(&self) -> ATermString {
        // SAFETY: the FFI returns the live name term of this vertex.
        ATermString::new(unsafe { ATerm::from_ptr(mcrl2_local_control_flow_graph_vertex_name(self.as_ref())) })
    }

    pub fn value(&self) -> DataExpression {
        // SAFETY: the FFI returns the live value term of this vertex.
        DataExpression::new(unsafe { ATerm::from_ptr(mcrl2_local_control_flow_graph_vertex_value(self.as_ref())) })
    }

    /// Returns the index of the variable associated with this vertex.
    pub fn index(&self) -> usize {
        mcrl2_local_control_flow_graph_vertex_index(self.as_ref())
    }

    /// Returns the outgoing edges of the vertex.
    pub fn outgoing_edges(&self) -> &[(*const local_control_flow_graph_vertex, Vec<usize>)] {
        &self.outgoing_edges
    }

    /// Construct a new vertex and retrieve its edges as well.
    /// TODO: This should probably be private.
    pub(crate) fn new(algorithm: Rc<UniquePtr<stategraph_algorithm>>, cfg: usize, vertex: usize) -> Self {
        let cfg = mcrl2_stategraph_local_algorithm_cfg(&algorithm, cfg).expect("cfg index is in range");
        // Only ever called with an index from `0..mcrl2_local_control_flow_graph_vertices`.
        let vertex = mcrl2_local_control_flow_graph_vertex(cfg, vertex).expect("vertex index is in range");
        let outgoing_edges_ffi = mcrl2_local_control_flow_graph_vertex_outgoing_edges(vertex);

        let outgoing_edges = outgoing_edges_ffi
            .iter()
            .map(|pair| (pair.vertex, pair.edges.iter().copied().collect()))
            .collect();

        Mcrl2ControlFlowGraphVertex {
            vertex,
            outgoing_edges,
            _algorithm: algorithm,
        }
    }

    fn as_ref(&self) -> &local_control_flow_graph_vertex {
        // SAFETY: the vertex is never modified and the owning `stategraph_algorithm`
        // (kept alive through the retained `Rc`) guarantees the pointer stays valid.
        unsafe { self.vertex.as_ref().expect("Pointer should be valid") }
    }
}

impl fmt::Debug for Mcrl2ControlFlowGraphVertex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}({})", self.name(), self.value().pretty_print())
    }
}

/// mcrl2::pbes_system::detail::predicate_variable
pub struct PredicateVariable {
    /// The set of variables used in this summand
    used: Vec<usize>,
    changed: Vec<usize>,

    // These fields currently store only the parameter
    // indices (domain) of the source, target, and copy functions, not the
    // full function mappings.
    source: Vec<usize>,
    target: Vec<usize>,
    copy: Vec<usize>,

    pvi: PbesPropositionalVariableInstantiation,

    /// The underlying FFI variable pointer.
    _variable: *const predicate_variable,
}

impl PredicateVariable {
    /// Returns the variable associated with this predicate variable.
    pub fn variable(&self) -> &PbesPropositionalVariableInstantiation {
        &self.pvi
    }

    /// Returns the used set of the predicate variable.
    pub fn used(&self) -> &[usize] {
        &self.used
    }

    /// Returns the changed set of the predicate variable.
    pub fn changed(&self) -> &[usize] {
        &self.changed
    }

    /// Returns the source function of the predicate variable.
    pub fn source(&self) -> &[usize] {
        &self.source
    }

    /// Returns the target function of the predicate variable.
    pub fn target(&self) -> &[usize] {
        &self.target
    }

    /// Returns the copy function of the predicate variable.
    pub fn copy(&self) -> &[usize] {
        &self.copy
    }

    /// Creates a new `PredicateVariable` from the given FFI variable pointer.
    pub(crate) fn new(variable: *const predicate_variable) -> Self {
        // SAFETY: The pointer comes from the stategraph algorithm which keeps the
        // predicate variables alive for the duration of the algorithm.
        let var = unsafe { variable.as_ref() }.expect("Pointer should be valid");

        PredicateVariable {
            _variable: variable,
            // SAFETY: the FFI returns the live PVI term of this predicate variable.
            pvi: PbesPropositionalVariableInstantiation::new(unsafe {
                ATerm::from_ptr(
                    mcrl2_sys::pbes::ffi::mcrl2_predicate_variable_propositional_variable_instantiation(var),
                )
            }),
            used: mcrl2_sys::pbes::ffi::mcrl2_predicate_variable_used(var),
            changed: mcrl2_sys::pbes::ffi::mcrl2_predicate_variable_changed(var),
            source: mcrl2_sys::pbes::ffi::mcrl2_predicate_variable_source(var),
            target: mcrl2_sys::pbes::ffi::mcrl2_predicate_variable_target(var),
            copy: mcrl2_sys::pbes::ffi::mcrl2_predicate_variable_copy(var),
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
    pub fn predicate_variables(&self) -> &[PredicateVariable] {
        &self.predicate_variables
    }

    /// Returns the variable of the equation.
    pub fn variable(&self) -> PropositionalVariable {
        // SAFETY: the FFI returns the live variable term of this equation.
        PropositionalVariable::new(unsafe { ATerm::from_ptr(mcrl2_stategraph_equation_variable(self.as_ref())) })
    }

    pub(crate) fn new(algorithm: Rc<UniquePtr<stategraph_algorithm>>, index: usize) -> Self {
        // Only ever called with an index from `0..mcrl2_stategraph_local_algorithm_equations`.
        let equation =
            mcrl2_stategraph_local_algorithm_equation(&algorithm, index).expect("equation index is in range");
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
        // `self.index` was validated when this equation was constructed.
        mcrl2_stategraph_local_algorithm_equation(&self.algorithm, self.index).expect("equation index is in range")
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
        Pbes::new(mcrl2_srf_pbes_to_pbes(
            self.srf_pbes.as_ref().expect("srf_pbes UniquePtr should not be null"),
        ))
    }

    /// Returns the initial state (propositional variable instantiation) of the SRF PBES.
    pub fn initial_state(&self) -> PbesPropositionalVariableInstantiation {
        // SAFETY: the FFI returns the live initial-state term of this SRF PBES.
        PbesPropositionalVariableInstantiation::new(unsafe {
            ATerm::from_ptr(mcrl2_sys::pbes::ffi::mcrl2_srf_pbes_initial_state(
                self.srf_pbes.as_ref().expect("srf_pbes UniquePtr should not be null"),
            ))
        })
    }

    /// Unify all parameters of the equations, and rename the summation variables
    /// that clash with a parameter name.
    ///
    /// Unification can give an equation a parameter that shares its name with a
    /// summation variable of one of its summands. During enumeration the value of
    /// that parameter would then be substituted for the bound variable, which can
    /// make the enumeration of an infinite sort non-terminating. mCRL2's
    /// `pbesreach` performs the same renaming after unifying the parameters.
    ///
    /// After unification the C++ `srf_pbes` object is updated in-place, so the
    /// Rust-side equation snapshot must be refreshed to stay consistent.
    pub fn unify_parameters(&mut self, ignore_ce_equations: bool, reset: bool) -> Result<(), MercError> {
        mcrl2_srf_pbes_unify_parameters(self.srf_pbes.pin_mut(), ignore_ce_equations, reset)?;
        mcrl2_srf_pbes_resolve_summand_variable_name_clashes(self.srf_pbes.pin_mut())?;

        // Refresh the equations snapshot from the now-unified C++ object.
        let mut ffi_equations = CxxVector::new();
        mcrl2_srf_pbes_equations(
            ffi_equations.pin_mut(),
            self.srf_pbes.as_ref().expect("srf_pbes UniquePtr should not be null"),
        );
        self.equations = ffi_equations.iter().map(|eq| SrfEquation::new(eq)).collect();
        self._ffi_equations = ffi_equations;

        Ok(())
    }

    /// Returns the data specification of the SRF PBES.
    pub fn data_specification(&self) -> DataSpecification {
        DataSpecification::new(mcrl2_srf_pbes_data_specification(
            self.srf_pbes.as_ref().expect("srf_pbes UniquePtr should not be null"),
        ))
    }

    /// Returns the srf equations of the SRF pbes.
    pub fn equations(&self) -> &[SrfEquation] {
        &self.equations
    }

    /// Returns whether every equation declares the same parameter list (same
    /// variables, same order) — i.e. whether this PBES has already been
    /// through [`SrfPbes::unify_parameters`]. An SRF PBES with no equations is
    /// vacuously unified.
    ///
    /// Consumers that lay out state vectors by "the" parameter vector (there
    /// being only one to speak of requires this) should assert it rather than
    /// silently reading equation 0's list: unification is easy to forget to
    /// call, and the mismatch otherwise only surfaces much later, as a C++
    /// assertion deep inside the enumerator.
    pub fn is_unified(&self) -> bool {
        let mut equations = self.equations.iter();
        let Some(first) = equations.next() else {
            return true;
        };
        let parameters: Vec<DataVariable> = first.variable().parameters().iter().collect();
        equations.all(|eq| {
            let other: Vec<DataVariable> = eq.variable().parameters().iter().collect();
            other == parameters
        })
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
        // SAFETY: the FFI returns the live variable term of this SRF equation,
        // wrapped immediately by `from_ptr`.
        PropositionalVariable::new(unsafe { ATerm::from_ptr(mcrl2_srf_pbes_equation_variable(self.as_ref())) })
    }

    /// Returns the summands of the equation.
    pub fn summands(&self) -> &[SrfSummand] {
        &self.summands
    }

    /// Returns true when the equation is conjunctive (universal, ∀), false for disjunctive (existential, ∃).
    pub fn is_conjunctive(&self) -> bool {
        mcrl2_srf_equation_is_conjunctive(self.as_ref())
    }

    /// Returns true when the equation has a μ (least) fixed-point symbol.
    pub fn is_mu(&self) -> bool {
        mcrl2_srf_equation_is_mu(self.as_ref())
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
        // SAFETY: `self.summand` is a live summand pointer, and the FFI returns
        // the live condition term wrapped immediately by `from_ptr`.
        PbesExpression::new(unsafe {
            ATerm::from_ptr(mcrl2_sys::pbes::ffi::mcrl2_srf_summand_condition(
                self.summand.as_ref().expect("Pointer should be valid"),
            ))
        })
    }

    /// Returns the variable of the summand.
    pub fn variable(&self) -> PbesExpression {
        // SAFETY: `self.summand` is a live summand pointer, and the FFI returns
        // the live variable term wrapped immediately by `from_ptr`.
        PbesExpression::new(unsafe {
            ATerm::from_ptr(mcrl2_sys::pbes::ffi::mcrl2_srf_summand_variable(
                self.summand.as_ref().expect("Pointer should be valid"),
            ))
        })
    }

    /// Returns the summation (existential) parameters of the summand.
    pub fn parameters(&self) -> ATermList<DataVariable> {
        // SAFETY: `self.summand` is a live summand pointer, and the FFI returns
        // the live parameters term wrapped immediately by `from_ptr`.
        ATermList::new(
            unsafe {
                ATerm::from_ptr(mcrl2_srf_summand_parameters(
                    self.summand.as_ref().expect("Pointer should be valid"),
                ))
            }
            .protect(),
        )
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
            "Summand(condition: {}, variable: {})",
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

/// Replace variables in the given PBES expression according to the given substitution sigma.
pub fn substitute_data_expressions(
    expr: &PbesExpression,
    sigma: Vec<(DataExpression, DataExpression)>,
) -> PbesExpression {
    // Do not into_iter here, as we need to keep sigma alive for the call.
    let pairs = to_assignment_pairs(&sigma);

    PbesExpression::new(ATerm::from_unique_ptr(mcrl2_pbes_expression_replace_variables(
        expr.term.get(),
        &pairs,
    )))
}

/// Reorders the parameters of every propositional variable instantiation in
/// the given PBES expression according to the permutation `pi`.
///
/// Returns an error when `pi` is not a permutation of `0..pi.len()`, or when an
/// instantiation with a different number of parameters is encountered.
// The `&Vec<usize>` is required by the `mcrl2-sys` FFI binding.
#[allow(clippy::ptr_arg)]
pub fn reorder_propositional_variables(expr: &PbesExpression, pi: &Vec<usize>) -> Result<PbesExpression, MercError> {
    Ok(PbesExpression::new(ATerm::from_unique_ptr(
        mcrl2_pbes_expression_replace_propositional_variables(expr.term.get(), pi)?,
    )))
}
