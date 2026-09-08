use merc_aterm::ATermRef;
use merc_aterm::Markable;
use merc_aterm::SymbolRef;
use merc_aterm::Transmutable;
use merc_aterm::storage::Marker;

/// A cheap, `Copy` handle into an [`ArenaList`]: a persistent singly-linked
/// list node.
#[derive(Clone, Copy, Default, Eq, PartialEq, Debug)]
pub(crate) struct ArenaListHandle(Option<u32>);

impl ArenaListHandle {
    pub(crate) fn is_empty(self) -> bool {
        self.0.is_none()
    }
}

struct Node<T> {
    value: T,
    parent: ArenaListHandle,
}

/// Backing store for every [`ArenaListHandle`] derived from it.
pub(crate) struct ArenaList<T> {
    nodes: Vec<Node<T>>,
}

// Written by hand rather than `#[derive(Default)]`, which would add an
// unnecessary `T: Default` bound.
impl<T> Default for ArenaList<T> {
    fn default() -> Self {
        ArenaList { nodes: Vec::new() }
    }
}

impl<T> ArenaList<T> {
    /// Drops every node, keeping the backing `Vec`'s capacity.
    pub(crate) fn clear(&mut self) {
        self.nodes.clear();
    }

    /// Returns a new handle with `value` pushed in front of `parent`.
    /// `parent` remains valid: any other handle built on it is unaffected.
    pub(crate) fn push(&mut self, parent: ArenaListHandle, value: T) -> ArenaListHandle {
        let index = u32::try_from(self.nodes.len()).expect("more arena-list nodes than fit in a u32");
        self.nodes.push(Node { value, parent });
        ArenaListHandle(Some(index))
    }

    /// [`ArenaList::push`]es every item of `values`, in order, in front of
    /// `parent`.
    pub(crate) fn push_all(&mut self, parent: ArenaListHandle, values: impl IntoIterator<Item = T>) -> ArenaListHandle {
        let mut current = parent;
        for value in values {
            current = self.push(current, value);
        }
        current
    }

    /// Splits `handle` into its value and the rest of the list, or `None` if
    /// `handle` is empty.
    pub(crate) fn pop(&self, handle: ArenaListHandle) -> Option<(&T, ArenaListHandle)> {
        let node = &self.nodes[handle.0? as usize];
        Some((&node.value, node.parent))
    }
}

/// Lets an [`ArenaList`] of term-holding values be wrapped in
/// [`merc_aterm::Protected`] directly.
impl<T: Markable> Markable for ArenaList<T> {
    fn mark(&self, marker: &mut Marker) {
        for node in &self.nodes {
            node.value.mark(marker);
        }
    }

    fn contains_term(&self, term: &ATermRef<'_>) -> bool {
        self.nodes.iter().any(|node| node.value.contains_term(term))
    }

    fn contains_symbol(&self, symbol: &SymbolRef<'_>) -> bool {
        self.nodes.iter().any(|node| node.value.contains_symbol(symbol))
    }

    fn len(&self) -> usize {
        self.nodes.len()
    }
}

// SAFETY: We only contain `T` values, which are themselves `Transmutable`, and
// a `Vec` of `Node<T>`.
unsafe impl<T: Transmutable> Transmutable for ArenaList<T>
where
    for<'a> T::Target<'a>: Sized,
{
    type Target<'a>
        = ArenaList<T::Target<'a>>
    where
        T: 'a;

    unsafe fn transmute_lifetime<'a>(&self) -> &'a Self::Target<'a> {
        // SAFETY: see the impl comment above.
        unsafe { std::mem::transmute::<&Self, &'a ArenaList<T::Target<'a>>>(self) }
    }

    unsafe fn transmute_lifetime_mut<'a>(&mut self) -> &'a mut Self::Target<'a> {
        // SAFETY: see the impl comment above.
        unsafe { std::mem::transmute::<&mut Self, &'a mut ArenaList<T::Target<'a>>>(self) }
    }
}

#[cfg(test)]
mod tests {
    use super::ArenaList;
    use super::ArenaListHandle;

    fn drain(arena: &ArenaList<u32>, mut handle: ArenaListHandle) -> Vec<u32> {
        let mut out = Vec::new();
        while let Some((&value, parent)) = arena.pop(handle) {
            out.push(value);
            handle = parent;
        }
        out
    }

    #[test]
    fn test_empty_handle_pops_nothing() {
        let arena: ArenaList<u32> = ArenaList::default();
        let handle = ArenaListHandle::default();
        assert!(handle.is_empty());
        assert!(arena.pop(handle).is_none());
    }

    #[test]
    fn test_push_then_pop_round_trips() {
        let mut arena = ArenaList::default();
        let handle = arena.push(ArenaListHandle::default(), 42);
        assert!(!handle.is_empty());
        let (&value, parent) = arena.pop(handle).expect("just pushed");
        assert_eq!(value, 42);
        assert!(parent.is_empty());
    }

    #[test]
    fn test_push_all_preserves_order_when_walked() {
        let mut arena = ArenaList::default();
        // `push_all` prepends each value in turn, so walking the result
        // yields the values in *reverse* of the order they were given.
        let handle = arena.push_all(ArenaListHandle::default(), [1, 2, 3]);
        assert_eq!(drain(&arena, handle), vec![3, 2, 1]);
    }

    #[test]
    fn test_extending_one_handle_leaves_siblings_untouched() {
        let mut arena = ArenaList::default();
        let base = arena.push_all(ArenaListHandle::default(), [1, 2]);

        let child_a = arena.push(base, 100);
        let child_b = arena.push(base, 200);

        assert_eq!(drain(&arena, child_a), vec![100, 2, 1]);
        assert_eq!(drain(&arena, child_b), vec![200, 2, 1]);
        assert_eq!(drain(&arena, base), vec![2, 1]);
    }

    #[test]
    fn test_clear_reuses_capacity_for_a_fresh_list() {
        let mut arena = ArenaList::default();
        arena.push(ArenaListHandle::default(), 7);
        arena.clear();

        let handle = arena.push(ArenaListHandle::default(), 9);
        assert_eq!(drain(&arena, handle), vec![9]);
    }
}
