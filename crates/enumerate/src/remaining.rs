#![forbid(unsafe_code)]

use std::rc::Rc;

/// One branch's still-to-instantiate variables, as indices into
/// [`Enumerator::var_pool`](crate::enumerator::Enumerator): the search's
/// original goal order plus any fresh variables a constructor expansion has
/// appended past it along this specific branch.
#[derive(Clone)]
pub(crate) struct RemainingList {
    original: Original,
    /// Fresh variables appended past `original` by constructor expansions
    /// along this branch, oldest first.
    extra: Vec<u32>,
}

/// `RemainingList`'s original-variable-order part, split so a list that
/// starts with at most one original variable never pays for an [`Rc`]
#[derive(Clone)]
enum Original {
    /// 0 or 1 elements, stored inline.
    Inline(Option<u32>),
    /// 2 or more elements
    Shared { indices: Rc<[u32]>, cursor: usize },
}

impl Original {
    fn is_empty(&self) -> bool {
        match self {
            Original::Inline(value) => value.is_none(),
            Original::Shared { indices, cursor } => *cursor >= indices.len(),
        }
    }

    /// The same position, advanced past its current head — used both to pop
    /// (the common case) and to build the "already exhausted" `original` a
    /// pop from `extra` needs to carry forward.
    fn advanced(&self) -> Original {
        match self {
            Original::Inline(_) => Original::Inline(None),
            Original::Shared { indices, cursor } => Original::Shared {
                indices: indices.clone(),
                cursor: cursor + 1,
            },
        }
    }
}

impl RemainingList {
    /// The initial list for a whole search: every original variable, in the
    /// order the caller wants them expanded in, none consumed yet.
    pub(crate) fn new(mut original: Vec<u32>) -> RemainingList {
        let original = if original.len() <= 1 {
            Original::Inline(original.pop())
        } else {
            Original::Shared {
                indices: original.into(),
                cursor: 0,
            }
        };
        RemainingList {
            original,
            extra: Vec::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.original.is_empty() && self.extra.is_empty()
    }

    /// Splits off the first still-remaining variable index and the rest of
    /// the list, or `None` if empty.
    pub(crate) fn pop_front(&self) -> Option<(u32, RemainingList)> {
        match &self.original {
            Original::Inline(Some(value)) => Some((
                *value,
                RemainingList {
                    original: self.original.advanced(),
                    extra: self.extra.clone(),
                },
            )),
            Original::Shared { indices, cursor } if *cursor < indices.len() => Some((
                indices[*cursor],
                RemainingList {
                    original: self.original.advanced(),
                    extra: self.extra.clone(),
                },
            )),
            Original::Inline(None) | Original::Shared { .. } => {
                let (&first, rest) = self.extra.split_first()?;
                Some((
                    first,
                    RemainingList {
                        original: self.original.advanced(),
                        extra: rest.to_vec(),
                    },
                ))
            }
        }
    }

    /// Returns a copy of this list with `fresh` appended at the tail
    pub(crate) fn with_appended(&self, fresh: impl IntoIterator<Item = u32>) -> RemainingList {
        let mut extra = self.extra.clone();
        extra.extend(fresh);
        RemainingList {
            original: self.original.clone(),
            extra,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RemainingList;

    fn drain(mut list: RemainingList) -> Vec<u32> {
        let mut out = Vec::new();
        while let Some((first, rest)) = list.pop_front() {
            out.push(first);
            list = rest;
        }
        out
    }

    #[test]
    fn test_pop_front_yields_original_order() {
        let list = RemainingList::new(vec![1, 2, 3]);
        assert_eq!(drain(list), vec![1, 2, 3]);
    }

    #[test]
    fn test_empty_list_pops_nothing() {
        let list = RemainingList::new(vec![]);
        assert!(list.is_empty());
        assert!(list.pop_front().is_none());
    }

    #[test]
    fn test_single_element_list_pops_its_only_element() {
        let list = RemainingList::new(vec![7]);
        assert_eq!(drain(list), vec![7]);
    }

    #[test]
    fn test_appended_variables_come_after_the_original_order() {
        let list = RemainingList::new(vec![1, 2]);
        let (first, rest) = list.pop_front().expect("non-empty");
        assert_eq!(first, 1);

        let appended = rest.with_appended([10, 11]);
        assert_eq!(drain(appended), vec![2, 10, 11]);
    }

    #[test]
    fn test_appended_variables_after_a_single_element_original() {
        let list = RemainingList::new(vec![1]);
        let (first, rest) = list.pop_front().expect("non-empty");
        assert_eq!(first, 1);

        let appended = rest.with_appended([10, 11]);
        assert_eq!(drain(appended), vec![10, 11]);
    }

    #[test]
    fn test_siblings_popped_from_the_same_rest_agree() {
        // Two "children" popping from the same shared rest (as every
        // `Finite`-branch child does) must each see the identical
        // continuation, independent of one another.
        let list = RemainingList::new(vec![1, 2, 3]);
        let (_, rest) = list.pop_front().expect("non-empty");

        let child_a = rest.with_appended([100]);
        let child_b = rest.with_appended([200]);

        assert_eq!(drain(child_a), vec![2, 3, 100]);
        assert_eq!(drain(child_b), vec![2, 3, 200]);
    }

    #[test]
    fn test_appending_twice_preserves_append_order() {
        let list = RemainingList::new(vec![]);
        let list = list.with_appended([1, 2]);
        let list = list.with_appended([3, 4]);
        assert_eq!(drain(list), vec![1, 2, 3, 4]);
    }
}
