use std::cmp::min;
use std::fmt;

use crate::utilities::DataPosition;
use ahash::HashMap;
use ahash::HashMapExt;
use log::trace;

use super::MatchAnnouncement;
use super::MatchObligation;

/// A match goal contains a number of obligations (positions that must still be
/// matched) and the corresponding rule that can be announced as being a match.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MatchGoal {
    pub obligations: Vec<MatchObligation>,
    pub announcement: MatchAnnouncement,
}

impl MatchGoal {
    pub fn new(announcement: MatchAnnouncement, obligations: Vec<MatchObligation>) -> Self {
        Self {
            obligations,
            announcement,
        }
    }

    /// Derive the greatest common prefix of the announcement and obligation positions
    /// of a list of match goals.
    pub fn greatest_common_prefix(goals: &[MatchGoal]) -> DataPosition {
        // gcp is empty if there are no match goals
        if goals.is_empty() {
            return DataPosition::empty();
        }

        // Initialise the prefix with the first match goal, can only shrink afterwards
        let first_match_pos = &goals
            .first()
            .expect("There is at least one match goal")
            .announcement
            .position;
        let mut gcp_length = first_match_pos.len();
        let prefix = first_match_pos;

        for g in goals {
            // Compare up to gcp_length or the length of the announcement position
            let compare_length = min(gcp_length, g.announcement.position.len());
            // gcp_length shrinks if they are not the same up to compare_length
            gcp_length = MatchGoal::common_prefix_length(
                &prefix.indices()[0..compare_length],
                &g.announcement.position.indices()[0..compare_length],
            );

            for mo in &g.obligations {
                // Compare up to gcp_length or the length of the match obligation position
                let compare_length = min(gcp_length, mo.position.len());
                // gcp_length shrinks if they are not the same up to compare_length
                gcp_length = MatchGoal::common_prefix_length(
                    &prefix.indices()[0..compare_length],
                    &mo.position.indices()[0..compare_length],
                );
            }
        }

        // The gcp is constructed by taking the first gcp_length indices of the first match goal prefix
        DataPosition::new(&prefix.indices()[0..gcp_length])
    }

    /// Removes the first len position indices of the match goal and obligation positions
    pub fn remove_prefix(mut goals: Vec<MatchGoal>, len: usize) -> Vec<MatchGoal> {
        for goal in &mut goals {
            // update match announcement
            goal.announcement.position = DataPosition::new(&goal.announcement.position.indices()[len..]);
            for mo_index in 0..goal.obligations.len() {
                let shortened = DataPosition::new(
                    &goal
                        .obligations
                        .get(mo_index)
                        .expect("The obligation index is in range")
                        .position
                        .indices()[len..],
                );
                goal.obligations
                    .get_mut(mo_index)
                    .expect("The obligation index is in range")
                    .position = shortened;
            }
        }
        goals
    }

    /// Returns a Vec of partitions of match goals. This partitioning can be
    /// done in multiple ways, but currently match goals are equivalent when
    /// their match announcements have a comparable position.
    pub fn partition(goals: Vec<MatchGoal>) -> Vec<Vec<MatchGoal>> {
        trace!("=== partition(match_goals = [ ===");
        for mg in &goals {
            trace!("\t {mg:?}");
        }
        trace!("]");

        // If one of the goals has a root position all goals are related.
        let partitions = if goals.iter().any(|g| g.announcement.position.is_empty()) {
            vec![goals]
        } else {
            partition_by_position(&goals)
        };

        for goals in &partitions {
            trace!("{{");
            for mg in goals {
                trace!("\t {mg:?}");
            }
            trace!("}}");
        }

        partitions
    }

    // Assumes two slices are of the same length and computes to what length they are equal
    fn common_prefix_length(pos1: &[usize], pos2: &[usize]) -> usize {
        debug_assert_eq!(pos1.len(), pos2.len(), "Given arrays should be of the same length.");

        pos1.iter().zip(pos2).take_while(|(a, b)| a == b).count()
    }

    /// Checks for two positions whether one is a subposition of the other.
    /// For example 2.2.3 and 2 are comparable. 2.2.3 and 1 are not.
    pub fn pos_comparable(p1: &DataPosition, p2: &DataPosition) -> bool {
        let mut index = 0;
        loop {
            if p1.len() == index || p2.len() == index {
                return true;
            }

            if p1.indices()[index] != p2.indices()[index] {
                return false;
            }
            index += 1;
        }
    }
}

/// Partitions goals that all have a non-root announcement position: goals whose
/// positions are comparable (see [`MatchGoal::pos_comparable`]) are grouped.
fn partition_by_position(goals: &[MatchGoal]) -> Vec<Vec<MatchGoal>> {
    // Create a mapping from positions to goals, goals are represented with an index
    // on function parameter goals
    let mut position_to_goals = HashMap::new();
    for (i, g) in goals.iter().enumerate() {
        if !position_to_goals.contains_key(&g.announcement.position) {
            position_to_goals.insert(g.announcement.position.clone(), vec![i]);
        } else {
            let vec = position_to_goals
                .get_mut(&g.announcement.position)
                .expect("The position was inserted above");
            vec.push(i);
        }
    }

    // Sort the positions. They are now in depth first order.
    let mut all_positions: Vec<DataPosition> = position_to_goals.keys().cloned().collect();
    all_positions.sort_unstable();

    // Compute the partitions, finished when all positions are processed
    let mut partitions = vec![];
    let mut p_index = 0; // position index
    while p_index < all_positions.len() {
        // Start the partition with a position
        let p = &all_positions[p_index];
        let mut goals_in_partition = vec![];

        // put the goals with position p in the partition
        let g = position_to_goals.get(p).expect("Every position has at least one goal");
        for i in g {
            goals_in_partition.push(goals[*i].clone());
        }

        // Go over the positions until we find a position that is not comparable to p
        // Because all_positions is sorted we know that once we find a position that is not comparable
        // all subsequent positions will also not be comparable.
        // Moreover, all positions in the partition are related to p. p is the highest in the partition.
        p_index += 1;
        while p_index < all_positions.len() && MatchGoal::pos_comparable(p, &all_positions[p_index]) {
            // Put the goals with position all_positions[p_index] in the partition
            let g = position_to_goals
                .get(&all_positions[p_index])
                .expect("Every position has at least one goal");
            for i in g {
                goals_in_partition.push(goals[*i].clone());
            }
            p_index += 1;
        }

        partitions.push(goals_in_partition);
    }

    partitions
}

impl fmt::Debug for MatchGoal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for obligation in &self.obligations {
            if !first {
                write!(f, ", ")?;
            }
            write!(f, "{obligation:?}")?;
            first = false;
        }

        write!(f, " ↪ {:?}", self.announcement)
    }
}
