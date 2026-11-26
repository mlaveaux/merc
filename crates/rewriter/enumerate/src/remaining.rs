#![forbid(unsafe_code)]

use crate::arena_list::ArenaList;
use crate::arena_list::ArenaListHandle;

/// Backing store for every [`RemainingList`] produced during one search.
pub(crate) type RemainingArena = ArenaList<u32>;

/// One branch's still-to-instantiate variables, as indices into
/// [`Enumerator::var_pool`](crate::enumerator::Enumerator): the search's
/// original goal order plus any fresh variables a constructor expansion has
/// appended past it along this specific branch.
///
/// # Details
///
/// A persistent FIFO queue built from two [`ArenaList`]s, in the classic
/// two-stack style: `front` is already in pop order (built once, in reverse,
/// from the caller's variable order, so popping it never needs rebuilding);
/// `back` accumulates constructor-expansion appends most-recently-pushed
/// first, and is only reversed into a fresh `front` once `front` runs dry.
/// Both halves are `Copy` handles into a shared arena, so cloning a list —
/// every sibling branch does — is free, and extending one branch's `back`
/// never disturbs another's.
#[derive(Clone, Copy, Default)]
pub(crate) struct RemainingList {
    front: ArenaListHandle,
    back: ArenaListHandle,
}

impl RemainingList {
    /// The initial list for a whole search: every original variable, in the
    /// order the caller wants them expanded in, none consumed yet.
    pub(crate) fn new<I>(arena: &mut RemainingArena, original: I) -> RemainingList
    where
        I: IntoIterator<Item = u32, IntoIter: DoubleEndedIterator>,
    {
        let front = arena.push_all(ArenaListHandle::default(), original.into_iter().rev());
        RemainingList {
            front,
            back: ArenaListHandle::default(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.front.is_empty() && self.back.is_empty()
    }

    /// Splits off the first still-remaining variable index and the rest of
    /// the list, or `None` if empty.
    pub(crate) fn pop_front(&self, arena: &mut RemainingArena) -> Option<(u32, RemainingList)> {
        if let Some((&value, parent)) = arena.pop(self.front) {
            return Some((
                value,
                RemainingList {
                    front: parent,
                    back: self.back,
                },
            ));
        }

        if self.back.is_empty() {
            return None;
        }

        // `front` ran dry: reverse `back` (newest-appended-first) into a
        // fresh `front` (pop order), then pop from that.
        let mut reversed = Vec::new();
        let mut current = self.back;
        while let Some((&value, parent)) = arena.pop(current) {
            reversed.push(value);
            current = parent;
        }
        let front = arena.push_all(ArenaListHandle::default(), reversed);
        let (&value, parent) = arena.pop(front).expect("just built from a non-empty `back`");

        Some((
            value,
            RemainingList {
                front: parent,
                back: ArenaListHandle::default(),
            },
        ))
    }

    /// Returns a copy of this list with `fresh` appended at the tail.
    pub(crate) fn with_appended<I>(&self, arena: &mut RemainingArena, fresh: I) -> RemainingList
    where
        I: IntoIterator<Item = u32>,
    {
        RemainingList {
            front: self.front,
            back: arena.push_all(self.back, fresh),
        }
    }

    /// Consumes the list, yielding its remaining variable indices in pop order.
    #[cfg(test)]
    pub(crate) fn drain(self, arena: &mut RemainingArena) -> impl Iterator<Item = u32> {
        let mut list = self;
        std::iter::from_fn(move || {
            let (value, rest) = list.pop_front(arena)?;
            list = rest;
            Some(value)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use merc_utilities::random_test;
    use rand::RngExt;

    use super::RemainingArena;
    use super::RemainingList;

    #[test]
    fn test_pop_front_yields_original_order() {
        let mut arena = RemainingArena::default();
        let list = RemainingList::new(&mut arena, vec![1, 2, 3]);
        assert_eq!(list.drain(&mut arena).collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn test_empty_list_pops_nothing() {
        let mut arena = RemainingArena::default();
        let list = RemainingList::new(&mut arena, vec![]);
        assert!(list.is_empty());
        assert!(list.pop_front(&mut arena).is_none());
    }

    #[test]
    fn test_appended_variables_come_after_the_original_order() {
        let mut arena = RemainingArena::default();
        let list = RemainingList::new(&mut arena, vec![1, 2]);
        let (first, rest) = list.pop_front(&mut arena).expect("non-empty");
        assert_eq!(first, 1);

        let appended = rest.with_appended(&mut arena, [10, 11]);
        assert_eq!(appended.drain(&mut arena).collect::<Vec<_>>(), vec![2, 10, 11]);
    }

    #[test]
    fn test_appended_variables_after_a_single_element_original() {
        let mut arena = RemainingArena::default();
        let list = RemainingList::new(&mut arena, vec![1]);
        let (first, rest) = list.pop_front(&mut arena).expect("non-empty");
        assert_eq!(first, 1);

        let appended = rest.with_appended(&mut arena, [10, 11]);
        assert_eq!(appended.drain(&mut arena).collect::<Vec<_>>(), vec![10, 11]);
    }

    #[test]
    fn test_siblings_popped_from_the_same_rest_agree() {
        // Two "children" popping from the same shared rest (as every
        // `Finite`-branch child does) must each see the identical
        // continuation, independent of one another.
        let mut arena = RemainingArena::default();
        let list = RemainingList::new(&mut arena, vec![1, 2, 3]);
        let (_, rest) = list.pop_front(&mut arena).expect("non-empty");

        let child_a = rest.with_appended(&mut arena, [100]);
        let child_b = rest.with_appended(&mut arena, [200]);

        assert_eq!(child_a.drain(&mut arena).collect::<Vec<_>>(), vec![2, 3, 100]);
        assert_eq!(child_b.drain(&mut arena).collect::<Vec<_>>(), vec![2, 3, 200]);
    }

    #[test]
    fn test_appending_twice_preserves_append_order() {
        let mut arena = RemainingArena::default();
        let list = RemainingList::new(&mut arena, vec![]);
        let list = list.with_appended(&mut arena, [1, 2]);
        let list = list.with_appended(&mut arena, [3, 4]);
        assert_eq!(list.drain(&mut arena).collect::<Vec<_>>(), vec![1, 2, 3, 4]);
    }

    /// Random pops and appends on random earlier versions of a list behave like a [VecDeque]
    /// that is cloned for every version: every version stays valid and independent.
    #[test]
    #[cfg_attr(miri, ignore)] // Test is too slow under miri
    fn test_random_operations_match_a_persistent_queue_model() {
        random_test(100, |rng| {
            let mut arena = RemainingArena::default();
            let original: Vec<u32> = (0..rng.random_range(0..5)).collect();
            let mut versions = vec![(
                RemainingList::new(&mut arena, original.clone()),
                VecDeque::from(original),
            )];
            let mut next = 100;

            for _ in 0..50 {
                let (list, model) = versions[rng.random_range(0..versions.len())].clone();
                if rng.random_bool(0.5) {
                    let mut model = model;
                    match list.pop_front(&mut arena) {
                        Some((value, rest)) => {
                            assert_eq!(Some(value), model.pop_front());
                            versions.push((rest, model));
                        }
                        None => assert!(model.is_empty() && list.is_empty()),
                    }
                } else {
                    let fresh: Vec<u32> = (next..next + rng.random_range(0..3)).collect();
                    next += 3;
                    let mut model = model;
                    model.extend(fresh.iter().copied());
                    versions.push((list.with_appended(&mut arena, fresh), model));
                }
            }

            for (list, model) in versions {
                assert_eq!(list.is_empty(), model.is_empty());
                assert_eq!(list.drain(&mut arena).collect::<Vec<_>>(), Vec::from(model));
            }
        });
    }
}
