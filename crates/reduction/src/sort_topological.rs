#![forbid(unsafe_code)]

use log::trace;

use merc_lts::LTS;
use merc_lts::LabelIndex;
use merc_lts::StateIndex;
use merc_utilities::MercError;
use merc_utilities::is_valid_permutation;

/// Returns a topological ordering of the states of the given LTS.
///
/// An error is returned if the LTS contains a cycle.
///     - filter: Only transitions satisfying the filter are considered part of the graph.
///     - reverse: If true, the topological ordering is reversed, i.e. successors before the incoming state.
pub fn sort_topological<F, L>(lts: &L, filter: F, reverse: bool) -> Result<Vec<StateIndex>, MercError>
where
    F: Fn(LabelIndex, StateIndex) -> bool,
    L: LTS,
{
    // The resulting order of states.
    let mut stack = Vec::new();

    let mut visited = vec![false; lts.num_of_states()];
    let mut depth_stack = Vec::new();
    let mut marks = vec![None; lts.num_of_states()];

    for state_index in lts.iter_states() {
        if marks[state_index].is_none()
            && !sort_topological_visit(
                lts,
                &filter,
                state_index,
                &mut depth_stack,
                &mut marks,
                &mut visited,
                &mut stack,
            )
        {
            trace!("There is a cycle from state {state_index} on path {stack:?}");
            return Err("Labelled transition system contains a cycle".into());
        }
    }

    if !reverse {
        stack.reverse();
    }
    trace!("Topological order: {stack:?}");

    // Turn the stack into a permutation.
    let mut reorder = vec![StateIndex::new(0); lts.num_of_states()];
    for (i, &state_index) in stack.iter().enumerate() {
        reorder[state_index] = StateIndex::new(i);
    }

    debug_assert!(
        is_topologically_sorted(lts, filter, |i| reorder[i], reverse),
        "The permutation {reorder:?} is not a valid topological ordering for the states of the given LTS"
    );

    Ok(reorder)
}

// The mark of a state in the depth first search.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mark {
    Temporary,
    Permanent,
}

/// Visits the given state in a depth first search.
///
/// Returns false if a cycle is detected.
fn sort_topological_visit<F>(
    lts: &impl LTS,
    filter: &F,
    state_index: StateIndex,
    depth_stack: &mut Vec<StateIndex>,
    marks: &mut [Option<Mark>],
    visited: &mut [bool],
    stack: &mut Vec<StateIndex>,
) -> bool
where
    F: Fn(LabelIndex, StateIndex) -> bool,
{
    // Perform a depth first search.
    depth_stack.push(state_index);

    while let Some(state) = depth_stack.pop() {
        match marks[state] {
            None => {
                marks[state] = Some(Mark::Temporary);
                depth_stack.push(state); // Re-add to stack to mark as permanent later
                for transition in lts
                    .outgoing_transitions(state)
                    .filter(|transition| filter(transition.label, transition.to))
                {
                    // If it was marked temporary, then a cycle is detected.
                    if marks[transition.to] == Some(Mark::Temporary) {
                        return false;
                    }
                    if marks[transition.to].is_none() {
                        depth_stack.push(transition.to);
                    }
                }
            }
            Some(Mark::Temporary) => {
                marks[state] = Some(Mark::Permanent);
                visited[state] = true;
                stack.push(state);
            }
            Some(Mark::Permanent) => {}
        }
    }

    true
}

/// Returns true if the given permutation is a topological ordering of the states of the given LTS.
fn is_topologically_sorted<F, P>(lts: &impl LTS, filter: F, permutation: P, reverse: bool) -> bool
where
    F: Fn(LabelIndex, StateIndex) -> bool,
    P: Fn(StateIndex) -> StateIndex,
{
    debug_assert!(is_valid_permutation(
        |i| permutation(StateIndex::new(i)).value(),
        lts.num_of_states()
    ));

    // Check that each vertex appears before its successors.
    for state_index in lts.iter_states() {
        let state_order = permutation(state_index);
        for transition in lts
            .outgoing_transitions(state_index)
            .filter(|transition| filter(transition.label, transition.to))
        {
            if reverse {
                if state_order <= permutation(transition.to) {
                    return false;
                }
            } else if state_order >= permutation(transition.to) {
                return false;
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {

    use merc_io::DumpFiles;
    use merc_lts::LabelledTransitionSystem;
    use merc_lts::random_lts;
    use merc_lts::write_aut;
    use merc_utilities::random_test;
    use rand::seq::SliceRandom;
    use test_log::test;

    use super::*;

    #[test]
    fn test_random_sort_topological_with_cycles() {
        random_test(100, |rng| {
            let lts = random_lts(rng, 10, 3, 2);
            if let Ok(order) = sort_topological(&lts, |_, _| true, false) {
                assert!(is_topologically_sorted(&lts, |_, _| true, |i| order[i], false))
            }
        });
    }

    #[test]
    fn test_random_reorder_states() {
        random_test(100, |rng| {
            let mut files = DumpFiles::new("test_random_reorder_states");

            let lts = random_lts(rng, 10, 3, 2);
            files.dump("input.aut", |f| write_aut(f, &lts)).unwrap();

            // Generate a random permutation.
            let mut rng = rand::rng();
            let order: Vec<StateIndex> = {
                let mut order: Vec<StateIndex> = (0..lts.num_of_states()).map(StateIndex::new).collect();
                order.shuffle(&mut rng);
                order
            };

            let new_lts = LabelledTransitionSystem::new_from_permutation(lts.clone(), |i| order[i]);
            files.dump("reordered.aut", |f| write_aut(f, &new_lts)).unwrap();

            assert_eq!(new_lts.num_of_labels(), lts.num_of_labels());
        });
    }
}
