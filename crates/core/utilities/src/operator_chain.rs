use std::collections::HashSet;
use std::hash::Hash;
use std::iter::FusedIterator;

use crate::recycle_vec::recycle_vec;

/// A handle to a tree node that may be an application of a binary connective, so that nested
/// applications of the same connective form a chain `a op b op c ...` that [ChainIter] flattens.
///
/// Implemented on cheap handles, such as `&T` or term references, rather than on the nodes
/// themselves, so that operands can be handed out with the lifetime of the whole tree.
pub trait BinaryChain: Clone {
    /// The connective.
    type Op: Copy;

    /// Returns the operands of this node if it is an application of `op`.
    fn binary_operands(&self, op: Self::Op) -> Option<(Self, Self)>;
}

/// Reusable storage for the worklist of a [ChainIter], so that repeated traversals do not
/// allocate.
///
/// The storage is empty between traversals, so `U` need not be the handle type of any particular
/// traversal: a long-lived context can keep, for example, a `ChainStack<TermRef<'static>>` and
/// lend it to traversals of shorter-lived terms. `U` must have the same size and alignment as the
/// handle type, so that the allocation can be reused; [ChainIter::new] checks this at compile time.
pub struct ChainStack<U> {
    stack: Vec<U>,
}

impl<U> ChainStack<U> {
    pub fn new() -> Self {
        ChainStack { stack: Vec::new() }
    }
}

impl<U> Default for ChainStack<U> {
    fn default() -> Self {
        ChainStack::new()
    }
}

/// Yields every operand of a nested `op` chain, in the order in which they are written. A node
/// that is not an application of `op` is a chain of one.
///
/// The walk uses an explicit stack rather than recursion, so that very deep chains, such as those
/// produced by quantifier enumeration, cannot overflow the call stack.
pub struct ChainIter<'s, T: BinaryChain, U> {
    stack: Vec<T>,
    op: T::Op,

    /// Receives the allocation of `stack` back when the traversal ends.
    storage: &'s mut ChainStack<U>,
}

impl<'s, T: BinaryChain, U> ChainIter<'s, T, U> {
    /// Starts a traversal of the `op` chain rooted at `root`, using the allocation of `storage`.
    pub fn new(root: T, op: T::Op, storage: &'s mut ChainStack<U>) -> Self {
        const {
            assert!(
                size_of::<T>() == size_of::<U>() && align_of::<T>() == align_of::<U>(),
                "the storage element type must have the layout of the handle type"
            );
        }

        let mut stack = recycle_vec(std::mem::take(&mut storage.stack));
        stack.push(root);

        ChainIter { stack, op, storage }
    }
}

impl<T: BinaryChain, U> Iterator for ChainIter<'_, T, U> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
        while let Some(node) = self.stack.pop() {
            match node.binary_operands(self.op) {
                Some((lhs, rhs)) => {
                    // Right operand first, so that the left one is popped first.
                    self.stack.push(rhs);
                    self.stack.push(lhs);
                }
                None => return Some(node),
            }
        }

        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        // Every node on the stack yields at least one operand.
        (self.stack.len(), None)
    }
}

impl<T: BinaryChain, U> FusedIterator for ChainIter<'_, T, U> {}

impl<T: BinaryChain, U> Drop for ChainIter<'_, T, U> {
    fn drop(&mut self) {
        self.storage.stack = recycle_vec(std::mem::take(&mut self.stack));
    }
}

/// The operands of the `op` chain rooted at `root`, like [ChainIter], but each one only once, in
/// the order of its first occurrence.
///
/// Every node is visited at most once, including the inner nodes of the chain: on maximally
/// shared terms the operands of a shared subchain are then expanded once rather than at every
/// occurrence, which would be exponential for nested sharing.
pub fn unique_chain_operands<T: BinaryChain + Hash + Eq>(root: T, op: T::Op) -> Vec<T> {
    let mut stack = vec![root];
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    while let Some(node) = stack.pop() {
        if !seen.insert(node.clone()) {
            continue;
        }

        match node.binary_operands(op) {
            Some((lhs, rhs)) => {
                stack.push(rhs);
                stack.push(lhs);
            }
            None => result.push(node),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::BinaryChain;
    use super::ChainIter;
    use super::ChainStack;

    /// A right-nested chain of `depth` connectives, which has `depth + 1` operands.
    #[derive(Clone, Copy)]
    struct Chain {
        depth: usize,
    }

    impl BinaryChain for Chain {
        type Op = ();

        fn binary_operands(&self, _op: ()) -> Option<(Chain, Chain)> {
            (self.depth > 0).then(|| (Chain { depth: 0 }, Chain { depth: self.depth - 1 }))
        }
    }

    #[test]
    fn test_chain_stack_keeps_allocation() {
        let mut storage = ChainStack::<usize>::new();

        assert_eq!(ChainIter::new(Chain { depth: 8 }, (), &mut storage).count(), 9);
        let capacity = storage.stack.capacity();
        assert!(capacity > 0, "the traversal did not return its allocation");

        assert_eq!(ChainIter::new(Chain { depth: 2 }, (), &mut storage).count(), 3);
        assert_eq!(storage.stack.capacity(), capacity);
    }
}
