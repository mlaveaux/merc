//! Default sizes and shared command-line argument structs used by the tools that build a decision
//! diagram from CLI input (`merc-sym`, `merc-lps`, `merc-pbes`), so that they can be adapted in a
//! single place.

#[cfg(feature = "clap")]
use std::ops::ControlFlow;

#[cfg(feature = "clap")]
use oxidd::ldd::LDDFunction;

#[cfg(feature = "clap")]
use merc_tools::KaHyParArgs;
#[cfg(feature = "clap")]
use merc_utilities::MercError;

#[cfg(feature = "clap")]
use crate::ExplorationStrategy;
#[cfg(feature = "clap")]
use crate::Order;
#[cfg(feature = "clap")]
use crate::VariableOrder;
#[cfg(feature = "clap")]
use crate::parse_order;

/// The number of inner nodes that a BDD manager is initialised with.
pub const BDD_NODE_CAPACITY: usize = 1 << 22;

/// The capacity of the apply cache that a BDD manager is initialised with.
pub const BDD_CACHE_CAPACITY: usize = 1 << 22;

/// The number of inner nodes that an LDD manager is initialised with.
pub const LDD_NODE_CAPACITY: usize = 1 << 22;

/// The capacity of the apply cache that an LDD manager is initialised with.
pub const LDD_CACHE_CAPACITY: usize = 1 << 22;

/// The default capacity, in gigabytes, used for [`OxiddArgs::capacity_gib`].
#[cfg(feature = "clap")]
const DEFAULT_OXIDD_CAPACITY_GIB: u32 = 1;

/// Command-line arguments for initialising an Oxidd decision diagram manager, shared by every tool
/// (`merc-sym`, `merc-lps`, `merc-pbes`) that builds a BDD or LDD manager from CLI input.
#[cfg(feature = "clap")]
#[derive(clap::Args, Debug)]
pub struct OxiddArgs {
    /// Number of worker threads for the Oxidd decision diagram manager.
    #[arg(long = "oxidd-workers", global = true, default_value_t = 1)]
    workers: u32,

    /// Capacity of the manager's inner-node table, in gigabytes (as a power of two, i.e. `1 << 30`
    /// bytes per gigabyte).
    #[arg(long = "oxidd-capacity", global = true, default_value_t = DEFAULT_OXIDD_CAPACITY_GIB)]
    capacity_gib: u32,

    /// Capacity of the manager's apply cache, in gigabytes; defaults to `--oxidd-capacity` when omitted.
    #[arg(long = "oxidd-cache-capacity", global = true)]
    cache_capacity_gib: Option<u32>,
}

#[cfg(feature = "clap")]
impl OxiddArgs {
    /// Initializes an Oxidd BDD manager based on these arguments.
    pub fn init_bdd_manager(&self) -> oxidd::bdd::BDDManagerRef {
        oxidd::bdd::new_manager(self.node_capacity(), self.cache_capacity(), self.workers)
    }

    /// Initializes an Oxidd LDD manager based on these arguments.
    pub fn init_ldd_manager(&self) -> oxidd::ldd::LDDManagerRef {
        oxidd::ldd::new_manager(self.node_capacity(), self.cache_capacity(), self.workers)
    }

    /// The configured inner-node capacity, converted from gigabytes to a node count.
    fn node_capacity(&self) -> usize {
        (self.capacity_gib as usize) << 30
    }

    /// The configured apply cache capacity, converted from gigabytes, defaulting to the node capacity.
    fn cache_capacity(&self) -> usize {
        self.cache_capacity_gib
            .map_or_else(|| self.node_capacity(), |gib| (gib as usize) << 30)
    }
}

/// Command-line arguments selecting how the parameters of the structure being encoded (an LPS's
/// process parameters, or a PBES's equation parameters) are ordered before it is turned into a
/// decision diagram, shared by every tool (`merc-lps`, `merc-pbes`) that exposes `--reorder`.
#[cfg(feature = "clap")]
#[derive(clap::Args, Debug)]
pub struct ReorderArgs {
    /// Reorder the parameters before exploring: 'mince' runs the MINCE algorithm, which requires the
    /// KaHyPar tool, or an explicit order can be given as a whitespace separated string of numbers.
    /// The reachable states are unaffected, only the size of the decision diagrams.
    #[arg(long, default_value_t = Order::None, value_parser = parse_order)]
    reorder: Order,

    #[command(flatten)]
    kahypar: KaHyParArgs,
}

#[cfg(feature = "clap")]
impl ReorderArgs {
    /// Returns the variable order to explore with, resolving the KaHyPar tool when `--reorder` is set.
    pub fn variable_order(&self) -> Result<VariableOrder, MercError> {
        self.reorder.resolve(|| self.kahypar.resolve())
    }
}

/// Command-line argument selecting the strategy used to apply transition groups during LDD-based
/// reachability, shared by every tool (`merc-sym`, `merc-lps`, `merc-pbes`) that performs symbolic
/// reachability via [`crate::reachability_with_options`] or [`crate::reachability_with_callback`].
#[cfg(feature = "clap")]
#[derive(clap::Args, Debug)]
pub struct ExplorationArgs {
    /// Strategy used to apply the transition groups during reachability.
    #[arg(long, short('s'), value_enum, default_value_t = ExplorationStrategy::default())]
    pub strategy: ExplorationStrategy,
}

/// Command-line argument that stops reachability early after a fixed number of iterations, shared
/// by `merc-sym` and `merc-lps`. Not offered by `merc-pbes`: an incomplete parity game cannot be
/// soundly solved, and `--explore-symbolic` keeps the same options as `--solve-symbolic` so the two
/// stay directly comparable (mirroring how the explicit PBES explorer offers no such flag either).
#[cfg(feature = "clap")]
#[derive(clap::Args, Debug)]
pub struct MaxIterationsArgs {
    /// Stop the exploration after this many iterations (rounds for saturation), and report what has
    /// been found until then. The reported states and deadlocks may then be incomplete.
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub max_iterations: Option<u32>,
}

#[cfg(feature = "clap")]
impl MaxIterationsArgs {
    /// Returns an `on_iteration` callback for [`crate::reachability_with_callback`] that stops the
    /// exploration once `--max-iterations` rounds have completed, recording the iteration it
    /// stopped at into `stopped_after` so the caller can report the result as incomplete afterwards
    /// (see [`MaxIterationsArgs::warn_if_stopped`]).
    pub fn on_iteration<'a>(
        &self,
        stopped_after: &'a mut Option<usize>,
    ) -> impl FnMut(usize, &LDDFunction) -> ControlFlow<()> + 'a {
        let max_iterations = self.max_iterations;
        move |iteration, _states| match max_iterations {
            Some(max) if iteration >= max as usize => {
                *stopped_after = Some(iteration);
                ControlFlow::Break(())
            }
            _ => ControlFlow::Continue(()),
        }
    }

    /// Logs a warning if `stopped_after` is `Some`, i.e. `--max-iterations` cut the exploration
    /// short, so the reported states and deadlocks may be incomplete.
    pub fn warn_if_stopped(stopped_after: Option<usize>) {
        if let Some(iteration) = stopped_after {
            log::warn!(
                "Stopped after {iteration} iteration(s) because of --max-iterations, the reported \
                 states and deadlocks may be incomplete"
            );
        }
    }
}
