use std::cmp::Ordering;
use std::hash::Hash;
use std::hash::Hasher;
use std::ops::Deref;
use std::ops::DerefMut;

use merc_utilities::Span;

/// Lets a value detach its own same-type recursive children, one level at a time, in place of
/// letting the compiler's default field-by-field drop glue reach them by native recursion.
///
/// [Spanned]'s own [Drop] impl is what actually needs this: a handful of the `T`s it wraps
/// (`StateFrmKind`, `DataExprKind`, ...) nest through a `Box<Spanned<T>>` chain of their own type,
/// and a pathologically deep chain of those overflows the stack on drop exactly the same way
/// unbounded native recursion overflows it during a *traversal* -- a distinct, separate bug from
/// the one [`crate::Traverse`]'s own stack-safety fix addressed, since dropping a value never runs
/// a traversal at all (see `review/stack-overflow-recursion.md`).
///
/// The default implementation is a no-op, correct for every `T` with no same-type recursion at all
/// -- the vast majority [Spanned] is used with (identifiers, single-level declarations, ...). Only
/// the eight node-kind enums this crate's `Traverse` impls cover (see
/// `crates/syntax/src/traverse.rs`'s `define_traversal!` invocations, which also generate this)
/// override it with real extraction logic, reusing the exact same per-variant child positions
/// `Traverse::push_children_mut` already enumerates for those types.
pub trait TakeRecursiveChildren: Sized {
    /// Replaces every same-type recursive child of `self` with a cheap, non-recursive value,
    /// pushing the real (possibly still deep) one it replaced onto `stack` instead of returning it,
    /// so a caller can keep detaching layers iteratively without native recursion.
    fn take_recursive_children(&mut self, stack: &mut Vec<Self>) {
        let _ = stack;
    }
}

impl TakeRecursiveChildren for String {}

/// A value of type `T` paired with the source [Span] it originates from.
///
/// Equality, ordering and hashing deliberately ignore the [Span] and consider
/// only `node`, so two structurally identical values at different source
/// locations compare and hash equal. Many passes rely on this structural
/// equality (hash maps, deduplication, `assert_eq!` in tests).
#[derive(Clone, Debug, Default)]
pub struct Spanned<T: TakeRecursiveChildren> {
    /// The wrapped value.
    pub node: T,
    /// The source location the value originates from.
    pub span: Span,
}

impl<T: TakeRecursiveChildren + Default> Spanned<T> {
    /// Splits `self` into its wrapped value and its span, without a partial move.
    ///
    /// `let Spanned { node, span } = value;`-style destructuring an *owned* [Spanned] is no longer
    /// legal now that it implements [Drop] (Rust forbids moving just one field out of a value that
    /// has a destructor, since it can no longer run that destructor on "the rest" of a
    /// partially-moved-from value -- this crate is `#![forbid(unsafe_code)]`, so that destructor
    /// can't be sidestepped with `ManuallyDrop` either). [std::mem::take] replaces each field with
    /// a cheap [Default] value in place instead, leaving `self` fully intact (if trivial) for its
    /// own ordinary drop to handle once this function returns.
    pub fn into_parts(mut self) -> (T, Span) {
        (std::mem::take(&mut self.node), std::mem::take(&mut self.span))
    }

    /// Discards the span and returns just the wrapped value; see [Self::into_parts].
    pub fn into_node(self) -> T {
        self.into_parts().0
    }

    /// Transforms the wrapped value while preserving the span.
    pub fn map<U: TakeRecursiveChildren + Default>(self, function: impl FnOnce(T) -> U) -> Spanned<U> {
        let (node, span) = self.into_parts();
        Spanned {
            node: function(node),
            span,
        }
    }
}

/// Wraps `node` together with its source `span`.
pub fn respan<T: TakeRecursiveChildren>(span: Span, node: T) -> Spanned<T> {
    Spanned { node, span }
}

impl<T: TakeRecursiveChildren> Deref for Spanned<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.node
    }
}

impl<T: TakeRecursiveChildren> DerefMut for Spanned<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.node
    }
}

impl<T: TakeRecursiveChildren + PartialEq> PartialEq for Spanned<T> {
    fn eq(&self, other: &Self) -> bool {
        self.node == other.node
    }
}

impl<T: TakeRecursiveChildren + Eq> Eq for Spanned<T> {}

impl<T: TakeRecursiveChildren + PartialOrd> PartialOrd for Spanned<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.node.partial_cmp(&other.node)
    }
}

impl<T: TakeRecursiveChildren + Ord> Ord for Spanned<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.node.cmp(&other.node)
    }
}

impl<T: TakeRecursiveChildren + Hash> Hash for Spanned<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.node.hash(state);
    }
}

/// Prevents the compiler's default, field-by-field drop glue from recursing through a
/// `Box`-chained tree one node at a time -- unbounded for a pathologically deep tree, and the
/// reason [TakeRecursiveChildren] exists (see its own doc comment).
///
/// Detaches every same-type child from `self.node` (via [TakeRecursiveChildren]) onto an explicit,
/// heap-allocated worklist and processes it iteratively rather than recursively: each popped value
/// has its own children detached the same way *before* it's allowed to drop (implicitly, at the end
/// of the loop body), so by the time any single value's fields are actually freed, none of them
/// still holds a deep subtree -- only ever the cheap, non-recursive value
/// [TakeRecursiveChildren::take_recursive_children] replaced it with.
///
/// A value dropped this way still re-enters this same `drop` once more per popped child (Rust has
/// no way to skip a type's own `Drop` when an owned value naturally goes out of scope), but by then
/// its children are already detached, so that re-entry only ever re-examines already-cheap values
/// and returns immediately: a small, constant amount of harmless extra nesting per node, not
/// nesting proportional to the tree's depth.
impl<T: TakeRecursiveChildren> Drop for Spanned<T> {
    fn drop(&mut self) {
        let mut stack = Vec::new();
        self.node.take_recursive_children(&mut stack);
        while let Some(mut child) = stack.pop() {
            child.take_recursive_children(&mut stack);
        }
    }
}
