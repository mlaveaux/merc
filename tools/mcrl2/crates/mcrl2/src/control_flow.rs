use std::collections::HashMap;
use std::fmt;

use itertools::Itertools;

use crate::ATerm;
use crate::ATermList;
use crate::DataExpression;
use crate::DataExpressionRef;
use crate::DataVariable;
use crate::LearnSuccessorsContext;
use crate::free_variables_data_expression;
use crate::is_application;
use crate::is_variable;

/// A summand-like structure that can be analysed.
pub trait CfgSummand {
    /// The summand's guard.
    fn condition(&self) -> &DataExpression;

    /// The parameter assignments performed by the summand, as a list of
    /// `data::assignment` terms (`lhs := rhs`, read with `.arg(0)` /
    /// `.arg(1)`). A parameter absent here is left unchanged by the summand.
    fn write_assignments(&self) -> &ATermList<ATerm>;
}

/// A control flow parameter's domain is a finite set of *locations* — the
/// closed values it can take. A summand that constrains and/or changes a
/// control flow parameter contributes one *edge* to that parameter's control
/// flow graph:
///
/// - the edge's **source** is the location the parameter must currently be in
///   for the summand to fire (the closed value `c` of a guard conjunct
///   `d == c`), or every location when the guard does not constrain `d`, and
/// - the edge's **target** is the location the parameter moves to once the
///   summand fires (the closed value it is assigned), or the same location as
///   `source` (a self-loop) when the summand leaves `d` unchanged, or unknown
///   when the summand assigns it a non-constant expression (only possible for
///   a summand excluded by [`ControlFlowGraph::new`]'s `live` predicate — see
///   [`CfgTarget::Unknown`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CfgEdge<V> {
    /// Position into [`ControlFlowGraph::control_flow_parameters`] identifying
    /// which control flow parameter this edge belongs to.
    pub position: usize,

    /// The location this edge departs from, i.e. the value the control flow
    /// parameter is required to have for the summand to fire. `None` when the
    /// summand's guard does not constrain it, so the edge departs from every
    /// location.
    pub source: Option<V>,

    /// The location this edge arrives at, i.e. what happens to the control
    /// flow parameter once the summand fires. See [`CfgTarget`].
    pub target: CfgTarget<V>,
}

/// The location a [`CfgEdge`] arrives at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CfgTarget<V> {
    /// The summand leaves the parameter unchanged, so the edge arrives back
    /// at `source` (a genuine self-loop).
    Unchanged,

    /// The summand assigns the parameter this closed value.
    Constant(V),

    /// The summand assigns the parameter a non-constant expression, so its
    /// value after firing is unknown: neither pinned to `source` nor to any
    /// other single fixed location. This can only occur for a summand
    /// excluded by [`ControlFlowGraph::new`]'s `live` predicate: for any live
    /// summand, `is_control_flow_parameter` already disqualifies the
    /// parameter entirely whenever a live summand writes it a non-constant
    /// value, so no edge for that parameter is ever built for any summand
    /// once that happens.
    Unknown,
}

/// The outcome of a control flow graph analysis.
///
/// A parameter `d` is a *control flow parameter* (CFP) when, in every summand,
/// it behaves like a program counter rather than a data variable:
///
/// - whenever a summand changes `d`, it assigns it a closed (constant) value
///   (whether or not that same summand also constrains `d`'s source value), and
/// - whenever a summand reads `d` to decide its guard, it does so through a
///   conjunct `d == c` with `c` a closed value (its *source* location), and
/// - a summand that does not constrain `d` leaves it unchanged.
///
/// Under these conditions the value of a CFP in any reachable state is one of a
/// statically known set of locations, and every summand's effect on it is
/// exactly one edge (see [`CfgEdge`]) between two such locations: the graph
/// `(locations, edges)` is the control flow graph of `d`.
///
/// The generic `V` is the representation of the values of the parameters.
pub struct ControlFlowGraph<V> {
    /// The indices into `parameters` identified as control flow parameters.
    control_flow_parameters: Vec<usize>,

    /// `edges[summand]` are the edges that summand contributes.
    edges: Vec<Vec<CfgEdge<V>>>,
}

impl<V> ControlFlowGraph<V> {
    /// Runs the control flow graph analysis over `summands`, whose guards and
    /// assignments are expressed in terms of `parameters`.
    ///
    /// `live` marks which summands actually participate in the reachable
    /// automaton: only a live summand can disqualify a parameter from being a
    /// control flow parameter (see `is_control_flow_parameter`), so that a
    /// summand belonging to an equation no summand ever transitions into cannot
    /// spuriously defeat the analysis merely by existing. A caller with no
    /// notion of dead code (every summand is live) can pass `|_| true`.
    ///
    /// `context` rewrites a term to normal form before it is tested for
    /// closedness. `intern` interns a normalised location into the caller's own
    /// representation `V`, so an edge's `source`/`target` can be compared
    /// directly in that representation.
    pub fn new<S: CfgSummand>(
        parameters: &[DataVariable],
        summands: &[S],
        live: impl Fn(&S) -> bool,
        context: &LearnSuccessorsContext,
        mut intern: impl FnMut(&DataExpressionRef) -> V,
    ) -> Self {
        // Analyse every summand once, extracting the source and target values.
        let analyses: Vec<SummandAnalysis> = summands
            .iter()
            .map(|summand| analyse_summand(summand, parameters, context))
            .collect();

        // A parameter is a control flow parameter iff every live summand treats
        // it as such, and at least one live summand actually constrains it.
        let control_flow_parameters: Vec<usize> = (0..parameters.len())
            .filter(|&j| is_control_flow_parameter(j, &analyses, summands, &live))
            .collect();

        // Build every summand's edges, interning their source/target locations
        // into the caller's own representation `V`.
        let mut edges: Vec<Vec<CfgEdge<V>>> = Vec::with_capacity(analyses.len());
        for analysis in &analyses {
            let mut summand_edges = Vec::new();
            for (position, &j) in control_flow_parameters.iter().enumerate() {
                let source = analysis.source.get(&j).map(|term| intern_term(&mut intern, term));
                let target = match analysis.target.get(&j) {
                    None => CfgTarget::Unchanged,
                    Some(None) => CfgTarget::Unknown,
                    Some(Some(term)) => CfgTarget::Constant(intern_term(&mut intern, term)),
                };

                if source.is_some() || matches!(target, CfgTarget::Constant(_)) {
                    summand_edges.push(CfgEdge {
                        position,
                        source,
                        target,
                    });
                }
            }
            edges.push(summand_edges);
        }

        let result = Self {
            control_flow_parameters,
            edges,
        };

        // Describe the identified control flow graph: the control flow
        // parameters, per parameter the distinct source and target locations
        // (the locations it can be in), and per summand the source/target
        // locations of the edge it contributes. `log::debug!` only evaluates
        // its arguments (and hence `CfgDisplay::fmt`, which does the actual
        // work of walking `analyses`) when the debug level is enabled.
        log::debug!(
            "{}",
            CfgDisplay {
                graph: &result,
                parameters,
                analyses: &analyses,
            }
        );

        result
    }

    /// The indices into the caller's parameter vector identified as control
    /// flow parameters.
    pub fn control_flow_parameters(&self) -> &[usize] {
        &self.control_flow_parameters
    }

    /// The edges summand `index` contributes to the control flow graph: one
    /// per control flow parameter it constrains and/or changes, in the
    /// representation `V` chosen by [`ControlFlowGraph::new`]'s `intern`.
    pub fn edges(&self, index: usize) -> &[CfgEdge<V>] {
        &self.edges[index]
    }
}

/// Displays a [`ControlFlowGraph`] together with the [`SummandAnalysis`] it was
/// built from: the control flow parameters, the distinct source and target
/// locations of each, and the edge each summand contributes.
///
/// The underlying `analyses` are not retained by [`ControlFlowGraph`] itself
/// (only their interned edges are), so this borrows them separately; it exists
/// purely to give [`ControlFlowGraph::new`]'s debug logging a `Display` impl to
/// format against instead of assembling the description into intermediate
/// `String`s by hand.
struct CfgDisplay<'a, V> {
    /// The graph being described; only its [`ControlFlowGraph::control_flow_parameters`] are used.
    graph: &'a ControlFlowGraph<V>,

    /// The process parameters `graph` was built over, indexed by
    /// [`ControlFlowGraph::control_flow_parameters`], used to print their names.
    parameters: &'a [DataVariable],

    /// The per-summand analysis `graph` was built from.
    analyses: &'a [SummandAnalysis],
}

impl<V> fmt::Display for CfgDisplay<'_, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let control_flow_parameters = self.graph.control_flow_parameters();

        writeln!(
            f,
            "control flow parameters: {}",
            control_flow_parameters
                .iter()
                .format_with(", ", |&j, f| f(&self.parameters[j].name()))
        )?;

        // Per parameter, the distinct source and target locations it is seen
        // at across all summands (the locations it can be in).
        for &j in control_flow_parameters {
            let sources = self
                .analyses
                .iter()
                .filter_map(|analysis| analysis.source.get(&j))
                .sorted_by_cached_key(ToString::to_string)
                .dedup();
            let targets = self
                .analyses
                .iter()
                .filter_map(|analysis| analysis.target.get(&j).and_then(Option::as_ref))
                .sorted_by_cached_key(ToString::to_string)
                .dedup();

            writeln!(
                f,
                "control flow graph for {}: source locations [{}], target locations [{}]",
                self.parameters[j].name(),
                sources.format(", "),
                targets.format(", "),
            )?;
        }

        // Per summand, the source/target locations of the edge it contributes
        // for each control flow parameter it constrains and/or changes.
        for (index, analysis) in self.analyses.iter().enumerate() {
            let mut edges = control_flow_parameters
                .iter()
                .filter_map(|&j| {
                    let source = analysis.source.get(&j);
                    let target = analysis.target.get(&j).and_then(Option::as_ref);
                    (source.is_some() || target.is_some()).then_some((j, source, target))
                })
                .peekable();

            if edges.peek().is_some() {
                writeln!(
                    f,
                    "summand {index} edges: {}",
                    edges.format_with(", ", |(j, source, target), f| {
                        f(&self.parameters[j].name())?;
                        f(&": ")?;
                        match source {
                            Some(value) => f(value)?,
                            None => f(&"*")?,
                        }
                        f(&" -> ")?;
                        match target {
                            Some(value) => f(value)?,
                            None => f(&"*")?,
                        }
                        Ok(())
                    })
                )?;
            }
        }

        Ok(())
    }
}

/// Interns `term`, normalised to a [`DataExpressionRef`], through `intern`.
fn intern_term<V>(intern: &mut impl FnMut(&DataExpressionRef) -> V, term: &ATerm) -> V {
    let value: DataExpressionRef<'_> = term.copy().into();
    intern(&value)
}

/// The result of analysing a single summand for the control flow graph.
struct SummandAnalysis {
    /// Maps a parameter index to its required source location (closed under
    /// `context`) when the summand's guard contains a conjunct `d == c`.
    source: HashMap<usize, ATerm>,

    /// Maps each parameter index changed by the summand to the target
    /// location of the edge it contributes for that parameter: `Some(c)` when
    /// it is assigned a constant value `c` (closed under `context`), `None`
    /// when it is assigned a non-constant expression. A parameter absent from
    /// this map is left unchanged by the summand.
    target: HashMap<usize, Option<ATerm>>,
}

/// Analyses a single summand, extracting its source locations and target
/// locations with respect to `parameters`.
fn analyse_summand<S: CfgSummand>(
    summand: &S,
    parameters: &[DataVariable],
    context: &LearnSuccessorsContext,
) -> SummandAnalysis {
    // Collect the top-level conjuncts of the guard and look for `d == c`
    // constraints on the parameters.
    let mut conjuncts = Vec::new();
    collect_conjuncts(summand.condition(), &mut conjuncts);

    let mut source = HashMap::new();
    for conjunct in &conjuncts {
        if let Some((index, value)) = as_parameter_equality(conjunct, parameters, context) {
            // A parameter cannot consistently be constrained to two different
            // closed values in the same (satisfiable) guard, so the first wins.
            source.entry(index).or_insert(value);
        }
    }

    // `write_assignments` contains only non-identity assignments, so every
    // parameter that appears here is genuinely changed by the summand.
    let mut target = HashMap::new();
    for assignment in summand.write_assignments().iter() {
        let lhs_arg = assignment.arg(0);
        let rhs_arg = assignment.arg(1);
        if lhs_arg.copy() == rhs_arg.copy() {
            continue;
        }

        let lhs = DataVariable::from(lhs_arg.protect());
        if let Some(index) = parameters.iter().position(|param| *param == lhs) {
            let rhs = rhs_arg.protect();
            target.insert(index, closed_value(context, &rhs));
        }
    }

    SummandAnalysis { source, target }
}

/// Returns whether parameter `j` is a control flow parameter across all `live`
/// `analyses`, i.e. every live summand either leaves it unchanged or changes it
/// only to a constant target location, and at least one live summand
/// constrains it to a source location.
///
/// A summand skipped by `live` is skipped entirely: its write behaviour cannot
/// disqualify `j`, and its source constraint (if any) does not count towards
/// `j` being constrained anywhere. This keeps dead code (a summand belonging
/// to an equation no summand ever transitions into) from spuriously defeating
/// the analysis merely by existing.
fn is_control_flow_parameter<S>(
    j: usize,
    analyses: &[SummandAnalysis],
    summands: &[S],
    live: impl Fn(&S) -> bool,
) -> bool {
    let mut constrained_somewhere = false;

    for (analysis, summand) in analyses.iter().zip(summands) {
        if !live(summand) {
            continue;
        }

        constrained_somewhere |= analysis.source.contains_key(&j);

        // The summand changes the parameter. For it to remain a control flow
        // parameter the target location must be a constant, regardless of
        // whether this summand also pins down the source location it
        // transitions from.
        if let Some(target) = analysis.target.get(&j)
            && target.is_none()
        {
            return false;
        }
    }

    constrained_somewhere
}

/// Collects the top-level conjuncts of `expr`, flattening nested `&&`.
fn collect_conjuncts(expr: &DataExpression, out: &mut Vec<DataExpression>) {
    if is_application(expr) && expr.data_function_symbol().name() == "&&" {
        for argument in expr.data_arguments() {
            collect_conjuncts(&DataExpression::new(argument.protect()), out);
        }
    } else {
        out.push(expr.clone());
    }
}

/// If `expr` is an equality `d == c` (in either order) between a parameter `d`
/// and a closed data expression `c`, returns the parameter index and the
/// source location `c` (rewritten to normal form under `context`, see
/// [`closed_value`]).
fn as_parameter_equality(
    expr: &DataExpression,
    parameters: &[DataVariable],
    context: &LearnSuccessorsContext,
) -> Option<(usize, ATerm)> {
    if !is_application(expr) || expr.data_function_symbol().name() != "==" {
        return None;
    }

    let arguments: Vec<ATerm> = expr.data_arguments().map(|argument| argument.protect()).collect();
    if arguments.len() != 2 {
        return None;
    }

    match_parameter_constant(&arguments[0], &arguments[1], parameters, context)
        .or_else(|| match_parameter_constant(&arguments[1], &arguments[0], parameters, context))
}

/// Returns the parameter index and source location when `variable` is a
/// parameter and `constant` is closed under `context` (see [`closed_value`]).
fn match_parameter_constant(
    variable: &ATerm,
    constant: &ATerm,
    parameters: &[DataVariable],
    context: &LearnSuccessorsContext,
) -> Option<(usize, ATerm)> {
    if !is_variable(variable) {
        return None;
    }

    let variable = DataVariable::from(variable.clone());
    let index = parameters.iter().position(|param| *param == variable)?;

    closed_value(context, constant).map(|value| (index, value))
}

/// Rewrites `term` under `context`'s substitution and returns the result when
/// it is closed (variable-free), i.e. a genuine location rather than an
/// expression that still depends on other parameters. Returns `None`
/// otherwise.
///
/// A caller that seeds `context` with a substitution (as `merc_lps` does for
/// the `@rewr_var` placeholders `replace_constants_by_variables` introduces)
/// gets those resolved to the constants they stand for; a caller whose context
/// carries no substitution (as `merc_pbes` does — SRF conversion has no
/// equivalent preprocessing step) only gets `term` normalised.
///
/// Source and target locations are both obtained through this same function so
/// that they end up in the same normal form and can be compared directly.
fn closed_value(context: &LearnSuccessorsContext, term: &ATerm) -> Option<ATerm> {
    let expr: DataExpressionRef<'_> = term.copy().into();
    let rewritten = context.rewrite_under_sigma(&expr);
    let rewritten_ref: DataExpressionRef<'_> = rewritten.copy().into();
    if free_variables_data_expression(&rewritten_ref).is_empty() {
        Some(rewritten)
    } else {
        None
    }
}
