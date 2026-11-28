#![forbid(unsafe_code)]

use log::trace;

use merc_lts::LTS;
use merc_lts::LabelIndex;
use merc_lts::StateIndex;
use merc_utilities::MercError;
use merc_utilities::is_valid_permutation;

/// Returns a topological ordering of the states of the given LTS, ensuring
/// `s` occurs before `t` whenever there is a transition from `s` to `t` that
/// `filter` accepts.
///
/// If `reverse` is true, the ordering is reversed (`t` before `s` instead).
///
/// # Errors
///
/// Returns an error if the transitions accepted by `filter` contain a cycle.
pub(crate) fn sort_topological<F, L>(lts: &L, filter: F, reverse: bool) -> Result<Vec<StateIndex>, MercError>
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
fn sort_topological_visit<F, L>(
    lts: &L,
    filter: &F,
    state_index: StateIndex,
    depth_stack: &mut Vec<StateIndex>,
    marks: &mut [Option<Mark>],
    visited: &mut [bool],
    stack: &mut Vec<StateIndex>,
) -> bool
where
    F: Fn(LabelIndex, StateIndex) -> bool,
    L: LTS,
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
                    .filter(|transition| state != transition.to && filter(transition.label, transition.to))
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
fn is_topologically_sorted<F, P, L>(lts: &L, filter: F, permutation: P, reverse: bool) -> bool
where
    F: Fn(LabelIndex, StateIndex) -> bool,
    P: Fn(StateIndex) -> StateIndex,
    L: LTS,
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
            .filter(|transition| state_index != transition.to && filter(transition.label, transition.to))
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
    use rand::Rng;
    use rand::RngExt;
    use rand::seq::SliceRandom;

    use merc_lts::LTS;
    use merc_lts::LabelIndex;
    use merc_lts::LabelledTransitionSystem;
    use merc_lts::StateIndex;
    use merc_lts::TransitionLabel;
    use merc_utilities::random_test;
    use test_log::test;

    use super::sort_topological;

    /// Builds an LTS whose `a` transitions (label 1) form a DAG over a shuffled order of the
    /// states, with additional backward tau transitions (label 0) and self-loops.
    fn random_dag(rng: &mut impl Rng) -> LabelledTransitionSystem<String> {
        let num_of_states = rng.random_range(1..30);
        let mut order: Vec<usize> = (0..num_of_states).collect();
        order.shuffle(rng);

        let mut transitions = Vec::new();
        for _ in 0..rng.random_range(0..60) {
            let i = rng.random_range(0..num_of_states);
            let j = rng.random_range(0..num_of_states);
            let (low, high) = (order[i.min(j)], order[i.max(j)]);
            transitions.push((low, if i == j { 0 } else { 1 }, high));
            if rng.random_bool(0.3) {
                transitions.push((high, 0, low));
            }
        }

        transitions.sort();
        transitions.dedup();
        let transitions: Vec<_> = transitions
            .into_iter()
            .map(|(from, label, to)| (StateIndex::new(from), LabelIndex::new(label), StateIndex::new(to)))
            .collect();
        LabelledTransitionSystem::new(
            StateIndex::new(0),
            Some(num_of_states),
            || transitions.iter().cloned(),
            vec![String::tau_label(), "a".to_string()],
        )
    }

    /// The result maps every state to its position: a permutation in which every accepted
    /// transition (other than a self-loop) goes forward, or backward when reversed.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_sort_topological() {
        random_test(100, |rng| {
            let lts = random_dag(rng);
            let visible = |label: LabelIndex, _| label.value() == 1;

            for reverse in [false, true] {
                let position = sort_topological(&lts, visible, reverse).unwrap();

                let mut sorted: Vec<usize> = position.iter().map(|p| p.value()).collect();
                sorted.sort();
                assert_eq!(sorted, (0..lts.num_of_states()).collect::<Vec<_>>());

                for s in lts.iter_states() {
                    for t in lts
                        .outgoing_transitions(s)
                        .filter(|t| t.label.value() == 1 && t.to != s)
                    {
                        if reverse {
                            assert!(position[s] > position[t.to], "{s} -a-> {} (reversed)", t.to);
                        } else {
                            assert!(position[s] < position[t.to], "{s} -a-> {}", t.to);
                        }
                    }
                }
            }

            // Without backward tau transitions, accepting every transition still gives an order.
            let has_backward = lts
                .iter_states()
                .any(|s| lts.outgoing_transitions(s).any(|t| t.label.value() == 0 && t.to != s));
            if !has_backward {
                assert!(sort_topological(&lts, |_, _| true, false).is_ok());
            }
        });
    }

    #[test]
    fn test_sort_topological_cycle() {
        // 0 -a-> 1 -a-> 2 -tau-> 0 is a cycle; without the tau transition it is a chain.
        let transitions = [(0, 1, 1), (1, 1, 2), (2, 0, 0), (2, 0, 2)]
            .map(|(from, label, to)| (StateIndex::new(from), LabelIndex::new(label), StateIndex::new(to)));
        let lts = LabelledTransitionSystem::new(
            StateIndex::new(0),
            None,
            || transitions.iter().cloned(),
            vec![String::tau_label(), "a".to_string()],
        );

        assert!(sort_topological(&lts, |_, _| true, false).is_err());

        let position = sort_topological(&lts, |label, _| label.value() == 1, false).unwrap();
        assert_eq!(position, [0, 1, 2].map(StateIndex::new));
        let position = sort_topological(&lts, |label, _| label.value() == 1, true).unwrap();
        assert_eq!(position, [2, 1, 0].map(StateIndex::new));
    }
}
