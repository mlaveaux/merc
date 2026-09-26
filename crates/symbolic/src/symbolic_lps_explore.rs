use std::fmt;
use std::rc::Rc;

use log::debug;

use merc_collections::IndexedSet;
use merc_explore::LPS;
use merc_explore::PermutedLps;
use merc_explore::Summand;
use merc_utilities::MercError;
use oxidd::ManagerRef;
use oxidd::ldd::LDDFunction;
use oxidd::ldd::LDDManagerRef;
use oxidd::ldd::RelationProductMeta;
use oxidd::ldd::Value;
use streaming_iterator::StreamingIterator;

use crate::ReadWritePattern;
use crate::SummandGrouping;
use crate::SymbolicLPS;
use crate::TransitionGroup;
use crate::VariableOrder;
use crate::iter;
use crate::print_read_write_patterns;
use crate::print_transition_groups;
use crate::transition_group_pattern;

/// A symbolic LDD view of any [`merc_explore::LPS`].
///
/// This adapts the explicit-exploration abstraction (an `LPS` exposing summands
/// with read/write positions, a `prepare` step selecting the summands that fire
/// from a state, and per-summand enumeration) into the disjunctive LDD
/// transition relation consumed by [`crate::reachability`]. A single generic
/// implementation therefore drives symbolic exploration for both LPSs and PBESs
/// in SRF form (and anything else implementing `LPS`).
///
/// The wrapped `LPS` interns parameter values in its own space; this adapter
/// keeps a separate per-position interning into dense LDD values so the decision
/// diagrams stay compact regardless of how the `LPS` numbers its values.
///
/// Position `i` of the state vector is stored at level `i` of the diagrams. A variable order is
/// therefore not something this adapter applies: the constructors permute the state vector of the
/// `LPS` itself (see [`PermutedLps`]) up front, and everything below works in a single position
/// space.
pub struct SymbolicLps<L: LPS> {
    /// The wrapped `LPS` definition; the single source of summands and the
    /// `prepare`/`enumerate` machinery. Shared immutably with every group.
    lps: Rc<L>,

    /// The symbolic transition groups, as determined by the [`SummandGrouping`].
    groups: Vec<SymbolicLpsGroup<L>>,

    /// The encoded initial state.
    initial_state: LDDFunction,
}

/// Per-run learning state threaded through [SymbolicLpsGroup::learn_successors].
///
/// Holds the mutable state shared by all groups of one [SymbolicLps], so the
/// groups themselves need no interior mutability.
pub struct SymbolicContext<L: LPS> {
    /// The enumeration backend created by [`LPS::create_context`].
    enumerate: <L::Summand as Summand>::Context,

    /// Per state-vector position interning of `LPS` values into dense LDD values.
    columns: Vec<IndexedSet<L::Value>>,

    /// Interning of the action labels observed during learning into dense LDD
    /// values, shared by every group so a label's index means the same thing
    /// everywhere. Positions correspond to the trailing "action" dimension each
    /// [`SymbolicLpsGroup`] appends to its relation, see
    /// [`TransitionGroup::action_label_index`].
    labels: IndexedSet<L::Label>,
}

impl<L: LPS> SymbolicContext<L> {
    /// Returns the per-position interning of `LPS` values into dense LDD values,
    /// as populated by [`TransitionGroup::learn_successors`] calls made with
    /// this context.
    pub fn columns(&self) -> &[IndexedSet<L::Value>] {
        &self.columns
    }

    /// Returns the interning of the action labels observed while learning with
    /// this context; a label's dense index is its position in the "action"
    /// dimension of every group's relation, see
    /// [`TransitionGroup::action_label_index`].
    pub fn labels(&self) -> &IndexedSet<L::Label> {
        &self.labels
    }
}

/// The choices that shape the symbolic encoding of an [`merc_explore::LPS`], mirroring the `--groups`
/// and `--reorder` options of the mCRL2 symbolic tools.
///
/// Neither option changes the reachable set, only the size of the decision diagrams and the amount of
/// work per exploration step.
#[derive(Clone, Debug, Default)]
pub struct SymbolicLpsOptions {
    /// How the summands are distributed over the transition groups.
    pub grouping: SummandGrouping,

    /// The order in which the state vector positions are stored in the diagram.
    pub order: VariableOrder,
}

impl<L: LPS> SymbolicLps<PermutedLps<L>> {
    /// Wraps `lps` into a symbolic LDD view, encoding its initial state, with one transition group
    /// per summand.
    pub fn new(manager: &LDDManagerRef, lps: L) -> Result<Self, MercError> {
        SymbolicLps::with_options(manager, lps, &SymbolicLpsOptions::default())
    }

    /// Wraps `lps` into a symbolic LDD view, encoding its initial state, distributing its summands
    /// over the transition groups according to `grouping`.
    ///
    /// Joining summands into a single group trades the number of relational products per exploration
    /// step against the size of each transition relation; it does not change the reachable set.
    pub fn with_grouping(manager: &LDDManagerRef, lps: L, grouping: &SummandGrouping) -> Result<Self, MercError> {
        let options = SymbolicLpsOptions {
            grouping: grouping.clone(),
            ..SymbolicLpsOptions::default()
        };
        SymbolicLps::with_options(manager, lps, &options)
    }

    /// Wraps `lps` into a symbolic LDD view, encoding its initial state, with the given grouping of
    /// its summands and order of its state vector positions.
    ///
    /// The order is applied by permuting the state vector of `lps` itself, so that position `i` of
    /// the LPS that is encoded below is stored at level `i` of the diagrams.
    pub fn with_options(manager: &LDDManagerRef, lps: L, options: &SymbolicLpsOptions) -> Result<Self, MercError> {
        let num_positions = lps.initial_state().len();

        let patterns = read_write_patterns(&lps, num_positions)?;
        debug!(
            "Read/write matrix of the summands:\n{}",
            print_read_write_patterns(&patterns)
        );

        // `order[level]` is the state vector position to store at `level` of the diagram, which is
        // realised by permuting the LPS; from here on the positions of `lps` *are* the levels.
        let order = options.order.compute(&patterns, num_positions)?;
        let lps = Rc::new(PermutedLps::new(lps, order)?);

        // The patterns of the permuted summands, i.e. the read/write matrix as the diagrams store it.
        let patterns = read_write_patterns(&*lps, num_positions)?;
        let initial_values = lps.initial_state();

        let mut columns: Vec<IndexedSet<L::Value>> = (0..num_positions).map(|_| IndexedSet::new()).collect();

        // Encode the initial state, interning each value into its column.
        let mut initial_vector: Vec<Value> = Vec::with_capacity(num_positions);
        for (position, value) in initial_values.iter().enumerate() {
            let (index, _) = columns[position].insert(*value);
            initial_vector.push(*index as Value);
        }
        let initial_state = manager.with_manager_shared(|m| LDDFunction::singleton(m, &initial_vector))?;

        // Distribute the summands over the transition groups, and derive the read/write pattern each
        // group has to cover.
        let group_indices = options.grouping.compute(&patterns)?;
        let group_patterns = group_indices
            .iter()
            .map(|indices| transition_group_pattern(&patterns, indices))
            .collect::<Result<Vec<_>, MercError>>()?;

        if !matches!(options.grouping, SummandGrouping::None) || !matches!(options.order, VariableOrder::None) {
            debug!(
                "Read/write matrix of the transition groups (grouping {}):\n{}",
                options.grouping,
                print_transition_groups(&group_indices, &group_patterns)
            );
        }

        let mut groups = Vec::with_capacity(group_indices.len());
        for (indices, pattern) in group_indices.into_iter().zip(group_patterns) {
            // The short-vector encoding requires sorted read/write indices, which the patterns
            // already report in increasing order.
            let read_indices: Vec<Value> = pattern.read_positions().map(|p| p as Value).collect();
            let write_indices: Vec<Value> = pattern.write_positions().map(|p| p as Value).collect();

            let (project_ldd, meta, read_positions, write_positions, relation, domain) =
                manager.with_manager_shared(|m| -> Result<_, MercError> {
                    let project_ldd = LDDFunction::projection_meta(m, &read_indices)?;
                    let RelationProductMeta {
                        meta,
                        read_positions,
                        write_positions,
                    } = LDDFunction::relation_product_meta(m, &read_indices, &write_indices)?;
                    let relation = LDDFunction::empty_set(m)?;
                    let domain = LDDFunction::empty_set(m)?;
                    Ok((project_ldd, meta, read_positions, write_positions, relation, domain))
                })?;

            groups.push(SymbolicLpsGroup {
                lps: Rc::clone(&lps),
                indices,
                read_indices,
                write_indices,
                read_positions,
                write_positions,
                project_ldd,
                meta,
                relation,
                domain,
            });
        }

        Ok(SymbolicLps {
            lps,
            groups,
            initial_state,
        })
    }
}

impl<L: LPS> SymbolicLps<L> {
    /// Returns the wrapped `LPS` definition.
    pub fn lps(&self) -> &L {
        &self.lps
    }

    /// Builds the per-position interning seeded with the initial-state values.
    ///
    /// Re-inserting `initial_state()` into fresh per-column sets reproduces the
    /// exact dense indices used when [Self::with_options] encoded the initial LDD, so the
    /// learned relations stay consistent with it.
    fn initial_columns(&self) -> Vec<IndexedSet<L::Value>> {
        let initial_values = self.lps.initial_state();
        let mut columns: Vec<IndexedSet<L::Value>> = (0..initial_values.len()).map(|_| IndexedSet::new()).collect();
        for (position, value) in initial_values.iter().enumerate() {
            columns[position].insert(*value);
        }
        columns
    }
}

/// Returns the read/write pattern of every summand of `lps` over the `num_positions` positions of its
/// state vector.
fn read_write_patterns<L: LPS>(lps: &L, num_positions: usize) -> Result<Vec<ReadWritePattern>, MercError> {
    lps.summands()
        .iter()
        .enumerate()
        .map(|(index, summand)| {
            // A symbolic transition relation is inherently positional: it stores only the read and
            // write columns, so there is no encoding for a summand whose next states change shape.
            let effect = summand.effect();
            let write_positions = effect.positions().ok_or_else(|| {
                MercError::from(format!(
                    "summand {index} has an opaque state effect, which symbolic exploration cannot encode"
                ))
            })?;

            ReadWritePattern::from_indices(num_positions, summand.read_positions(), write_positions)
                .map_err(|error| MercError::from(format!("summand {index}: {error}")))
        })
        .collect()
}

impl<L: LPS> SymbolicLPS for SymbolicLps<L> {
    type Group = SymbolicLpsGroup<L>;

    fn initial_state(&self) -> &LDDFunction {
        &self.initial_state
    }

    fn transition_groups(&self) -> &[Self::Group] {
        &self.groups
    }

    fn transition_groups_mut(&mut self) -> &mut [Self::Group] {
        &mut self.groups
    }

    fn create_context(&self) -> SymbolicContext<L> {
        SymbolicContext {
            enumerate: self.lps.create_context(),
            columns: self.initial_columns(),
            labels: IndexedSet::new(),
        }
    }
}

/// A group of summands of a [`SymbolicLps`], encoded as a single short-vector LDD
/// transition relation that is learned on the fly during reachability.
pub struct SymbolicLpsGroup<L: LPS> {
    /// The wrapped `LPS`, shared immutably with [SymbolicLps].
    lps: Rc<L>,

    /// Indices of the summands in this group within `lps.summands()`, in increasing order.
    indices: Vec<usize>,

    /// Diagram levels read by the group (sorted), which are the state vector positions it reads.
    read_indices: Vec<Value>,

    /// Diagram levels written by the group (sorted), which are the state vector positions it writes.
    write_indices: Vec<Value>,

    /// Interleaved short-vector positions of `read_indices`.
    read_positions: Vec<usize>,

    /// Interleaved short-vector positions of `write_indices`.
    write_positions: Vec<usize>,

    /// Projection meta selecting `read_indices` from a full state.
    project_ldd: LDDFunction,

    /// Relational-product meta for `read_indices`/`write_indices`.
    meta: LDDFunction,

    /// The learned transition relation `T' -> U'`, grown on the fly.
    relation: LDDFunction,

    /// The domain of [Self::relation]: the projections on `read_indices` whose successors have
    /// already been learned. Only grown when learning is requested with caching, and empty otherwise.
    domain: LDDFunction,
}

impl<L: LPS> TransitionGroup for SymbolicLpsGroup<L> {
    type Context = SymbolicContext<L>;

    fn relation(&self) -> &LDDFunction {
        &self.relation
    }

    fn read_indices(&self) -> &[u32] {
        &self.read_indices
    }

    fn write_indices(&self) -> &[u32] {
        &self.write_indices
    }

    fn action_label_index(&self) -> Option<usize> {
        L::HAS_LABELS.then_some(self.read_indices.len() + self.write_indices.len())
    }

    fn meta(&self) -> &LDDFunction {
        &self.meta
    }

    fn learn_successors(
        &mut self,
        context: &mut SymbolicContext<L>,
        storage: &LDDManagerRef,
        todo: &LDDFunction,
        cached: bool,
    ) -> Result<(), MercError> {
        let projection = todo.project(&self.project_ldd)?;

        // With caching only the projections that have not been learned from before are enumerated;
        // the ones in the domain already contributed all their transitions to the relation.
        let proj = if cached {
            projection.minus(&self.domain)?
        } else {
            projection.clone()
        };

        // Borrow the backend and the interning as disjoint fields so the
        // enumeration callback can grow the interning while the backend call is
        // in progress.
        let SymbolicContext {
            enumerate,
            columns,
            labels,
        } = context;

        // Reusable full-length state buffer.
        let mut full_state = self.lps.initial_state();

        // The trailing dimension of `interleaved` carries the interned action label, one past the
        // read/write positions.
        let action_position = self.read_indices.len() + self.write_indices.len();
        let vector_len = if L::HAS_LABELS {
            action_position + 1
        } else {
            action_position
        };

        // Reusable interleaved short vector for the relation singletons.
        let mut interleaved: Vec<Value> = vec![0; vector_len];

        // Accumulate into a local relation so the enumeration callback does not
        // have to borrow `self`.
        let mut relation = self.relation.clone();

        // Reused buffer for the summands of this group that fire from the current state.
        let mut applicable: Vec<usize> = Vec::with_capacity(self.indices.len());

        let mut states = iter(&proj);
        while let Some(short_state) = states.next() {
            debug_assert_eq!(
                short_state.len(),
                self.read_indices.len(),
                "Projected state must have one value per read index"
            );

            // Overlay the short read values into the full state buffer and the
            // interleaved read positions.
            for (k, &value) in short_state.iter().enumerate() {
                let position = self.read_indices[k] as usize;
                full_state[position] = *columns[position]
                    .get_by_index(value as usize)
                    .expect("read value must already be interned");
                interleaved[self.read_positions[k]] = value;
            }

            // Prepare the backend assignments for this state and collect the summands of this group
            // that may fire from it.
            applicable.clear();
            applicable.extend(
                self.lps
                    .prepare(enumerate, &full_state)
                    .filter(|i| self.indices.binary_search(i).is_ok()),
            );

            // Enumerate the successors of every applicable summand, building the write side of each
            // short transition vector and unioning it into the relation.
            //
            // A summand that does not write one of the group's write positions leaves it untouched
            // (the [`StateEffect::Positions`] contract), so `next_state` carries the source value
            // there, which the group reads by construction of its read positions.
            for &index in &applicable {
                let write_indices = &self.write_indices;
                let write_positions = &self.write_positions;
                let interleaved = &mut interleaved;
                let relation = &mut relation;
                let labels = &mut *labels;

                self.lps.summands()[index].enumerate(enumerate, &full_state, |label, next_state| {
                    for (m, &write_index) in write_indices.iter().enumerate() {
                        let position = write_index as usize;
                        let (value, _) = columns[position].insert(next_state[position]);
                        interleaved[write_positions[m]] = *value as Value;
                    }

                    if L::HAS_LABELS {
                        let (label_index, _) = labels.insert(label.clone());
                        interleaved[action_position] = *label_index as Value;
                    }

                    let cube = storage.with_manager_shared(|m| LDDFunction::singleton(m, interleaved.as_slice()))?;
                    *relation = relation.union(&cube)?;
                    Ok(())
                })?;
            }
        }

        self.relation = relation;
        if cached {
            // Only extend the domain once every projection above was enumerated successfully,
            // otherwise a failed learning call would hide the states it never reached.
            self.domain = self.domain.union(&projection)?;
        }
        Ok(())
    }
}

impl<L: LPS> fmt::Debug for SymbolicLps<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "SymbolicLps:")?;
        for (i, group) in self.groups.iter().enumerate() {
            writeln!(f, "  {i}: {group:?}")?;
        }
        Ok(())
    }
}

impl<L: LPS> fmt::Debug for SymbolicLpsGroup<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "summands {:?} (read {:?}, write {:?})",
            self.indices, self.read_indices, self.write_indices
        )
    }
}

#[cfg(test)]
mod tests {
    use merc_explore::StateEffect;
    use merc_utilities::Timing;

    use crate::ExplorationStrategy;
    use crate::LDD_CACHE_CAPACITY;
    use crate::LDD_NODE_CAPACITY;
    use crate::LddLenCache;
    use crate::ReachabilityOptions;
    use crate::ldd_len;
    use crate::reachability_with_options;

    use super::*;

    /// A grid [`LPS`]: summand `i` increments position `i` while it is below its bound, and one
    /// extra summand increments the first two positions at once. The diagonal summand couples two
    /// positions, so the transition groups are not all over a single position.
    struct GridLps {
        summands: Vec<GridSummand>,
        num_positions: usize,
    }

    /// A guarded increment of [`GridLps`], writing the positions it also reads.
    struct GridSummand {
        /// The positions incremented, which are exactly the ones read and written.
        positions: Vec<usize>,
        /// The guard: every incremented position must be below this bound.
        bound: usize,
    }

    impl GridLps {
        /// The grid over `bounds.len()` positions, with the diagonal summand over the first two.
        fn new(bounds: &[usize]) -> GridLps {
            let mut summands: Vec<GridSummand> = bounds
                .iter()
                .enumerate()
                .map(|(position, &bound)| GridSummand {
                    positions: vec![position],
                    bound,
                })
                .collect();
            summands.push(GridSummand {
                positions: vec![0, 1],
                bound: bounds[0].min(bounds[1]),
            });

            GridLps {
                summands,
                num_positions: bounds.len(),
            }
        }
    }

    impl LPS for GridLps {
        type Value = usize;
        type Label = ();
        type StateInfo = ();
        type Summand = GridSummand;

        fn initial_state(&self) -> Vec<usize> {
            vec![0; self.num_positions]
        }

        fn summands(&self) -> &[GridSummand] {
            &self.summands
        }

        fn create_context(&self) -> Vec<usize> {
            Vec::new()
        }

        fn prepare<'a>(&'a self, _context: &mut Vec<usize>, _state: &'a [usize]) -> impl Iterator<Item = usize> + 'a {
            0..self.summands.len()
        }

        fn state_info(&self, _state: &[usize], _context: &Vec<usize>) {}
    }

    impl Summand for GridSummand {
        type Value = usize;
        type Label = ();
        type Context = Vec<usize>;

        fn enumerate<F>(&self, context: &mut Vec<usize>, state: &[usize], mut report: F) -> Result<(), MercError>
        where
            F: FnMut(&(), &[usize]) -> Result<(), MercError>,
        {
            if self.positions.iter().any(|&position| state[position] >= self.bound) {
                return Ok(());
            }

            context.clear();
            context.extend_from_slice(state);
            for &position in &self.positions {
                context[position] += 1;
            }
            report(&(), context)
        }

        fn read_positions(&self) -> &[usize] {
            &self.positions
        }

        fn effect(&self) -> StateEffect<'_> {
            StateEffect::Positions(&self.positions)
        }
    }

    /// Explores the grid over `bounds` with the given encoding and strategy,
    /// and returns the reachable state count.
    fn explored_count(bounds: &[usize], options: &SymbolicLpsOptions, strategy: ExplorationStrategy) -> usize {
        let storage = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);
        let mut symbolic =
            SymbolicLps::with_options(&storage, GridLps::new(bounds), options).expect("the encoding is valid");

        let mut context = symbolic.create_context();
        let states = reachability_with_options(
            &storage,
            &mut symbolic,
            &mut context,
            &ReachabilityOptions {
                strategy,
                ..ReachabilityOptions::default()
            },
            &Timing::new(),
        )
        .expect("reachability succeeds")
        .states;
        ldd_len(&states, &mut LddLenCache::new())
            .exact()
            .expect("the grids are small") as usize
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Miri is too slow.
    fn test_variable_order_preserves_the_reachable_states() {
        let bounds = [2, 3, 4];

        // Every combination of coordinates below its bound is reachable.
        let expected = bounds.iter().map(|bound| bound + 1).product::<usize>();

        let strategies = [
            ExplorationStrategy::BreadthFirst,
            ExplorationStrategy::Chaining,
            ExplorationStrategy::Fixpoint,
            ExplorationStrategy::FixpointChaining,
            ExplorationStrategy::Saturation,
        ];

        for strategy in strategies {
            assert_eq!(
                explored_count(&bounds, &SymbolicLpsOptions::default(), strategy),
                expected,
                "strategy {strategy:?} changed the reachable states"
            );
        }

        // The order only decides how the state vector is stored, never which states are reachable.
        for order in [vec![0, 1, 2], vec![2, 1, 0], vec![1, 2, 0], vec![0, 2, 1]] {
            for grouping in [SummandGrouping::None, SummandGrouping::Used, SummandGrouping::Simple] {
                let options = SymbolicLpsOptions {
                    grouping,
                    order: VariableOrder::Explicit(order.clone()),
                };
                for strategy in strategies {
                    assert_eq!(
                        explored_count(&bounds, &options, strategy),
                        expected,
                        "variable order {order:?}, strategy {strategy:?} changed the reachable states"
                    );
                }
            }
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // miri does not work with oxidd.
    fn test_variable_order_permutes_the_transition_groups() {
        let storage = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);
        let options = SymbolicLpsOptions {
            grouping: SummandGrouping::None,
            order: VariableOrder::Explicit(vec![2, 1, 0]),
        };
        let symbolic =
            SymbolicLps::with_options(&storage, GridLps::new(&[2, 3, 4]), &options).expect("the encoding is valid");

        // Summand `i` reads and writes position `i`, which the reversed order stores at level
        // `2 - i`; the groups are given as read/write levels alone.
        let levels: Vec<Vec<Value>> = symbolic
            .transition_groups()
            .iter()
            .map(|group| group.read_indices().to_vec())
            .collect();
        assert_eq!(levels, vec![vec![2], vec![1], vec![0], vec![1, 2]]);
    }

    #[test]
    #[cfg_attr(miri, ignore)] // miri does not work with oxidd.
    fn test_invalid_variable_order_is_rejected() {
        let storage = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);
        let options = SymbolicLpsOptions {
            grouping: SummandGrouping::None,
            order: VariableOrder::Explicit(vec![0, 1]),
        };
        assert!(SymbolicLps::with_options(&storage, GridLps::new(&[2, 3, 4]), &options).is_err());
    }

    /// [GridLps] has exactly one reachable deadlock: the corner where every position is at its
    /// bound (every single-position summand and the diagonal summand are then guarded off). This
    /// exercises [`TransitionGroup::learn_successors`] (the real [`SymbolicLpsGroup`] adapter, not
    /// the hand-built groups [`crate::ldd::symbolic_explore`]'s own tests use) together with
    /// [`ReachabilityOptions::detect_deadlocks`], which no other test in this module does.
    #[test]
    #[cfg_attr(miri, ignore)] // Miri is too slow.
    fn test_detect_deadlocks_finds_the_grid_corner() {
        let bounds = [2usize, 3, 4];
        // Every combination of coordinates below its bound is reachable; only the corner where
        // every coordinate equals its bound has no outgoing transition.
        let expected_states = bounds.iter().map(|bound| bound + 1).product::<usize>();

        let strategies = [
            ExplorationStrategy::BreadthFirst,
            ExplorationStrategy::Chaining,
            ExplorationStrategy::Fixpoint,
            ExplorationStrategy::FixpointChaining,
            ExplorationStrategy::Saturation,
        ];

        for grouping in [SummandGrouping::None, SummandGrouping::Used, SummandGrouping::Simple] {
            for cached in [false, true] {
                for strategy in strategies {
                    let storage = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);
                    let options = SymbolicLpsOptions {
                        grouping: grouping.clone(),
                        order: VariableOrder::None,
                    };
                    let mut symbolic = SymbolicLps::with_options(&storage, GridLps::new(&bounds), &options)
                        .expect("the encoding is valid");

                    let mut context = symbolic.create_context();
                    let result = reachability_with_options(
                        &storage,
                        &mut symbolic,
                        &mut context,
                        &ReachabilityOptions {
                            strategy,
                            detect_deadlocks: true,
                            cached,
                        },
                        &Timing::new(),
                    )
                    .expect("reachability succeeds");

                    let num_states = ldd_len(&result.states, &mut LddLenCache::new())
                        .exact()
                        .expect("the grid is small") as usize;
                    assert_eq!(
                        num_states, expected_states,
                        "grouping {grouping:?}, cached {cached}, strategy {strategy:?}: reachable states"
                    );

                    let deadlocks = result.deadlocks.expect("detect_deadlocks was requested");
                    let num_deadlocks = ldd_len(&deadlocks, &mut LddLenCache::new())
                        .exact()
                        .expect("the grid is small") as usize;
                    assert_eq!(
                        num_deadlocks, 1,
                        "grouping {grouping:?}, cached {cached}, strategy {strategy:?}: deadlock count"
                    );
                }
            }
        }
    }
}
