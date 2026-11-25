use std::fmt;

use mcrl2::ATerm;
use mcrl2::ATermList;
use mcrl2::CfgSummand;
use mcrl2::ControlFlowGraph;
use mcrl2::DataExpression;
use mcrl2::SrfPbes;
use merc_explore::LPS;
use merc_utilities::MercError;
use merc_utilities::ShardedCounter;

use crate::explore_common::PbesVertex;
use crate::explore_srf::PbesSrfContext;
use crate::explore_srf::PbesSrfLps;
use crate::explore_srf::PbesSrfSummand;

/// Lets [`ControlFlowGraph`] read an SRF summand's condition and (non-identity)
/// write assignments without depending on `merc_pbes` directly.
///
/// This is the same analysis `merc_lps` layers over LPS summands: an SRF
/// summand assigns the unified parameter vector of its target equation, which
/// has the same positional shape as an LPS summand assigning process
/// parameters directly.
impl CfgSummand for PbesSrfSummand {
    fn condition(&self) -> &DataExpression {
        PbesSrfSummand::condition(self)
    }

    fn write_assignments(&self) -> &ATermList<ATerm> {
        PbesSrfSummand::write_assignments(self)
    }
}

/// A PBES-in-SRF view that selectively enables summands based on a
/// [`ControlFlowGraph`], on top of the equation-index pruning that
/// [`PbesSrfLps::prepare`] always applies.
///
/// The [`LPS::prepare`] implementation drops summands whose source-value
/// condition cannot hold in the current state, mirroring how
/// `merc_lps::CfgLinearProcessSpecification` prunes an LPS's summands.
///
/// Pruning never changes the explored parity game: a dropped summand has a
/// condition that is false in the current state and would have produced no
/// successor equations anyway.
pub struct CfgPbesSrfLps {
    /// The underlying SRF view that performs the actual enumeration.
    inner: PbesSrfLps,

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

/// Per-summand selection counters for a [`CfgPbesSrfLps`].
#[derive(Default)]
struct SummandCfgCounters {
    /// Number of states in which this summand passed the control flow guard and
    /// was explored.
    selected: ShardedCounter,
    /// Number of states in which this summand was pruned because its control
    /// flow guard could not hold.
    pruned: ShardedCounter,
}

impl CfgPbesSrfLps {
    /// Builds the SRF view and runs the control flow graph analysis on top.
    pub fn new(srf: SrfPbes) -> Result<Self, MercError> {
        let inner = PbesSrfLps::new(srf)?;

        let context = inner.analysis_context();
        let parameters = inner.parameters();
        // A summand belonging to an equation that is itself unreachable from
        // the initial equation must not be allowed to disqualify a genuine
        // control flow parameter merely by existing in the unified PBES.
        // The state layout is `[equation_index, params...]`.
        let initial_state: Vec<_> = inner.initial_state()[1..]
            .iter()
            .map(|&value| inner.value(value))
            .collect();
        let graph = ControlFlowGraph::new(
            &parameters,
            &initial_state,
            inner.summands(),
            |summand| inner.is_equation_reachable(summand.equation_index()),
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

    /// Returns the data parameter indices identified as control flow
    /// parameters (see [`ControlFlowGraph`]).
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
    /// Index of the summand in the SRF view's flat summand list.
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
/// [`CfgPbesSrfLps`].
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

impl LPS for CfgPbesSrfLps {
    type Value = usize;
    type Label = ();
    type StateInfo = PbesVertex;
    // A PBES has no notion of an action; do not encode a trailing "action" dimension for it.
    const HAS_LABELS: bool = false;
    type Summand = PbesSrfSummand;

    fn initial_state(&self) -> Vec<usize> {
        self.inner.initial_state()
    }

    fn summands(&self) -> &[Self::Summand] {
        self.inner.summands()
    }

    fn create_context(&self) -> PbesSrfContext {
        self.inner.create_context()
    }

    fn prepare<'a>(
        &'a self,
        context: &mut PbesSrfContext,
        state: &'a [Self::Value],
    ) -> impl Iterator<Item = usize> + 'a {
        // The inner SRF view already restricts candidates to the current
        // equation's summands (state[0]); further filter those by the
        // source-location constraints on the control flow parameters.
        let candidates = self.inner.prepare(context, state);

        let control_flow_parameters = self.graph.control_flow_parameters();
        #[cfg(feature = "metrics")]
        let summand_metrics = &self.summand_metrics;

        candidates.filter(move |&index| {
            // A summand is pruned when the current state's value at one of
            // its control flow parameters is in another location than the one
            // its guard requires; a value that is not a location term is never
            // excluded. State layout is `[equation_index, params...]`, hence
            // the `1 +` offset.
            let selected = self.graph.edges(index).iter().all(|edge| {
                edge.source.is_none_or(|source| {
                    self.graph
                        .location(edge.position, &state[1 + control_flow_parameters[edge.position]])
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

    fn state_info(&self, state: &[Self::Value], context: &PbesSrfContext) -> Self::StateInfo {
        self.inner.state_info(state, context)
    }
}
