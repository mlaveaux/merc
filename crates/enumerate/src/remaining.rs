#![forbid(unsafe_code)]

use crate::arena_list::ArenaList;
use crate::arena_list::ArenaListHandle;

/// Backing store for every [`RemainingList`] produced during one search.
/// Owned by [`Enumerator`](crate::enumerator::Enumerator) and
/// [`ArenaList::clear`]ed at the start of each search, so the backing
/// storage's capacity carries over to the next one.
pub(crate) type RemainingArena = ArenaList<u32>;

/// One branch's still-to-instantiate variables, as indices into
/// [`Enumerator::var_pool`](crate::enumerator::Enumerator): the search's
/// original goal order plus any fresh variables a constructor expansion has
/// appended past it along this specific branch.
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
    pub(crate) fn new(arena: &mut RemainingArena, original: Vec<u32>) -> RemainingList {
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
    pub(crate) fn with_appended(
        &self,
        arena: &mut RemainingArena,
        fresh: impl IntoIterator<Item = u32>,
    ) -> RemainingList {
        RemainingList {
            front: self.front,
            back: arena.push_all(self.back, fresh),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RemainingArena;
    use super::RemainingList;

    fn drain(arena: &mut RemainingArena, mut list: RemainingList) -> Vec<u32> {
        let mut out = Vec::new();
        while let Some((first, rest)) = list.pop_front(arena) {
            out.push(first);
            list = rest;
        }
        out
    }

    #[test]
    fn test_pop_front_yields_original_order() {
        let mut arena = RemainingArena::default();
        let list = RemainingList::new(&mut arena, vec![1, 2, 3]);
        assert_eq!(drain(&mut arena, list), vec![1, 2, 3]);
    }

    #[test]
    fn test_empty_list_pops_nothing() {
        let mut arena = RemainingArena::default();
        let list = RemainingList::new(&mut arena, vec![]);
        assert!(list.is_empty());
        assert!(list.pop_front(&mut arena).is_none());
    }

    #[test]
    fn test_single_element_list_pops_its_only_element() {
        let mut arena = RemainingArena::default();
        let list = RemainingList::new(&mut arena, vec![7]);
        assert_eq!(drain(&mut arena, list), vec![7]);
    }

    #[test]
    fn test_appended_variables_come_after_the_original_order() {
        let mut arena = RemainingArena::default();
        let list = RemainingList::new(&mut arena, vec![1, 2]);
        let (first, rest) = list.pop_front(&mut arena).expect("non-empty");
        assert_eq!(first, 1);

        let appended = rest.with_appended(&mut arena, [10, 11]);
        assert_eq!(drain(&mut arena, appended), vec![2, 10, 11]);
    }

    #[test]
    fn test_appended_variables_after_a_single_element_original() {
        let mut arena = RemainingArena::default();
        let list = RemainingList::new(&mut arena, vec![1]);
        let (first, rest) = list.pop_front(&mut arena).expect("non-empty");
        assert_eq!(first, 1);

        let appended = rest.with_appended(&mut arena, [10, 11]);
        assert_eq!(drain(&mut arena, appended), vec![10, 11]);
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

        assert_eq!(drain(&mut arena, child_a), vec![2, 3, 100]);
        assert_eq!(drain(&mut arena, child_b), vec![2, 3, 200]);
    }

    #[test]
    fn test_appending_twice_preserves_append_order() {
        let mut arena = RemainingArena::default();
        let list = RemainingList::new(&mut arena, vec![]);
        let list = list.with_appended(&mut arena, [1, 2]);
        let list = list.with_appended(&mut arena, [3, 4]);
        assert_eq!(drain(&mut arena, list), vec![1, 2, 3, 4]);
    }
}
