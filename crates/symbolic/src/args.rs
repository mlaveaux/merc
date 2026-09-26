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

/// Bytes per inner-node table entry for the LDD manager (`oxidd::ldd::new_manager`'s
/// `inner_node_capacity`), used to convert a user-requested number of gigabytes into the raw
/// entry count the manager actually allocates.
///
/// Derived from oxidd's (`oxidd-manager-index`) node layout for an LDD node,
/// `NodeWithLevel<ET = (), V = u32, ARITY = 2>`: `rc: AtomicU32` (4 bytes) +
/// `level: AtomicLevelNo` (`AtomicU32`, 4 bytes) + `children: UnsafeCell<[Edge; 2]>` (`Edge` is a
/// `#[repr(transparent)]` `u32`, so 2 * 4 = 8 bytes) + `value: u32` (LDD's node value type, 4
/// bytes) = 20 bytes, with no padding since every field is 4-byte aligned. Confirmed empirically:
/// requesting `--oxidd-capacity 1` (the default) makes the manager's out-of-memory abort report an
/// allocation of exactly `20 * (1 << 30)` = 21474836480 bytes before this fix.
#[cfg(feature = "clap")]
const LDD_NODE_ENTRY_BYTES: usize = 20;

/// Bytes per apply-cache entry for the LDD manager (`oxidd::ldd::new_manager`'s
/// `apply_cache_capacity`).
///
/// Derived from oxidd's (`oxidd-cache`) direct-mapped cache entry layout,
/// `Entry<M, LDDOp, ENTRY_CAP = 5>` (LDD's `cache_entry_capacity` is 5): a 1-byte mutex + two
/// 1-byte occupancy counters + a 1-byte `LDDOp` (`#[repr(u8)]`) pack into 4 bytes, followed by
/// `5 * size_of::<Datum<Edge>>()` = `5 * 4` = 20 bytes of operand/value storage, for 24 bytes
/// total. Confirmed empirically: requesting the default cache capacity makes the manager's
/// out-of-memory abort report an allocation of exactly `24 * (1 << 30)` = 25769803776 bytes before
/// this fix.
#[cfg(feature = "clap")]
const LDD_CACHE_ENTRY_BYTES: usize = 24;

/// Bytes per inner-node table entry for the BDD manager (`oxidd::bdd::new_manager`'s
/// `inner_node_capacity`).
///
/// Same layout as [`LDD_NODE_ENTRY_BYTES`], but a BDD node's value type is `()` (BDD nodes carry
/// no extra value, unlike LDD's `u32`), so there is no 4-byte `value` field: 4 + 4 + 8 + 0 = 16
/// bytes.
#[cfg(feature = "clap")]
const BDD_NODE_ENTRY_BYTES: usize = 16;

/// Bytes per apply-cache entry for the BDD manager (`oxidd::bdd::new_manager`'s
/// `apply_cache_capacity`).
///
/// Same layout as [`LDD_CACHE_ENTRY_BYTES`], but BDD's `cache_entry_capacity` is 4, not 5:
/// 4 + 4 * 4 = 20 bytes.
#[cfg(feature = "clap")]
const BDD_CACHE_ENTRY_BYTES: usize = 20;

/// Converts a number of gigabytes (as `1 << 30` bytes) into the number of fixed-size entries of
/// `bytes_per_entry` bytes that fit in that many bytes, so that a manager sized with the result
/// actually allocates approximately `gib` gigabytes of memory for that table, rather than `gib *
/// (1 << 30)` raw entries.
#[cfg(feature = "clap")]
fn gib_to_entries(gib: u32, bytes_per_entry: usize) -> usize {
    (((gib as u64) << 30) / bytes_per_entry as u64) as usize
}

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
        oxidd::bdd::new_manager(
            self.node_capacity(BDD_NODE_ENTRY_BYTES),
            self.cache_capacity(BDD_CACHE_ENTRY_BYTES),
            self.workers,
        )
    }

    /// Initializes an Oxidd LDD manager based on these arguments.
    pub fn init_ldd_manager(&self) -> oxidd::ldd::LDDManagerRef {
        oxidd::ldd::new_manager(
            self.node_capacity(LDD_NODE_ENTRY_BYTES),
            self.cache_capacity(LDD_CACHE_ENTRY_BYTES),
            self.workers,
        )
    }

    /// The configured inner-node capacity, converted from gigabytes to a node count using the
    /// given manager's per-entry byte size.
    fn node_capacity(&self, bytes_per_entry: usize) -> usize {
        gib_to_entries(self.capacity_gib, bytes_per_entry)
    }

    /// The configured apply cache capacity, converted from gigabytes using the given manager's
    /// per-entry byte size, defaulting to the same number of gigabytes as `--oxidd-capacity` when
    /// `--oxidd-cache-capacity` is omitted.
    fn cache_capacity(&self, bytes_per_entry: usize) -> usize {
        let gib = self.cache_capacity_gib.unwrap_or(self.capacity_gib);
        gib_to_entries(gib, bytes_per_entry)
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
