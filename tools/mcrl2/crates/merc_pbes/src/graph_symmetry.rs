use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::io::Write;
use std::path::PathBuf;

use itertools::Itertools;
use log::debug;
use log::info;
use log::trace;
use petgraph::graph::NodeIndex;
use petgraph::graph::UnGraph;
use petgraph::graph6::ToGraph6;

use mcrl2::ATermRef;
use mcrl2::DataAbstractionRef;
use mcrl2::DataApplicationRef;
use mcrl2::DataExpressionRef;
use mcrl2::DataFunctionSymbolRef;
use mcrl2::DataMachineNumberRef;
use mcrl2::DataVariable;
use mcrl2::DataVariableRef;
use mcrl2::Pbes;
use mcrl2::PbesConnective;
use mcrl2::PbesExistsRef;
use mcrl2::PbesExpression;
use mcrl2::PbesExpressionRef;
use mcrl2::PbesFlattenIter;
use mcrl2::PbesFlattenStack;
use mcrl2::PbesForallRef;
use mcrl2::PbesImpRef;
use mcrl2::PbesNotRef;
use mcrl2::PbesPropositionalVariableInstantiation;
use mcrl2::PbesPropositionalVariableInstantiationRef;
use mcrl2::SortExpression;
use mcrl2::flatten_associative;
use mcrl2::is_abstraction;
use mcrl2::is_application;
use mcrl2::is_function_symbol;
use mcrl2::is_machine_number;
use mcrl2::is_pbes_and;
use mcrl2::is_pbes_exists;
use mcrl2::is_pbes_forall;
use mcrl2::is_pbes_imp;
use mcrl2::is_pbes_not;
use mcrl2::is_pbes_or;
use mcrl2::is_pbes_propositional_variable_instantiation;
use mcrl2::is_untyped_identifier;
use mcrl2::is_variable;
use mcrl2::is_where_clause;
use mcrl2::pbes_expression_pvi;
use merc_utilities::MercError;

use crate::explore_common::UNIFY_IGNORE_CE_EQUATIONS;
use crate::explore_common::UNIFY_RESET_PARAMETERS;
use crate::permutation::Permutation;

/// Binary function symbols treated as commutative; listing a non-commutative one unsoundly widens the symmetry group.
const COMMUTATIVE_FUNCTION_SYMBOLS: &[&str] = &["&&", "||", "==", "!=", "<=>", "+", "*", "max", "min"];

/// Subset of [`COMMUTATIVE_FUNCTION_SYMBOLS`] that are also associative, and as
/// such can be flattened.
const ASSOCIATIVE_FUNCTION_SYMBOLS: &[&str] = &["&&", "||", "+", "*", "max", "min"];

/// True iff `name` is a known commutative binary symbol at the given arity.
fn is_commutative(name: &str, arity: usize) -> bool {
    arity == 2 && COMMUTATIVE_FUNCTION_SYMBOLS.contains(&name)
}

/// True iff `name` is associative-commutative and its chains should be flattened into one n-ary SDG vertex.
fn is_flat_operator(name: &str, arity: usize) -> bool {
    arity == 2 && ASSOCIATIVE_FUNCTION_SYMBOLS.contains(&name)
}

/// A vertex of the symmetry detection graph.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum SdgVertex {
    /// The k'th (0-based) data parameter `d_k`. Allocated first, so
    /// `NodeIndex(k) == Parameter(k)`.
    Parameter(usize),

    /// A subformula or subterm, identified by its maximally shared term. This
    /// means that (syntactically) identical subterms across different equations
    /// collapse to a single vertex.
    Term(PbesExpression),

    /// The synthetic update position `X_{i,k}`: not a PBES term, and never
    /// deduplicated (one fresh vertex per `(equation, pvi-index,
    /// parameter-index)` triple, by construction).
    Update {
        equation: usize,
        pvi: usize,
        parameter: usize,
    },
}

/// `C(v)`, the "structural" colour of a vertex, excluding the orthogonal
/// `C_eq` component (see [`Sdg::equations`]).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum VertexColour {
    /// `C(x) = par` for a PBES parameter vertex, refined by the parameter's
    /// sort.
    ///
    /// The sort is part of the colour because function symbols are coloured by
    /// name alone and mCRL2 has overloading, e.g. `==` and `<` exist at every
    /// sort but cannot be interchanged.
    Parameter(SortExpression),

    /// A quantifier-bound variable, coloured by its sort but, like
    /// [`VertexColour::Quantifier`], deliberately *not* by name.
    ///
    /// This is coarser than the paper's `C(x) = x` for `x` not a parameter,
    /// which colours a bound variable by its name. In the paper we assume
    /// structural alpha renaming, but in practice we obtain PBESes where all
    /// bound variables are freshly named, so name-colouring would discard most
    /// real symmetries. This fact is enforced by [`SdgBuilder::push_scope`]
    BoundVariable(SortExpression),

    /// `C(f(t1,...,tk)) = f`, identified by name. Also used for nullary
    /// function symbols/constants (`sub#(f()) = {}`, so these are leaves).
    Function(String),

    /// A machine number constant, coloured by its value so that an
    /// automorphism cannot equate two different constants.
    MachineNumber(u64),

    /// `C(phi1 (+) phi2) = (+)` for `(+) in {and, or, not, imp}` (the paper's
    /// grammar only has `{and, or}`; `not`/`imp` are additional mCRL2
    /// connectives given their own distinct colours the same way).
    Connective(Connective),

    /// `C(Qe:D.phi) = (Q,D)`, with the bound variable's *name* dropped for the
    /// reason given on [`VertexColour::BoundVariable`]. Also generalised to a
    /// vector of sorts.
    Quantifier(Quantifier, Vec<SortExpression>),

    /// `C(X(t1,...,tn)) = pvi`.
    Pvi,

    /// `C(X_{i,k}) = update`.
    Update,
}

impl fmt::Display for VertexColour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VertexColour::Parameter(s) => write!(f, "par:{s}"),
            VertexColour::BoundVariable(s) => write!(f, "bvar:{s}"),
            VertexColour::Function(name) => f.write_str(name),
            VertexColour::MachineNumber(n) => write!(f, "{n}"),
            VertexColour::Connective(c) => write!(f, "{c}"),
            VertexColour::Quantifier(q, ss) => write!(f, "{q}:{}", ss.iter().format(",")),
            VertexColour::Pvi => f.write_str("pvi"),
            VertexColour::Update => f.write_str("update"),
        }
    }
}

impl VertexColour {
    fn dot_fill_colour(&self) -> &'static str {
        match self {
            VertexColour::Parameter(_) => "#aec6cf",
            VertexColour::BoundVariable(_) => "#d5e8d4",
            VertexColour::Function(_) => "#fff2cc",
            VertexColour::MachineNumber(_) => "#ffe6cc",
            VertexColour::Connective(_) => "#f8cecc",
            VertexColour::Quantifier(..) => "#e1d5e7",
            VertexColour::Pvi => "#dae8fc",
            VertexColour::Update => "#f5f5f5",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Connective {
    And,
    Or,
    Not,
    Imp,
}

impl fmt::Display for Connective {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Connective::And => "&&",
            Connective::Or => "||",
            Connective::Not => "!",
            Connective::Imp => "=>",
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Quantifier {
    Forall,
    Exists,
    /// Data-level lambda abstraction (`lambda x:D. body`).
    Lambda,
    /// Data-level set/bag comprehension or untyped set/bag comprehension binder.
    Comprehension,
}

impl fmt::Display for Quantifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Quantifier::Forall => "forall",
            Quantifier::Exists => "exists",
            Quantifier::Lambda => "lambda",
            Quantifier::Comprehension => "comp",
        })
    }
}

impl fmt::Display for EdgeColour {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EdgeColour::Uncoloured => Ok(()),
            EdgeColour::Argument(positions) => write!(f, "{}", positions.iter().format(",")),
            EdgeColour::Update(roles) => write!(f, "{}", roles.iter().format(",")),
        }
    }
}

/// `C(e)`, the colour of an edge, passed to GAP as a native edge colour.
///
/// Note that this is a set of positions or roles, not a single position or
/// role, because GAP cannot deal with parallel edges.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum EdgeColour {
    /// The set of positions `{i | t_i = psi}` of a non-commutative
    /// function's argument `psi`. For duplicated positions `f(x,y,x)` we get an
    /// edge to `x` coloured `{1,3}`, not two parallel edges.
    Argument(BTreeSet<usize>),

    /// Any argument of a commutative function; any other edge the paper gives
    /// the "no colour" label.
    Uncoloured,

    /// The set of [`UpdateRole`]s that coincide on the same target vertex of an
    /// update vertex `X_{i,k}`. Note that for copy updates (see
    /// [`SdgBuilder::add_update_vertices`]) the `Data` and `Par` edges land on
    /// the same vertex and are combined, so this is a set, not a singleton.
    Update(BTreeSet<UpdateRole>),
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
enum UpdateRole {
    Pvi,
    Data,
    Par,
}

impl fmt::Display for UpdateRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            UpdateRole::Pvi => "pvi",
            UpdateRole::Data => "data",
            UpdateRole::Par => "par",
        })
    }
}

/// The symmetry detection graph (SDG) of a PBES, as constructed by
/// [`build_sdg`].
pub struct Sdg {
    /// Undirected, matching the paper's "nondirected colored graph". Exactly
    /// one edge per `(phi, psi)` vertex pair: [`EdgeColour`]'s combined
    /// labels resolve the only two situations that could otherwise force a
    /// parallel edge, so `graph` is always simple (no parallel edges, no
    /// self-loops).
    graph: UnGraph<SdgVertex, EdgeColour>,

    /// `C(v)`, indexed by `NodeIndex::index()`.
    colours: Vec<VertexColour>,

    /// `C_eq(v)`: the set of equation indices whose right-hand side reaches
    /// this vertex, indexed by `NodeIndex::index()`.
    equations: Vec<BTreeSet<usize>>,

    /// The unified parameter vector; `parameters[k]` is the vertex
    /// `NodeIndex(k)`.
    parameters: Vec<DataVariable>,

    /// Names of the bound predicate variables, in equation order, for
    /// diagnostics only.
    equation_names: Vec<String>,
}

impl Sdg {
    /// Returns the number of parameters (the size of the permutation domain
    /// that symmetries are ultimately expressed over).
    pub fn num_parameters(&self) -> usize {
        self.parameters.len()
    }

    /// Returns the number of vertices in the graph.
    pub fn num_vertices(&self) -> usize {
        self.graph.node_count()
    }

    /// Returns the number of edges in the graph.
    pub fn num_edges(&self) -> usize {
        self.graph.edge_count()
    }
}

/// Returns the shared parameter vector after unification. Panics if equations
/// disagree, which cannot happen because [`build_sdg`] calls
/// `SrfPbes::unify_parameters` before reaching this point.
fn unified_parameters(equations: &mcrl2::PbesEquations) -> Result<Vec<DataVariable>, MercError> {
    let Some(first) = equations.first() else {
        return Ok(Vec::new());
    };

    let parameters: Vec<DataVariable> = first.variable().parameters().iter().collect();
    for equation in equations.iter().skip(1) {
        let other: Vec<DataVariable> = equation.variable().parameters().iter().collect();
        if other != parameters {
            return Err(format!(
                "Equation for '{}' does not declare the same parameter vector as equation for '{}'; \
                 the symmetry detection graph currently requires every equation to share an \
                 identical (name, sort)-ordered parameter vector.\n  {}: [{}]\n  {}: [{}]",
                equation.variable().name(),
                first.variable().name(),
                first.variable().name(),
                parameters.iter().format(", "),
                equation.variable().name(),
                other.iter().format(", "),
            )
            .into());
        }
    }

    Ok(parameters)
}

/// Builds the symmetry detection graph of `pbes`.
///
/// Preconditions, both checked and reported as an error rather than assumed:
/// - Every equation must already share the same parameter vector; call
///   [`graph_symmetries`] (which calls [`Pbes::unify_parameters`] first) when
///   that precondition is not yet established.
/// - No quantifier or data-level abstraction in any equation may bind a
///   variable with the same name *and* sort as a parameter -- see
///   `SdgBuilder::push_scope` for why such shadowing, though it parses and
///   type-checks fine, cannot be tolerated here.
///
/// `where` clauses are also rejected (see `SdgBuilder::colour_of`); PBES
/// standard form does not produce them, but a user-supplied PBES may still
/// contain one.
pub fn build_sdg(pbes: &Pbes) -> Result<Sdg, MercError> {
    let equations = pbes.equations();
    let parameters = unified_parameters(&equations)?;

    let mut builder = SdgBuilder::new(&parameters);

    // Allocate the parameter vertices first, so `NodeIndex(k) == parameter
    // k`. This is what makes "restrict an automorphism to the parameter
    // vertices" trivial: GAP point `k+1` <-> parameter `k`.
    for (k, parameter) in parameters.iter().enumerate() {
        let term = PbesExpression::new(parameter.clone().into());
        let colour = VertexColour::Parameter(parameter.sort().protect());
        let index = builder.add_vertex(SdgVertex::Parameter(k), colour);
        builder.term_map.insert(term, index);
    }
    debug_assert_eq!(builder.graph.node_count(), parameters.len());

    let n = parameters.len();
    debug!("SDG build: {} parameter(s): [{}]", n, parameters.iter().format(", "));

    for (e, equation) in equations.iter().enumerate() {
        let formula = equation.formula();
        builder.visit(formula.copy(), e)?;

        // Deduplicate PVIs by ATerm identity: `Y(n) && Y(n)` yields two occurrences
        // from the traversal but they share one vertex, so one set of update vertices suffices.
        // The paper instead ranges `i` over all `npred(X)` occurrences, which would give the
        // shared PVI vertex two identical fans of update vertices; those only enlarge `Aut(G)`
        // by permutations of the duplicated fans, which restrict to the identity on parameters.
        debug!(
            "SDG build: equation {} '{}' — {} vertices so far",
            e,
            equation.variable().name(),
            builder.graph.node_count()
        );

        let mut seen_pvis: HashSet<PbesPropositionalVariableInstantiation> = HashSet::new();
        for pvi in pbes_expression_pvi(&formula.copy()) {
            if seen_pvis.insert(pvi.clone()) {
                builder.add_update_vertices(e, &pvi, n)?;
            }
        }
    }

    // Postconditions.
    debug_assert_eq!(builder.colours.len(), builder.equations.len());
    debug_assert_eq!(builder.colours.len(), builder.graph.node_count());
    for k in 0..n {
        debug_assert!(
            matches!(builder.colours[k], VertexColour::Parameter(_)),
            "the first n vertices must be exactly the parameter vertices"
        );
    }
    for (index, colour) in builder.colours.iter().enumerate().skip(n) {
        debug_assert!(
            !matches!(colour, VertexColour::Parameter(_)),
            "no vertex beyond the first n may be coloured Parameter (found at index {index})"
        );
    }
    for node in builder.graph.node_indices() {
        debug_assert!(
            builder.graph.find_edge(node, node).is_none(),
            "the SDG must not contain self-loops"
        );
    }

    Ok(Sdg {
        graph: builder.graph,
        colours: builder.colours,
        equations: builder.equations,
        parameters,
        equation_names: equations.iter().map(|eq| eq.variable().name().to_string()).collect(),
    })
}

/// Builds up an [`Sdg`] incrementally by walking a PBES's right-hand sides.
struct SdgBuilder {
    /// The graph under construction; becomes [`Sdg::graph`].
    graph: UnGraph<SdgVertex, EdgeColour>,

    /// `C(v)` per vertex, indexed by `NodeIndex::index()`.
    colours: Vec<VertexColour>,

    /// `C_eq(v)` per vertex, indexed by `NodeIndex::index()`.
    equations: Vec<BTreeSet<usize>>,

    /// Deduplicates [`SdgVertex::Term`] vertices by (maximally shared) term
    /// identity -- see [`SdgVertex::Term`].
    term_map: HashMap<PbesExpression, NodeIndex>,

    /// Bound (quantifier-/abstraction-scoped) variables currently in scope,
    /// innermost last, used to tell a bound-variable occurrence apart from a
    /// PBES parameter of the same name (see [`VertexColour::BoundVariable`]).
    scope: Vec<DataVariable>,

    /// The unified PBES parameters, checked against every binder in
    /// [`Self::push_scope`] -- see there for why a collision cannot be tolerated.
    parameters: HashSet<DataVariable>,
}

impl SdgBuilder {
    fn new(parameters: &[DataVariable]) -> Self {
        SdgBuilder {
            graph: UnGraph::new_undirected(),
            colours: Vec::new(),
            equations: Vec::new(),
            term_map: HashMap::new(),
            scope: Vec::new(),
            parameters: parameters.iter().cloned().collect(),
        }
    }

    /// Adds a fresh vertex, keeping `colours`/`equations` in lockstep with
    /// `graph`'s node indices.
    fn add_vertex(&mut self, vertex: SdgVertex, colour: VertexColour) -> NodeIndex {
        let index = self.graph.add_node(vertex);
        debug_assert_eq!(index.index(), self.colours.len());
        self.colours.push(colour);
        self.equations.push(BTreeSet::new());
        index
    }

    fn mark_equation(&mut self, node: NodeIndex, equation: usize) {
        self.equations[node.index()].insert(equation);
    }

    /// Adds an edge `(u, v, colour)`, merging into an already-existing `(u,
    /// v)` edge's label set instead of inserting a parallel edge if one is
    /// already present. See [`EdgeColour`].
    fn add_or_merge_edge(&mut self, u: NodeIndex, v: NodeIndex, colour: EdgeColour) {
        if let Some(edge) = self.graph.find_edge(u, v) {
            let existing = self
                .graph
                .edge_weight_mut(edge)
                .expect("find_edge returned a valid edge index");
            trace!(
                "edge: merge v{}--v{} colour {:?} into {:?}",
                u.index(),
                v.index(),
                colour,
                existing
            );
            *existing = merge_edge_colour(existing.clone(), colour);
        } else {
            trace!("edge: add   v{}--v{} colour {:?}", u.index(), v.index(), colour);
            self.graph.add_edge(u, v, colour);
        }
    }

    /// Pushes `variables` onto the bound-variable scope, returning how many
    /// were pushed (for [`Self::pop_scope`]).
    ///
    /// Rejects a variable that collides (same name *and* sort) with a PBES
    /// parameter. [`VertexColour::BoundVariable`] relies on distinguishing a
    /// binder from a parameter by checking whether the *name* is in scope, but
    /// [`Self::visit`] deduplicates vertices by ATerm identity, and mCRL2's
    /// hash-consing makes a bound variable and a parameter of the same name
    /// and sort the very same term. Such a collision would therefore merge the
    /// parameter's vertex with the binder's, coloured whichever the traversal
    /// happens to visit first -- silently corrupting the automorphism
    /// computation rather than raising an error. This precondition is not
    /// enforced by mCRL2's type checker (shadowing a parameter with an
    /// identically-sorted bound variable parses and type-checks fine), so it
    /// must be checked here.
    fn push_scope<I>(&mut self, variables: I, equation: usize) -> Result<usize, MercError>
    where
        I: Iterator<Item = DataVariable>,
    {
        let mut count = 0;
        for variable in variables {
            if self.parameters.contains(&variable) {
                return Err(format!(
                    "equation {equation}: quantifier-/abstraction-bound variable '{variable}' has \
                     the same name and sort as a PBES parameter. Symmetry detection cannot tell such \
                     a bound variable apart from the parameter it shadows (mCRL2's hash-consed terms \
                     make them the same term), so it would corrupt the detected symmetries rather than \
                     just shadowing it as expected. Rename the bound variable, or the parameter, so \
                     that no binder in any equation collides with a parameter."
                )
                .into());
            }
            self.scope.push(variable);
            count += 1;
        }
        Ok(count)
    }

    fn pop_scope(&mut self, count: usize) {
        self.scope.truncate(self.scope.len() - count);
    }

    /// Interns a vertex for `term` (deduplicated by term identity), marks it with `equation`, and
    /// recurses into children. Hand-rolled because `NodeIndex` must flow upward and argument
    /// positions vary per child — constraints the visitor traits cannot express.
    fn visit(&mut self, term: PbesExpressionRef<'_>, equation: usize) -> Result<NodeIndex, MercError> {
        let key = term.protect();
        if let Some(&node) = self.term_map.get(&key) {
            trace!("visit: reused v{} for `{}` (eq {})", node.index(), term, equation);
            self.mark_equation(node, equation);
            return Ok(node);
        }

        let colour = self.colour_of(&key)?;
        let node = self.add_vertex(SdgVertex::Term(key.clone()), colour.clone());
        trace!(
            "visit: new   v{} {:?} for `{}` (eq {})",
            node.index(),
            colour,
            term,
            equation
        );
        self.term_map.insert(key.clone(), node);
        self.mark_equation(node, equation);

        self.visit_children(&key, node, equation)?;
        Ok(node)
    }

    /// Visits `child`, then adds edge `(parent, child, colour)`.
    fn visit_child(
        &mut self,
        parent: NodeIndex,
        child: PbesExpressionRef<'_>,
        colour: EdgeColour,
        equation: usize,
    ) -> Result<(), MercError> {
        let child_node = self.visit(child, equation)?;
        self.add_or_merge_edge(parent, child_node, colour);
        Ok(())
    }

    /// Determines `C(term)`. Does not recurse; see [`Self::visit_children`]
    /// for the edges to `term`'s children.
    fn colour_of(&self, term: &PbesExpression) -> Result<VertexColour, MercError> {
        let r: ATermRef<'_> = term.copy().into();

        if is_pbes_and(&r) {
            Ok(VertexColour::Connective(Connective::And))
        } else if is_pbes_or(&r) {
            Ok(VertexColour::Connective(Connective::Or))
        } else if is_pbes_not(&r) {
            Ok(VertexColour::Connective(Connective::Not))
        } else if is_pbes_imp(&r) {
            Ok(VertexColour::Connective(Connective::Imp))
        } else if is_pbes_forall(&r) {
            let forall = PbesForallRef::from(r);
            let sorts = forall.variables().iter().map(|v| v.sort().protect()).collect();
            Ok(VertexColour::Quantifier(Quantifier::Forall, sorts))
        } else if is_pbes_exists(&r) {
            let exists = PbesExistsRef::from(r);
            let sorts = exists.variables().iter().map(|v| v.sort().protect()).collect();
            Ok(VertexColour::Quantifier(Quantifier::Exists, sorts))
        } else if is_pbes_propositional_variable_instantiation(&r) {
            Ok(VertexColour::Pvi)
        } else if is_variable(&r) {
            let variable = DataVariableRef::from(r);
            if self.scope.iter().any(|bound| bound.name() == variable.name()) {
                Ok(VertexColour::BoundVariable(variable.sort().protect()))
            } else {
                Ok(VertexColour::Parameter(variable.sort().protect()))
            }
        } else if is_application(&r) {
            let application = DataApplicationRef::from(r);
            Ok(VertexColour::Function(
                application.data_function_symbol().name().to_string(),
            ))
        } else if is_function_symbol(&r) {
            let symbol = DataFunctionSymbolRef::from(r);
            Ok(VertexColour::Function(symbol.name().to_string()))
        } else if is_machine_number(&r) {
            let number = DataMachineNumberRef::from(r);
            Ok(VertexColour::MachineNumber(number.value()))
        } else if is_untyped_identifier(&r) {
            // Should not occur in a well-typed PBES; treat as an opaque
            // nullary "function" so it at least gets a stable colour.
            Ok(VertexColour::Function(format!("{r:?}")))
        } else if is_abstraction(&r) {
            // Data-level binder: lambda, forall, exists, set/bag comprehension.
            // Colour the same as the PBES-level quantifier for forall/exists so
            // that structurally identical sub-formulas remain deduplicated.
            let abstraction = DataAbstractionRef::from(r.copy());
            let sorts: Vec<SortExpression> = abstraction.variables().iter().map(|v| v.sort().protect()).collect();
            let bo = abstraction.binding_operator();
            let q = if bo.is_forall() {
                Quantifier::Forall
            } else if bo.is_exists() {
                Quantifier::Exists
            } else if bo.is_lambda() {
                Quantifier::Lambda
            } else {
                Quantifier::Comprehension
            };
            Ok(VertexColour::Quantifier(q, sorts))
        } else if is_where_clause(&r) {
            Err(MercError::from(
                "where clauses are not supported in the symmetry detection graph construction",
            ))
        } else {
            unreachable!("Unknown PBES/data expression kind for term {:?}", r)
        }
    }

    /// Adds edges from `node` (the vertex for `term`) to the vertices of its
    /// immediate children, recursing via [`Self::visit`]. Mirrors
    /// Definition 2's `sub#`, generalized to the mCRL2 connectives and data
    /// expressions as documented on [`VertexColour`], and flattening
    /// associative-commutative chains (see [`is_flat_operator`]).
    fn visit_children(&mut self, term: &PbesExpression, node: NodeIndex, equation: usize) -> Result<(), MercError> {
        let r: ATermRef<'_> = term.copy().into();

        if is_pbes_and(&r) || is_pbes_or(&r) {
            // Flatten the whole chain into a single n-ary vertex, so that `a && b
            // && c` (stored as `&&(&&(a,b),c)`) yields one vertex with three
            // uncoloured edges rather than nested binary ones -- the same
            // treatment `is_flat_operator` gives to associative-commutative data
            // functions below.
            let connective = if is_pbes_and(&r) {
                PbesConnective::And
            } else {
                PbesConnective::Or
            };

            let mut stack = PbesFlattenStack::new();
            for leaf in PbesFlattenIter::new(term.copy(), connective, &mut stack) {
                self.visit_child(node, leaf, EdgeColour::Uncoloured, equation)?;
            }
        } else if is_pbes_imp(&r) {
            // Implication is NOT commutative: lhs => rhs ≠ rhs => lhs.
            // Color the edges by position to prevent spurious symmetries.
            let imp = PbesImpRef::from(r);
            self.visit_child(
                node,
                imp.lhs(),
                EdgeColour::Argument([1].into_iter().collect()),
                equation,
            )?;
            self.visit_child(
                node,
                imp.rhs(),
                EdgeColour::Argument([2].into_iter().collect()),
                equation,
            )?;
        } else if is_pbes_not(&r) {
            let not = PbesNotRef::from(r);
            self.visit_child(node, not.body(), EdgeColour::Uncoloured, equation)?;
        } else if is_pbes_forall(&r) {
            let forall = PbesForallRef::from(r);
            let pushed = self.push_scope(forall.variables().iter(), equation)?;
            self.visit_child(node, forall.body(), EdgeColour::Uncoloured, equation)?;
            self.pop_scope(pushed);
        } else if is_pbes_exists(&r) {
            let exists = PbesExistsRef::from(r);
            let pushed = self.push_scope(exists.variables().iter(), equation)?;
            self.visit_child(node, exists.body(), EdgeColour::Uncoloured, equation)?;
            self.pop_scope(pushed);
        } else if is_pbes_propositional_variable_instantiation(&r) {
            // A PVI is not itself descended into: Definition 3 reaches its
            // arguments only through update vertices (see
            // `SdgBuilder::add_update_vertices`), matching "phi is not
            // itself a PVI" in the edge rule.
        } else if is_variable(&r) || is_function_symbol(&r) || is_machine_number(&r) || is_untyped_identifier(&r) {
            // Leaves: sub#(x) = {} (also true for a nullary function symbol,
            // a machine number, and an untyped identifier).
        } else if is_application(&r) {
            let application = DataApplicationRef::from(r.copy());
            let head = application.data_function_symbol();
            let arity = application.data_arguments().len();

            if is_flat_operator(head.name(), arity) {
                // Flatten the entire chain into a single n-ary vertex so that
                // `a && b && c` (stored as `&&(&&(a,b),c)`) yields one vertex
                // with three uncoloured edges rather than nested binary nodes.
                let name = head.name();
                let expr: DataExpressionRef<'_> = r.into();
                let leaves = flatten_associative(&expr, |t| {
                    is_application(t) && DataApplicationRef::from(t.copy()).data_function_symbol().name() == name
                });
                for leaf in &leaves {
                    self.visit_child(node, leaf.copy().into(), EdgeColour::Uncoloured, equation)?;
                }
            } else {
                let commutative = is_commutative(head.name(), arity);

                // Group arguments by the vertex they resolve to, combining
                // positions for repeated arguments (see `EdgeColour::Argument`).
                let mut by_child: BTreeMap<NodeIndex, BTreeSet<usize>> = BTreeMap::new();
                for (position, argument) in application.data_arguments().enumerate() {
                    let child = self.visit(argument.into(), equation)?;
                    by_child.entry(child).or_default().insert(position + 1);
                }

                for (child, positions) in by_child {
                    let colour = if commutative {
                        EdgeColour::Uncoloured
                    } else {
                        EdgeColour::Argument(positions)
                    };
                    self.add_or_merge_edge(node, child, colour);
                }
            }
        } else if is_abstraction(&r) {
            let abstraction = DataAbstractionRef::from(r);
            let pushed = self.push_scope(abstraction.variables().iter(), equation)?;
            self.visit_child(node, abstraction.body().into(), EdgeColour::Uncoloured, equation)?;
            self.pop_scope(pushed);
        } else if is_where_clause(&r) {
            return Err(MercError::from(
                "where clauses are not supported in the symmetry detection graph construction",
            ));
        } else {
            unreachable!("Unknown PBES/data expression kind for term {:?}", r)
        }
        Ok(())
    }

    /// Adds the update vertices `X_{i,k}` (for `k` in `1..=n`) for a single
    /// PVI `pvi` occurring in the right-hand side of `equation`, along with
    /// their edges to the PVI vertex, the PVI's argument vertices, and the
    /// global parameter vertices.
    fn add_update_vertices(
        &mut self,
        equation: usize,
        pvi: &PbesPropositionalVariableInstantiation,
        n: usize,
    ) -> Result<(), MercError> {
        let pvi_expression: PbesExpression = pvi.clone().into();
        // The PVI vertex must already exist: it was interned while walking
        // the right-hand side (PVIs are leaves of that walk, but they are
        // still visited and given a vertex -- see `visit_children`).
        let pvi_node = self.visit(pvi_expression.copy(), equation)?;

        let arguments: Vec<PbesExpression> = pvi.arguments().iter().map(PbesExpression::from).collect();
        if arguments.len() != n {
            return Err(format!(
                "Predicate variable instance '{}' has {} argument(s), but the unified parameter \
                 vector has {} parameter(s); every predicate variable instance must supply exactly \
                 one argument per parameter.",
                pvi.name(),
                arguments.len(),
                n
            )
            .into());
        }

        // The update vertex's index within this equation's right-hand side
        // (the paper's `i` in `X_{i,k}`): the PVI-index counter used here
        // only needs to be unique per `(equation, pvi term)`, since update
        // vertices are never deduplicated regardless of its value -- so the
        // PVI's own vertex index doubles as a stable, sufficiently unique
        // `i`.
        let i = pvi_node.index();
        debug!(
            "update vertices: eq {} pvi '{}' (v{}) — {} parameter(s)",
            equation,
            pvi.name(),
            i,
            n
        );

        for (k, argument) in arguments.iter().enumerate() {
            let update_vertex = SdgVertex::Update {
                equation,
                pvi: i,
                parameter: k,
            };
            let update_node = self.add_vertex(update_vertex, VertexColour::Update);
            trace!(
                "update vertex v{} X_({},{},{}) (eq {})",
                update_node.index(),
                equation,
                i,
                k,
                equation
            );
            self.mark_equation(update_node, equation);

            let data_node = self.visit(argument.copy(), equation)?;
            let par_node = NodeIndex::new(k);

            // Group the (at most three) targets by vertex identity, and
            // colour each resulting edge with the combined set of roles
            // that land on it (see `EdgeColour::Update`).
            let mut by_target: BTreeMap<NodeIndex, BTreeSet<UpdateRole>> = BTreeMap::new();
            by_target.entry(pvi_node).or_default().insert(UpdateRole::Pvi);
            by_target.entry(data_node).or_default().insert(UpdateRole::Data);
            by_target.entry(par_node).or_default().insert(UpdateRole::Par);

            for (target, roles) in by_target {
                self.add_or_merge_edge(update_node, target, EdgeColour::Update(roles));
            }
        }

        Ok(())
    }
}

/// Merges two edge colours that were found to coincide on the same `(u, v)`
/// vertex pair. See [`EdgeColour`] and [`SdgBuilder::add_or_merge_edge`].
fn merge_edge_colour(a: EdgeColour, b: EdgeColour) -> EdgeColour {
    match (a, b) {
        (EdgeColour::Argument(mut xs), EdgeColour::Argument(ys)) => {
            xs.extend(ys);
            EdgeColour::Argument(xs)
        }
        (EdgeColour::Update(mut xs), EdgeColour::Update(ys)) => {
            xs.extend(ys);
            EdgeColour::Update(xs)
        }
        (EdgeColour::Uncoloured, EdgeColour::Uncoloured) => EdgeColour::Uncoloured,
        (a, b) => unreachable!(
            "Cannot merge incompatible edge colours {:?} and {:?} on the same vertex pair",
            a, b
        ),
    }
}

/// The SDG in the form GAP's Digraphs package expects: a symmetric digraph
/// (every undirected edge as two opposite directed arcs) with 1-based points.
pub struct GapGraph {
    /// `out_neighbours[u]` lists the 1-based out-neighbours of vertex `u+1`.
    out_neighbours: Vec<Vec<usize>>,
    /// Positionally aligned with `out_neighbours`; `edge_colours[u][j]` is the
    /// dense colour index of the j-th arc out of vertex `u+1`.
    edge_colours: Vec<Vec<usize>>,
    /// `vertex_colours[u]` is the dense colour index of vertex `u+1`.
    vertex_colours: Vec<usize>,
    /// How many of the leading vertices (NodeIndex 0..n-1) are parameter vertices.
    pub num_parameters: usize,
}

impl Sdg {
    pub fn to_gap_graph(&self) -> GapGraph {
        let n = self.graph.node_count();
        let mut vc_map: HashMap<String, usize> = HashMap::new();
        let mut next_vc = 1usize;
        let mut vertex_colours = vec![0usize; n];

        for node in self.graph.node_indices() {
            let key = format!("{:?}|{:?}", self.colours[node.index()], self.equations[node.index()]);
            let dense = *vc_map.entry(key).or_insert_with(|| {
                let c = next_vc;
                next_vc += 1;
                c
            });
            vertex_colours[node.index()] = dense;
        }

        let mut ec_map: HashMap<String, usize> = HashMap::new();
        let mut next_ec = 1usize;
        let mut out_neighbours = vec![Vec::new(); n];
        let mut edge_colours = vec![Vec::new(); n];

        for edge in self.graph.edge_indices() {
            let (u, v) = self.graph.edge_endpoints(edge).unwrap();
            let colour = self.graph.edge_weight(edge).unwrap();
            let ec_key = format!("{:?}", colour);
            let dense_ec = *ec_map.entry(ec_key).or_insert_with(|| {
                let c = next_ec;
                next_ec += 1;
                c
            });
            out_neighbours[u.index()].push(v.index() + 1);
            edge_colours[u.index()].push(dense_ec);
            out_neighbours[v.index()].push(u.index() + 1);
            edge_colours[v.index()].push(dense_ec);
        }

        debug_assert!(
            out_neighbours
                .iter()
                .zip(&edge_colours)
                .all(|(nb, ec)| nb.len() == ec.len()),
            "out_neighbours and edge_colours must be positionally aligned"
        );
        debug_assert!(
            out_neighbours
                .iter()
                .enumerate()
                .all(|(u, nb)| { nb.iter().zip(&edge_colours[u]).collect::<HashSet<_>>().len() == nb.len() }),
            "no two arcs from the same source may share both target and colour"
        );

        GapGraph {
            out_neighbours,
            edge_colours,
            vertex_colours,
            num_parameters: self.parameters.len(),
        }
    }
}

/// Writes the SDG as a Graphviz DOT file, including vertex/edge colours.
pub fn write_dot<W>(sdg: &Sdg, w: &mut W) -> Result<(), MercError>
where
    W: Write,
{
    writeln!(w, "graph sdg {{")?;
    writeln!(w, "  node [style=filled];")?;

    for node in sdg.graph.node_indices() {
        let i = node.index();
        let vc = &sdg.colours[i];

        let label = match vc {
            VertexColour::Parameter(_) => sdg.parameters[i].name().to_string(),
            // Update nodes are unlabeled; shape + dashed edges identify them.
            VertexColour::Update => String::new(),
            VertexColour::Pvi => {
                if let SdgVertex::Term(expression) = &sdg.graph[node] {
                    PbesPropositionalVariableInstantiationRef::from(expression.copy())
                        .name()
                        .to_string()
                } else {
                    unreachable!()
                }
            }
            VertexColour::BoundVariable(_) => {
                if let SdgVertex::Term(expression) = &sdg.graph[node] {
                    let r: ATermRef<'_> = expression.copy().into();
                    DataVariableRef::from(r).name().to_string()
                } else {
                    unreachable!()
                }
            }
            VertexColour::Quantifier(q, _) => {
                if let SdgVertex::Term(expression) = &sdg.graph[node] {
                    let r: ATermRef<'_> = expression.copy().into();
                    let vars: Vec<String> = if is_pbes_forall(&r) {
                        PbesForallRef::from(r)
                            .variables()
                            .iter()
                            .map(|v| format!("{}:{}", v.name(), v.sort().pretty_print()))
                            .collect()
                    } else if is_pbes_exists(&r) {
                        PbesExistsRef::from(r)
                            .variables()
                            .iter()
                            .map(|v| format!("{}:{}", v.name(), v.sort().pretty_print()))
                            .collect()
                    } else {
                        DataAbstractionRef::from(r)
                            .variables()
                            .iter()
                            .map(|v| format!("{}:{}", v.name(), v.sort().pretty_print()))
                            .collect()
                    };
                    format!("{q} {}", vars.join(","))
                } else {
                    unreachable!()
                }
            }
            _ => vc.to_string(),
        };

        let shape = match vc {
            VertexColour::Parameter(_) => "box",
            VertexColour::Update => "diamond",
            VertexColour::Pvi => "hexagon",
            VertexColour::Quantifier(..) => "parallelogram",
            _ => "ellipse",
        };

        let fill = vc.dot_fill_colour();
        if matches!(vc, VertexColour::Update) {
            writeln!(
                w,
                "  n{i} [label=\"\", shape=diamond, fillcolor=\"{fill}\", width=0.2, height=0.2, fixedsize=true];"
            )?;
        } else {
            writeln!(w, "  n{i} [label=\"{label}\", shape={shape}, fillcolor=\"{fill}\"];")?;
        }
    }

    for edge in sdg.graph.edge_indices() {
        let (u, v) = sdg.graph.edge_endpoints(edge).unwrap();
        let colour = sdg.graph.edge_weight(edge).unwrap();
        let elabel = colour.to_string();

        if matches!(colour, EdgeColour::Update(_)) {
            if elabel.is_empty() {
                writeln!(w, "  n{} -- n{} [style=dashed];", u.index(), v.index())?;
            } else {
                writeln!(
                    w,
                    "  n{} -- n{} [style=dashed, label=\"{elabel}\"];",
                    u.index(),
                    v.index()
                )?;
            }
        } else if elabel.is_empty() {
            writeln!(w, "  n{} -- n{};", u.index(), v.index())?;
        } else {
            writeln!(w, "  n{} -- n{} [label=\"{elabel}\"];", u.index(), v.index())?;
        }
    }

    writeln!(w, "}}")?;
    Ok(())
}

/// petgraph only implements the 4-byte N(n) encoding; the format supports up to 68_719_476_735.
const GRAPH6_MAX_NODES: usize = 258_047;

/// Renders the structural skeleton of the SDG in graph6 format.
///
/// graph6 encodes only the adjacency structure of a simple undirected graph;
/// vertex and edge colours are not representable. Use this as a debug or
/// interop artifact (nauty's `showg`, GAP's `DigraphFromGraph6String`), not
/// as the channel through which colours reach GAP (that goes through the script).
pub fn graph6_string(sdg: &Sdg) -> Result<String, MercError> {
    if sdg.graph.node_count() > GRAPH6_MAX_NODES {
        return Err(
            format!("petgraph's graph6 encoder does not support graphs over {GRAPH6_MAX_NODES} vertices").into(),
        );
    }
    Ok(sdg.graph.graph6_string())
}

/// Generates a self-contained GAP script that computes `Aut(G)` via the
/// Digraphs package, restricts generators to the parameter vertices, and
/// prints the result between `SDG-BEGIN`/`SDG-END` sentinels.
///
/// Every statement ends in `;;` because a script fed on stdin is read as an
/// interactive session where a single `;` echoes the value to stdout.
/// The sentinels also defend against GAP's exit-0-on-syntax-error behaviour.
fn gap_script(graph: &GapGraph) -> String {
    let n = graph.num_parameters;
    let num_vertices = graph.out_neighbours.len();

    let neighbours_str = (0..num_vertices)
        .map(|u| {
            if graph.out_neighbours[u].is_empty() {
                "[]".to_string()
            } else {
                format!("[{}]", graph.out_neighbours[u].iter().join(","))
            }
        })
        .join(",");

    let vc_str = graph.vertex_colours.iter().join(",");

    let ec_str = (0..num_vertices)
        .map(|u| {
            if graph.edge_colours[u].is_empty() {
                "[]".to_string()
            } else {
                format!("[{}]", graph.edge_colours[u].iter().join(","))
            }
        })
        .join(",");

    format!(
        r#"SetPrintFormattingStatus("*stdout*", false);;
if LoadPackage("digraphs") = fail then
  Error("the GAP package 'Digraphs' could not be loaded; install it from https://digraphs.github.io/Digraphs/");
fi;;
D := Digraph([{neighbours_str}]);;
vcolours := [{vc_str}];;
ecolours := [{ec_str}];;
A := AutomorphismGroup(D, vcolours, ecolours);;
R := List(GeneratorsOfGroup(A), g -> RestrictedPerm(g, [1..{n}]));;
P := GroupByGenerators(R, ());;
Print("SDG-BEGIN\n");;
Print("order ", Size(A), "\n");;
Print("restricted ", Size(P), "\n");;
for g in GeneratorsOfGroup(P) do
  for i in [1..{n}] do Print(i^g, " "); od;
  Print("\n");
od;;
Print("SDG-END\n");;
QUIT;;
"#
    )
}

/// Configuration for invoking the external GAP process.
pub struct GapConfig {
    /// Path or name of the GAP executable (default: `"gap"` on `$PATH`).
    pub executable: String,
    /// If set, the generated GAP script is also written to this file.
    pub dump_script: Option<PathBuf>,
}

impl Default for GapConfig {
    fn default() -> Self {
        GapConfig {
            executable: "gap".to_string(),
            dump_script: None,
        }
    }
}

/// Invokes GAP with the generated script on stdin and returns the captured stdout.
///
/// Flags used (verified on GAP 4.12.1):
/// - `-q` suppresses the banner and prompt
/// - `-A` disables autoloading of suggested packages
/// - `--quitonbreak` makes runtime errors exit with non-zero status
///   (note: do NOT add `-T`/`--nobreakloop`, which defeats `--quitonbreak`)
///
/// Note that `-r` must NOT be added: it disables the user GAP root directory
/// `~/.gap`, which is where both `InstallPackage` and a manual build without
/// root privileges install the Digraphs package.
pub fn run_gap(script: &str, config: &GapConfig) -> Result<String, MercError> {
    if let Some(path) = &config.dump_script {
        fs::write(path, script)
            .map_err(|e| MercError::from(format!("failed to write GAP script to '{}': {}", path.display(), e)))?;
    }

    let output = duct::cmd(&*config.executable, ["-q", "-A", "--quitonbreak"])
        .stdin_bytes(script.to_owned())
        .stdout_capture()
        .stderr_capture()
        .unchecked()
        .run()
        .map_err(|e| {
            if e.kind() == ErrorKind::NotFound {
                MercError::from(format!(
                    "GAP executable '{}' not found; install GAP from https://www.gap-system.org/ \
                     or pass --gap-path to specify its location",
                    config.executable
                ))
            } else {
                MercError::from(e)
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut msg = format!("GAP exited with {}", output.status);
        if stderr.contains("digraphs") || stderr.contains("Digraphs") {
            msg.push_str(
                "; the Digraphs package may not be installed — \
                 see https://digraphs.github.io/Digraphs/",
            );
        } else if !stderr.trim().is_empty() {
            msg.push_str(&format!(": {}", stderr.trim()));
        }
        return Err(msg.into());
    }

    String::from_utf8(output.stdout).map_err(|e| MercError::from(e.to_string()))
}

/// Parses the output produced by the GAP script between the `SDG-BEGIN`/`SDG-END`
/// sentinels back into `(automorphism_group_order, symmetry_group_order, generators)`.
///
/// GAP prints permutations as 1-indexed image vectors; this function converts
/// them to 0-indexed and builds [`Permutation`] values via [`Permutation::from_mapping`].
fn parse_gap_output(stdout: &str, num_parameters: usize) -> Result<(u128, u128, Vec<Permutation>), MercError> {
    // Extract lines strictly between the sentinels.
    let begin_pos = stdout.find("SDG-BEGIN").ok_or_else(|| {
        MercError::from("GAP output is missing 'SDG-BEGIN' sentinel — check for syntax errors in the generated script")
    })?;
    let after_begin = &stdout[begin_pos + "SDG-BEGIN".len()..];
    let end_pos = after_begin
        .find("SDG-END")
        .ok_or_else(|| MercError::from("GAP output is missing 'SDG-END' sentinel"))?;
    let inner = after_begin[..end_pos].trim();

    let mut lines = inner.lines();

    let aut_order: u128 = lines
        .next()
        .and_then(|l| l.strip_prefix("order "))
        .and_then(|s| s.trim().parse().ok())
        .ok_or_else(|| MercError::from("expected 'order <n>' as first line inside sentinels"))?;

    let sym_order: u128 = lines
        .next()
        .and_then(|l| l.strip_prefix("restricted "))
        .and_then(|s| s.trim().parse().ok())
        .ok_or_else(|| MercError::from("expected 'restricted <n>' as second line inside sentinels"))?;

    let mut generators = Vec::new();
    for (line_no, line) in lines.enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let images: Vec<usize> = line
            .split_whitespace()
            .map(|s| {
                let v = s
                    .parse::<usize>()
                    .map_err(|_| MercError::from(format!("invalid image '{}' on generator line {}", s, line_no)))?;
                v.checked_sub(1).ok_or_else(|| {
                    MercError::from(format!(
                        "invalid image '{}' on generator line {} (expected >= 1)",
                        s, line_no
                    ))
                })
            })
            .collect::<Result<_, _>>()?;

        if images.len() != num_parameters {
            return Err(format!(
                "generator line {} has {} images but expected {} (num_parameters)",
                line_no,
                images.len(),
                num_parameters
            )
            .into());
        }

        let mapping: Vec<(usize, usize)> = images.into_iter().enumerate().filter(|(from, to)| from != to).collect();

        if !mapping.is_empty() {
            generators.push(Permutation::from_mapping(mapping));
        }
    }

    Ok((aut_order, sym_order, generators))
}

/// Result returned by [`graph_symmetries`].
pub struct GraphSymmetryResult {
    /// The symmetry detection graph the automorphisms were computed on.
    pub sdg: Sdg,

    /// `|Aut(G)|`, the order of the automorphism group of the whole SDG.
    pub automorphism_group_order: u128,

    /// `|Sym(pbes)|`, the order after restricting to the parameter vertices.
    pub symmetry_group_order: u128,

    /// Generators of `Sym(pbes)`, as permutations of the parameter indices.
    pub generators: Vec<Permutation>,
}

/// Constructs the "symmetry detection graph" (SDG) of a PBES, and uses it (via
/// the GAP automorphism-group computation, see [`run_gap`]) to derive
/// permutation symmetries of the PBES's parameters using auto morphisms of the
/// SDG.
pub fn graph_symmetries(pbes: &Pbes, config: &GapConfig) -> Result<GraphSymmetryResult, MercError> {
    // Unify on a copy so build_sdg works on a PBES with one parameter vector
    // while the caller keeps the equations it passed in.
    let mut pbes = pbes.clone();
    pbes.unify_parameters(UNIFY_IGNORE_CE_EQUATIONS, UNIFY_RESET_PARAMETERS)?;
    let sdg = build_sdg(&pbes)?;
    info!(
        "SDG: {} vertices, {} edges, {} parameters",
        sdg.num_vertices(),
        sdg.num_edges(),
        sdg.num_parameters()
    );
    debug!("SDG equation names: [{}]", sdg.equation_names.iter().format(", "));

    let gap_graph = sdg.to_gap_graph();
    let script = gap_script(&gap_graph);
    let stdout = run_gap(&script, config)?;
    let (aut_order, sym_order, generators) = parse_gap_output(&stdout, gap_graph.num_parameters)?;

    info!(
        "|Aut(G)| = {}, |Sym(pbes)| = {}, {} generator(s)",
        aut_order,
        sym_order,
        generators.len()
    );

    Ok(GraphSymmetryResult {
        sdg,
        automorphism_group_order: aut_order,
        symmetry_group_order: sym_order,
        generators,
    })
}

#[cfg(test)]
mod tests {
    use mcrl2::Pbes;
    use merc_utilities::test_logger;
    use petgraph::graph::NodeIndex;
    use petgraph::visit::EdgeRef;
    use std::sync::OnceLock;

    use test_case::test_case;

    use super::EdgeColour;
    use super::GapConfig;
    use super::GraphSymmetryResult;
    use super::Quantifier;
    use super::UpdateRole;
    use super::VertexColour;
    use super::build_sdg;
    use super::graph_symmetries;

    /// Returns `true` when GAP with the Digraphs package is usable.
    /// Cached so the probe script runs at most once per test process.
    fn gap_with_digraphs_available() -> bool {
        static AVAILABLE: OnceLock<bool> = OnceLock::new();
        *AVAILABLE.get_or_init(|| {
            duct::cmd("gap", ["-q", "-A", "--quitonbreak"])
                .stdin_bytes("if LoadPackage(\"digraphs\") = fail then QUIT_GAP(1); fi;; QUIT_GAP(0);;")
                .stdout_null()
                .stderr_null()
                .unchecked()
                .run()
                .map(|o| o.status.success())
                .unwrap_or(false)
        })
    }

    /// Runs `graph_symmetries` on the given PBES source, skipping if GAP or
    /// Digraphs is unavailable.
    fn check_gap_symmetries(source: &str) -> Option<GraphSymmetryResult> {
        if !gap_with_digraphs_available() {
            return None;
        }
        let pbes = mcrl2::Pbes::from_text(source).unwrap();
        Some(graph_symmetries(&pbes, &GapConfig::default()).unwrap())
    }

    #[test_case(include_str!("../../../../../examples/pbes/a.text.pbes"); "a")]
    #[test_case(include_str!("../../../../../examples/pbes/b.text.pbes"); "b")]
    #[test_case(include_str!("../../../../../examples/pbes/c.text.pbes"); "c")]
    #[test_case(include_str!("../../../../../examples/pbes/alloc3.text.pbes");  "alloc3")]
    #[test_case(include_str!("../../../../../examples/pbes/alloc7.text.pbes");  "alloc7")]
    #[test_case(include_str!("../../../../../examples/pbes/alloc9.text.pbes");  "alloc9")]
    #[test_case(include_str!("../../../../../examples/pbes/dining8.text.pbes"); "dining8")]
    fn test_gap_symmetries(source: &str) {
        check_gap_symmetries(source);
    }

    /// Checks the graph structure of the SDG built from `c.text.pbes`, which
    /// has 4 parameters.
    #[test]
    fn test_c_pbes_parameter_vertices() {
        test_logger();
        let pbes = Pbes::from_text(include_str!("../../../../../examples/pbes/c.text.pbes")).unwrap();
        let sdg = build_sdg(&pbes).unwrap();

        assert_eq!(sdg.num_parameters(), 4, "c.text.pbes has 4 parameters");
        for k in 0..4 {
            assert!(
                matches!(sdg.colours[k], VertexColour::Parameter(_)),
                "vertex {k} should be the k'th parameter"
            );
        }
        // No other vertex may be coloured Parameter.
        assert!(
            sdg.colours
                .iter()
                .skip(4)
                .all(|c| !matches!(c, VertexColour::Parameter(_)))
        );

        assert!(sdg.num_vertices() > 4, "there should be vertices beyond the parameters");
        assert!(sdg.num_edges() > 0);
    }

    #[test]
    fn test_a_and_b_pbes_build_without_error() {
        test_logger();
        for source in [
            include_str!("../../../../../examples/pbes/a.text.pbes"),
            include_str!("../../../../../examples/pbes/b.text.pbes"),
        ] {
            let pbes = Pbes::from_text(source).unwrap();
            build_sdg(&pbes).unwrap();
        }
    }

    /// Identical subterms across two different equations collapse to a
    /// single vertex.
    #[test]
    fn test_identical_subterms_share_one_vertex_across_equations() {
        test_logger();
        let pbes = Pbes::from_text(
            "pbes mu X(n: Nat) = val(n == 0) && Y(n);
                  mu Y(n: Nat) = val(n == 0) && X(n);
             init X(0);",
        )
        .unwrap();
        let sdg = build_sdg(&pbes).unwrap();

        // `n == 0` occurs (syntactically identically) in both equations, so
        // it must be exactly one vertex, reachable from both equation 0 (X)
        // and equation 1 (Y).
        let condition_node = sdg
            .graph
            .node_indices()
            .find(|&index| matches!(&sdg.colours[index.index()], VertexColour::Function(name) if name == "=="))
            .expect("there should be a vertex for the '==' application");
        assert_eq!(
            sdg.equations[condition_node.index()],
            [0, 1].into_iter().collect(),
            "the shared condition must be reachable from both equations"
        );
    }

    /// Update vertices are never deduplicated, even when two `(equation,
    /// pvi, parameter)` triples would otherwise look structurally identical.
    #[test]
    fn test_update_vertices_are_never_deduplicated() {
        test_logger();
        let pbes = Pbes::from_text(
            "pbes mu X(n: Nat) = Y(n) && Y(n);
                  mu Y(n: Nat) = val(n == 0);
             init X(0);",
        )
        .unwrap();
        let sdg = build_sdg(&pbes).unwrap();

        // Two distinct PVI occurrences `Y(n)` in X's right-hand side, each
        // contributing its own update vertex X_{i,0}, even though both
        // PVIs are syntactically identical.
        let update_count = sdg
            .graph
            .node_indices()
            .filter(|&index| sdg.colours[index.index()] == VertexColour::Update)
            .count();
        assert_eq!(update_count, 1, "one PVI occurrence (post-dedup) times one parameter");
    }

    /// The head function symbol of an application is a *colour*, not a vertex:
    /// `n + n` should produce exactly two vertices, not three.
    #[test]
    fn test_head_function_symbol_is_not_a_vertex() {
        test_logger();
        let pbes = Pbes::from_text("pbes mu X(n: Nat) = val(n + n == n); init X(0);").unwrap();
        let sdg = build_sdg(&pbes).unwrap();

        let function_names: Vec<&String> = sdg
            .colours
            .iter()
            .filter_map(|colour| match colour {
                VertexColour::Function(name) => Some(name),
                _ => None,
            })
            .collect();

        // "+" and "==" should both appear as vertex colours.
        assert!(function_names.iter().any(|name| name.as_str() == "+"));
        assert!(function_names.iter().any(|name| name.as_str() == "=="));
    }

    /// A non-commutative function applied twice to the *same* argument
    /// (`n - n`) produces exactly one edge to `n`, coloured with the
    /// combined position set `{1,2}`.
    #[test]
    fn test_noncommutative_repeated_argument_gets_combined_label() {
        test_logger();
        let pbes = Pbes::from_text("pbes mu X(n: Int) = val(n - n == 0); init X(0);").unwrap();
        let sdg = build_sdg(&pbes).unwrap();

        let minus_node = sdg
            .graph
            .node_indices()
            .find(|&index| matches!(&sdg.colours[index.index()], VertexColour::Function(name) if name == "-"))
            .expect("there should be a vertex for the '-' application");

        // Use edges_connecting to count only edges between minus_node and the parameter `n`
        // (NodeIndex(0)), not all incident edges (which would include the parent `==` edge).
        let n_node = NodeIndex::new(0);
        let edges: Vec<_> = sdg.graph.edges_connecting(minus_node, n_node).collect();
        assert_eq!(edges.len(), 1, "n - n should have exactly one edge to 'n'");
        assert_eq!(*edges[0].weight(), EdgeColour::Argument([1, 2].into_iter().collect()));
    }

    /// A commutative function applied twice to the same argument (`n ==
    /// n`) still produces exactly one (uncoloured) edge.
    #[test]
    fn test_commutative_repeated_argument_stays_uncoloured() {
        test_logger();
        let pbes = Pbes::from_text("pbes mu X(n: Nat) = val(n == n); init X(0);").unwrap();
        let sdg = build_sdg(&pbes).unwrap();

        let eq_node = sdg
            .graph
            .node_indices()
            .find(|&index| matches!(&sdg.colours[index.index()], VertexColour::Function(name) if name == "=="))
            .expect("there should be a vertex for the '==' application");

        let edges: Vec<_> = sdg.graph.edges(eq_node).collect();
        assert_eq!(edges.len(), 1, "n == n should reach 'n' via exactly one edge");
        assert_eq!(*edges[0].weight(), EdgeColour::Uncoloured);
    }

    /// A PVI that copies a parameter unchanged (`X(n)` from within `X`'s own
    /// right-hand side) makes the `data(X,i,k)` and `d_k` update-edge
    /// targets coincide: the update vertex must have exactly one combined
    /// `{Data, Par}`-coloured edge to `n`, plus a separate `{Pvi}`-coloured
    /// edge to the PVI vertex -- not three edges, and not a silently
    /// dropped role.
    #[test]
    fn test_update_edge_combines_roles_on_parameter_copy() {
        test_logger();
        let pbes = Pbes::from_text("pbes mu X(n: Nat) = X(n); init X(0);").unwrap();
        let sdg = build_sdg(&pbes).unwrap();

        let update_node = sdg
            .graph
            .node_indices()
            .find(|&index| sdg.colours[index.index()] == VertexColour::Update)
            .expect("there should be exactly one update vertex");

        let mut edges: Vec<_> = sdg.graph.edges(update_node).map(|e| e.weight().clone()).collect();
        edges.sort_by_key(|colour| format!("{colour:?}"));

        assert_eq!(
            edges.len(),
            2,
            "expected one {{Pvi}} edge and one combined {{Data,Par}} edge"
        );
        assert!(edges.contains(&EdgeColour::Update([UpdateRole::Pvi].into_iter().collect())));
        assert!(edges.contains(&EdgeColour::Update(
            [UpdateRole::Data, UpdateRole::Par].into_iter().collect()
        )));

        // And the combined edge's target must be the parameter vertex `n`,
        // i.e. NodeIndex(0).
        let combined_target = sdg
            .graph
            .edges(update_node)
            .find(|e| *e.weight() == EdgeColour::Update([UpdateRole::Data, UpdateRole::Par].into_iter().collect()))
            .map(|e| e.target())
            .unwrap();
        assert_eq!(combined_target, NodeIndex::new(0));
    }

    /// Two equations declaring different parameter vectors are rejected
    /// with a clear error rather than silently producing a malformed graph.
    #[test]
    fn test_rejects_non_uniform_parameter_vectors() {
        test_logger();
        // Use the same arity but different parameter names so mCRL2 accepts the PBES
        // while `unified_parameters` still rejects it (n:Nat ≠ m:Nat as ATerms).
        let pbes = Pbes::from_text(
            "pbes mu X(n: Nat) = Y(n);
                  mu Y(m: Nat) = val(true);
             init X(0);",
        )
        .unwrap();

        let result = build_sdg(&pbes);
        assert!(result.is_err(), "differing parameter vectors must be rejected");
    }

    /// A quantifier-bound variable that collides with a parameter (same name
    /// *and* sort) must be rejected: mCRL2 type-checks the shadowing fine, but
    /// hash-consing would make the two the same ATerm and silently merge their
    /// SDG vertices (see [`super::SdgBuilder::push_scope`]).
    #[test]
    fn test_rejects_pbes_quantifier_shadowing_parameter() {
        test_logger();
        let pbes = Pbes::from_text("pbes mu X(n: Nat) = exists n: Nat . val(n > 0); init X(0);").unwrap();

        let result = build_sdg(&pbes);
        assert!(
            result.is_err(),
            "a bound variable colliding with a parameter must be rejected"
        );
    }

    /// The same collision through a data-level binder (`lambda`/comprehension,
    /// here `forall` written as a data expression) must also be rejected.
    #[test]
    fn test_rejects_data_level_binder_shadowing_parameter() {
        test_logger();
        let pbes = Pbes::from_text("pbes mu X(n: Nat) = val(forall n: Nat . n > 0); init X(0);").unwrap();

        let result = build_sdg(&pbes);
        assert!(
            result.is_err(),
            "a data-level binder shadowing a parameter must be rejected"
        );
    }

    /// Reusing a parameter's *name* with a *different* sort is not a collision
    /// (they are distinct ATerms), so it must still be accepted.
    #[test]
    fn test_accepts_bound_variable_reusing_name_with_different_sort() {
        test_logger();
        let pbes = Pbes::from_text("pbes mu X(n: Nat) = exists n: Bool . val(n); init X(0);").unwrap();

        let sdg = build_sdg(&pbes).unwrap();
        assert_eq!(sdg.num_parameters(), 1);
    }

    /// A bound (quantifier-scoped) variable must not be coloured the same
    /// as a PBES parameter, even when it shares a name with one -- otherwise
    /// an automorphism could conflate the two.
    #[test]
    fn test_bound_variable_is_not_coloured_as_parameter() {
        test_logger();
        // Use a different name for the bound variable so it is a distinct ATerm from the
        // parameter `n`. Under mCRL2 hash-consing, `n:Nat` and `n:Nat` are the same ATerm
        // and would therefore collapse to one vertex regardless of scope.
        let pbes = Pbes::from_text("pbes mu X(n: Nat) = exists m: Nat . val(n == m); init X(0);").unwrap();
        let sdg = build_sdg(&pbes).unwrap();

        let parameter_count = sdg
            .colours
            .iter()
            .filter(|c| matches!(c, VertexColour::Parameter(_)))
            .count();
        assert_eq!(parameter_count, 1);

        let has_bound_variable = sdg.colours.iter().any(|c| matches!(c, VertexColour::BoundVariable(_)));
        assert!(
            has_bound_variable,
            "the quantifier-bound 'm' should be coloured BoundVariable"
        );
    }

    /// Parameters of different sorts must never be interchangeable.
    ///
    /// Function symbols are coloured by name only (`==` exists at every sort), so
    /// without the sort in the parameter colour these two parameters have
    /// isomorphic neighbourhoods and GAP reports a `Bool` <-> `Nat` swap. Applying
    /// it would hand `set_assignments` a value of the wrong sort.
    #[test]
    fn test_parameters_of_different_sorts_are_not_interchangeable() {
        test_logger();
        let pbes = Pbes::from_text("pbes nu X(b: Bool, n: Nat) = X(b, n); init X(true, 0);").unwrap();
        let sdg = build_sdg(&pbes).unwrap();

        assert_ne!(
            sdg.colours[0], sdg.colours[1],
            "a Bool and a Nat parameter must get different colours"
        );
    }

    /// A data-level binder expression (e.g. `val(exists m:Nat. n==m)`) must be
    /// assigned a `Quantifier` colour and its bound variable must be coloured
    /// `BoundVariable`, not `Parameter`.
    #[test]
    fn test_data_level_binder_does_not_panic() {
        test_logger();
        // `val(exists m: Nat . n == m)` wraps a data-level binder inside val(...)
        // so the ATerm reaching colour_of is `Binder(Exists, [m:Nat], ==(n,m))`.
        let pbes = Pbes::from_text("pbes mu X(n: Nat) = val(exists m: Nat . n == m); init X(0);").unwrap();
        let sdg = build_sdg(&pbes).unwrap();

        let has_quantifier = sdg
            .colours
            .iter()
            .any(|c| matches!(c, VertexColour::Quantifier(Quantifier::Exists, _)));
        assert!(
            has_quantifier,
            "data-level exists binder must produce a Quantifier(Exists) vertex"
        );

        let has_bound_variable = sdg.colours.iter().any(|c| matches!(c, VertexColour::BoundVariable(_)));
        assert!(
            has_bound_variable,
            "the data-level bound 'm' must be coloured BoundVariable"
        );
    }

    #[test]
    fn probe_empty_pbes() {
        test_logger();
        let pbes = Pbes::from_text("pbes init val(true);").unwrap();
        let sdg = build_sdg(&pbes);
        println!("empty pbes build_sdg result: {}", sdg.is_ok());
        if let Ok(sdg) = sdg {
            println!(
                "num_parameters={} num_vertices={} num_edges={}",
                sdg.num_parameters(),
                sdg.num_vertices(),
                sdg.num_edges()
            );
        }
    }
}
