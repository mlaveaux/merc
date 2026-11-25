use std::fmt;

use mcrl2::ATerm;
use mcrl2::ATermList;
use mcrl2::CfgSummand;
use mcrl2::ControlFlowGraph;
use mcrl2::DataExpression;
use mcrl2::LearnSuccessorsContext;
use mcrl2::LinearProcessSpecification;
use merc_explore::LPS;
use merc_explore::Summand;
use merc_utilities::MercError;
use merc_utilities::ShardedCounter;

use crate::explore_explicit::ExplicitContext;
use crate::explore_explicit::ExplicitLinearProcessSpecification;
use crate::explore_explicit::ExplicitSummand;
use crate::explore_explicit::Mcrl2MultiActionLabel;

/// Lets [`ControlFlowGraph`] read an LPS summand's guard and (non-identity)
/// write assignments without depending on `merc_lps` directly.
impl CfgSummand for ExplicitSummand {
    fn condition(&self) -> &DataExpression {
        ExplicitSummand::condition(self)
    }

    fn write_assignments(&self) -> &ATermList<ATerm> {
        ExplicitSummand::write_assignments(self)
    }
}

/// An explicit-state LPS view that selectively enables summands based on a
/// [`ControlFlowGraph`].
///
/// The [`LPS::prepare`] implementation drops summands whose control flow guard
/// cannot hold in the current state, mirroring how a PBES in SRF form only
/// explores the summands belonging to its current equation.
///
/// Pruning never changes the explored transition system: a dropped summand has a
/// guard that is false in the current state and would have produced no
/// transitions anyway.
pub struct CfgLinearProcessSpecification {
    /// The underlying explicit LPS that performs the actual enumeration.
    inner: ExplicitLinearProcessSpecification,

    /// The control flow graph driving the summand selection. Locations are
    /// interned as indices into `inner`'s value mapping, so an edge's
    /// source/target can be compared directly against a state's entries.
    graph: ControlFlowGraph<usize>,

    /// Per-summand selection counters, indexed identically to `graph`'s edges.
    /// Only updated when the `metrics` feature is enabled; otherwise they stay
    /// at zero. Updated through `&self` so the view can be shared across
    /// worker threads during parallel exploration.
    summand_metrics: Vec<SummandCfgCounters>,
}

/// Per-summand selection counters for a [`CfgLinearProcessSpecification`].
#[derive(Default)]
struct SummandCfgCounters {
    /// Number of states in which this summand passed the control flow guard and
    /// was explored.
    selected: ShardedCounter,
    /// Number of states in which this summand was pruned because its control
    /// flow guard could not hold.
    pruned: ShardedCounter,
}

impl CfgLinearProcessSpecification {
    /// Builds the explicit LPS and runs the control flow graph analysis on top.
    pub fn new(lps: LinearProcessSpecification) -> Result<Self, MercError> {
        let inner = ExplicitLinearProcessSpecification::new(lps)?;

        // Certain preprocessing steps (notably `replace_constants_by_variables`)
        // introduce expressions that must be rewritten; `LearnSuccessorsContext`
        // seeds that substitution from `inner.lps()` itself.
        let context = LearnSuccessorsContext::new(inner.lps());
        let parameters = inner.parameters();
        // Every summand of an explicit LPS is a candidate from every state (see
        // `ExplicitLinearProcessSpecification::prepare`), so none of them is dead
        // code the analysis should discount.
        let initial_state: Vec<_> = inner
            .initial_state()
            .into_iter()
            .map(|value| inner.value(value))
            .collect();
        let graph = ControlFlowGraph::new(
            &parameters,
            &initial_state,
            inner.summands(),
            |_| true,
            &context,
            |value| inner.intern_normal_form(&context, value),
        );
        let summand_metrics = (0..inner.summands().len())
            .map(|_| SummandCfgCounters::default())
            .collect();
        Ok(Self {
            inner,
            graph,
            summand_metrics,
        })
    }

    /// Returns the process parameter indices identified as control flow
    /// parameters.
    pub fn control_flow_parameters(&self) -> &[usize] {
        self.graph.control_flow_parameters()
    }

    /// Collects per-summand control flow pruning metrics.
    ///
    /// The returned [`CfgMetrics`] implements [`fmt::Display`] for a
    /// human-readable summary of how often each summand was selected versus
    /// pruned.
    pub fn metrics(&self) -> CfgMetrics {
        let summands = self
            .summand_metrics
            .iter()
            .enumerate()
            .map(|(index, counters)| SummandCfgMetrics {
                index,
                selected: counters.selected.get(),
                pruned: counters.pruned.get(),
            })
            .collect();

        CfgMetrics { summands }
    }
}

/// Control flow pruning metrics for a single summand.
#[derive(Clone, Copy, Debug)]
pub struct SummandCfgMetrics {
    /// Index of the summand in the LPS.
    pub index: usize,
    /// Number of states in which this summand was selected (explored).
    pub selected: u64,
    /// Number of states in which this summand was pruned.
    pub pruned: u64,
}

impl SummandCfgMetrics {
    /// Total number of states for which this summand was evaluated.
    pub fn evaluated(&self) -> u64 {
        self.selected + self.pruned
    }

    /// Fraction of evaluations in which this summand was pruned, in `[0.0, 1.0]`.
    ///
    /// Returns `0.0` when the summand was never evaluated.
    pub fn prune_rate(&self) -> f64 {
        let evaluated = self.evaluated();
        if evaluated == 0 {
            0.0
        } else {
            self.pruned as f64 / evaluated as f64
        }
    }
}

/// Aggregated control flow pruning metrics for every summand of a
/// [`CfgLinearProcessSpecification`].
#[derive(Clone, Debug)]
pub struct CfgMetrics {
    /// Per-summand metrics, ordered by summand index.
    pub summands: Vec<SummandCfgMetrics>,
}

impl CfgMetrics {
    /// Total number of summand selections across all summands.
    pub fn total_selected(&self) -> u64 {
        self.summands.iter().map(|s| s.selected).sum()
    }

    /// Total number of summand prunings across all summands.
    pub fn total_pruned(&self) -> u64 {
        self.summands.iter().map(|s| s.pruned).sum()
    }

    /// Fraction of evaluations pruned across all summands.
    pub fn prune_rate(&self) -> f64 {
        let pruned = self.total_pruned();
        let evaluated = pruned + self.total_selected();
        if evaluated == 0 {
            0.0
        } else {
            pruned as f64 / evaluated as f64
        }
    }
}

impl fmt::Display for CfgMetrics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if cfg!(not(feature = "metrics")) {
            return write!(f, "enable the 'metrics' feature to see control flow metrics");
        }

        writeln!(f, "summand control flow metrics:")?;
        writeln!(
            f,
            "  {:>7}  {:>10}  {:>10}  {:>8}",
            "summand", "selected", "pruned", "prune%"
        )?;
        for s in &self.summands {
            writeln!(
                f,
                "  {:>7}  {:>10}  {:>10}  {:>7.1}%",
                s.index,
                s.selected,
                s.pruned,
                s.prune_rate() * 100.0
            )?;
        }
        write!(
            f,
            "  total: {} selected, {} pruned ({:.1}% prune rate)",
            self.total_selected(),
            self.total_pruned(),
            self.prune_rate() * 100.0
        )
    }
}

impl LPS for CfgLinearProcessSpecification {
    type Value = usize;
    type Label = Mcrl2MultiActionLabel;
    type StateInfo = ();
    type Summand = ExplicitSummand;

    fn initial_state(&self) -> Vec<usize> {
        self.inner.initial_state()
    }

    fn summands(&self) -> &[Self::Summand] {
        self.inner.summands()
    }

    fn create_context(&self) -> ExplicitContext {
        self.inner.create_context()
    }

    fn prepare<'a>(
        &'a self,
        context: &mut ExplicitContext,
        state: &'a [Self::Value],
    ) -> impl Iterator<Item = usize> + 'a {
        // Stage the per-state substitution in the enumeration backend; the
        // unfiltered summand list returned by the explicit LPS is discarded in
        // favour of the control-flow-filtered one below.
        let _staged = self.inner.prepare(context, state);

        // The returned iterator borrows `state` directly and reads the control
        // flow values on demand, avoiding a per-state allocation.
        let control_flow_parameters = self.graph.control_flow_parameters();
        #[cfg(feature = "metrics")]
        let summand_metrics = &self.summand_metrics;

        (0..self.inner.summands().len()).filter(move |&index| {
            // A summand is pruned when the current state's value at one of
            // its control flow parameters is in another location than the one
            // its guard requires; a value that is not a location term is never
            // excluded.
            let selected = self.graph.edges(index).iter().all(|edge| {
                edge.source.is_none_or(|source| {
                    self.graph
                        .location(edge.position, &state[control_flow_parameters[edge.position]])
                        .is_none_or(|location| location == source)
                })
            });

            #[cfg(feature = "metrics")]
            {
                let counters = &summand_metrics[index];
                if selected {
                    counters.selected.increment();
                } else {
                    counters.pruned.increment();
                }
            }

            selected
        })
    }

    fn state_info(&self, _state: &[Self::Value], _context: &<Self::Summand as Summand>::Context) -> Self::StateInfo {}
}
