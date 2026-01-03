#![forbid(unsafe_code)]

use log::debug;
use log::trace;
use merc_io::LargeFormatter;
use merc_lts::LTS;
use merc_lts::LabelIndex;
use merc_lts::StateIndex;

use crate::BlockIndex;
use crate::IndexedPartition;
use crate::Partition;
use crate::sort_topological;

/// Computes the strongly connected tau component partitioning of the given LTS.
pub fn tau_scc_decomposition(lts: &impl LTS) -> IndexedPartition {
    scc_decomposition(lts, &|_, label_index, _| lts.is_hidden_label(label_index))
}

/// Computes the strongly connected component partitioning of the given LTS.
pub fn scc_decomposition<F>(lts: &impl LTS, filter: &F) -> IndexedPartition
where
    F: Fn(StateIndex, LabelIndex, StateIndex) -> bool,
{
    let mut partition = IndexedPartition::new(lts.num_of_states());

    // The stack for the depth first search.
    let mut stack = Vec::new();

    // Keep track of already visited states.
    let mut state_info: Vec<Option<StateInfo>> = vec![None; lts.num_of_states()];

    let mut smallest_index = 0;
    let mut next_block_number = BlockIndex::new(0);

    // The outer depth first search used to traverse all the states.
    for state_index in lts.iter_states() {
        if state_info[state_index].is_none() {
            trace!("State {state_index}");

            strongly_connect(
                state_index,
                lts,
                filter,
                &mut partition,
                &mut smallest_index,
                &mut next_block_number,
                &mut stack,
                &mut state_info,
            )
        }
    }

    trace!("SCC partition {partition}");
    debug!(
        "Found {} strongly connected components",
        LargeFormatter(partition.num_of_blocks())
    );
    partition
}

#[derive(Clone, Debug)]
struct StateInfo {
    /// A unique index for every state.
    index: usize,

    /// Keeps track of the lowest state that can be reached on the stack.
    lowlink: usize,

    /// Keeps track of whether this state is on the stack.
    on_stack: bool,
}

/// Tarjan's strongly connected components algorithm.
///
/// The `filter` can be used to determine which (from, label, to) edges should
/// to be connected.
///
/// The `smallest_index`, `stack` and `indices` are updated in each recursive
/// call to keep track of the current SCC.
#[allow(clippy::too_many_arguments)]
fn strongly_connect<F>(
    state_index: StateIndex,
    lts: &impl LTS,
    filter: &F,
    partition: &mut IndexedPartition,
    smallest_index: &mut usize,
    next_block_number: &mut BlockIndex,
    stack: &mut Vec<StateIndex>,
    state_info: &mut Vec<Option<StateInfo>>,
) where
    F: Fn(StateIndex, LabelIndex, StateIndex) -> bool,
{
    trace!("Visiting state {state_index}");

    state_info[state_index] = Some(StateInfo {
        index: *smallest_index,
        lowlink: *smallest_index,
        on_stack: true,
    });

    *smallest_index += 1;

    // Start a depth first search from the current state.
    stack.push(state_index);

    // Consider successors of the current state.
    for transition in lts.outgoing_transitions(state_index) {
        if filter(state_index, transition.label, transition.to) {
            if let Some(meta) = &mut state_info[transition.to] {
                if meta.on_stack {
                    // Successor w is in stack S and hence in the current SCC
                    // If w is not on stack, then (v, w) is an edge pointing to an SCC already found and must be ignored
                    // v.lowlink := min(v.lowlink, w.lowlink);
                    let w_index = state_info[transition.to]
                        .as_ref()
                        .expect("The state must be visited in the recursive call")
                        .index;
                    let info = state_info[state_index.value()]
                        .as_mut()
                        .expect("This state was added before");
                    info.lowlink = info.lowlink.min(w_index);
                }
            } else {
                // Successor w has not yet been visited; recurse on it
                strongly_connect(
                    transition.to,
                    lts,
                    filter,
                    partition,
                    smallest_index,
                    next_block_number,
                    stack,
                    state_info,
                );

                // v.lowlink := min(v.lowlink, w.lowlink);
                let w_lowlink = state_info[transition.to.value()]
                    .as_ref()
                    .expect("The state must be visited in the recursive call")
                    .lowlink;
                let info = state_info[state_index.value()]
                    .as_mut()
                    .expect("This state was added before");
                info.lowlink = info.lowlink.min(w_lowlink);
            }
        }
    }

    let info = state_info[state_index.value()]
        .as_ref()
        .expect("This state was added before");
    if info.lowlink == info.index {
        // Start a new strongly connected component.
        while let Some(index) = stack.pop() {
            let info = state_info[index.value()].as_mut().expect("This state was on the stack");
            info.on_stack = false;

            trace!("Added state {index} to block {next_block_number}");
            partition.set_block(index, *next_block_number);

            if index == state_index || stack.is_empty() {
                *next_block_number = BlockIndex::new(next_block_number.value() + 1);
                break;
            }
        }
    }
}

/// Returns true iff the labelled transition system has tau-loops.
pub fn has_tau_loop<L>(lts: &L) -> bool
where
    L: LTS,
{
    sort_topological(lts, |label_index, _| lts.is_hidden_label(label_index), false).is_err()
}

#[cfg(test)]
mod tests {
    use merc_io::DumpFiles;
    use merc_lts::LabelIndex;
    use merc_lts::LabelledTransitionSystem;
    use merc_lts::StateIndex;
    use merc_lts::random_lts;
    use merc_lts::write_aut;
    use merc_utilities::random_test;
    use test_log::test;

    use crate::Partition;
    use crate::quotient_lts_naive;

    use super::*;

    /// Returns the reachable states from the given state index.
    fn reachable_states(
        lts: &impl LTS,
        state_index: StateIndex,
        filter: &impl Fn(StateIndex, LabelIndex, StateIndex) -> bool,
    ) -> Vec<usize> {
        let mut stack = vec![state_index];
        let mut visited = vec![false; lts.num_of_states()];

        // Depth first search to find all reachable states.
        while let Some(inner_state_index) = stack.pop() {
            for transition in lts.outgoing_transitions(inner_state_index) {
                if filter(inner_state_index, LabelIndex::new(0), transition.to) && !visited[transition.to.value()] {
                    visited[transition.to.value()] = true;
                    stack.push(transition.to);
                }
            }
        }

        // All the states that were visited are reachable.
        visited
            .into_iter()
            .enumerate()
            .filter_map(|(index, visited)| if visited { Some(index) } else { None })
            .collect()
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_tau_scc_decomposition() {
        random_test(100, |rng| {
            let mut files = DumpFiles::new("test_random_tau_scc_decomposition");

            let lts = random_lts(rng, 10, 3, 3);
            files.dump("input.aut", |f| write_aut(f, &lts)).unwrap();

            let partitioning = tau_scc_decomposition(&lts);
            let reduction = quotient_lts_naive(&lts, &partitioning, true);
            assert!(!has_tau_loop(&reduction), "The SCC decomposition contains tau-loops");

            files
                .dump("tau_scc_decomposition.aut", |f| write_aut(f, &reduction))
                .unwrap();

            // Check that states in a strongly connected component are reachable from each other.
            for state_index in lts.iter_states() {
                let reachable = reachable_states(&lts, state_index, &|_, label, _| lts.is_hidden_label(label));

                // All other states in the same block should be reachable.
                let block = partitioning.block_number(state_index);

                for other_state_index in lts
                    .iter_states()
                    .filter(|index| state_index != *index && partitioning.block_number(*index) == block)
                {
                    assert!(
                        reachable.contains(&other_state_index),
                        "State {state_index} and {other_state_index} should be connected"
                    );
                }
            }

            assert!(
                reduction.num_of_states() == tau_scc_decomposition(&reduction).num_of_blocks(),
                "Applying SCC decomposition again should yield the same number of SCC after second application"
            );
        });
    }

    #[test]
    fn test_cycles() {
        let transitions = [(0, 0, 2), (0, 0, 4), (1, 0, 0), (2, 0, 1), (2, 0, 0)]
            .map(|(from, label, to)| (StateIndex::new(from), LabelIndex::new(label), StateIndex::new(to)));

        let lts = LabelledTransitionSystem::new(
            StateIndex::new(0),
            None,
            || transitions.iter().cloned(),
            vec!["tau".to_string(), "a".to_string()],
        );

        let _ = tau_scc_decomposition(&lts);
    }
}
