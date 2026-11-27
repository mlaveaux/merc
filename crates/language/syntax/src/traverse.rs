use std::convert::Infallible;
use std::ops::ControlFlow;

use merc_utilities::Step;
use merc_utilities::Visit;

use crate::ActFrm;
use crate::ActFrmKind;
use crate::AssignmentData;
use crate::BagElement;
use crate::ConstructorDecl;
use crate::DataExpr;
use crate::DataExprKind;
use crate::DataExprUpdate;
use crate::PbesExpr;
use crate::PbesExprKind;
use crate::PresExpr;
use crate::PresExprKind;
use crate::ProcessExpr;
use crate::ProcessExprKind;
use crate::RegFrm;
use crate::RegFrmKind;
use crate::SortExpression;
use crate::SortExpressionKind;
use crate::Spanned;
use crate::StateFrm;
use crate::StateFrmKind;
use crate::TakeRecursiveChildren;

/// The outcome of descending into a subtree: `Continue(())` when it was fully traversed, or
/// `Break(Ok(value))`/`Break(Err(error))` when it stopped early. Both outcomes share the break arm
/// so a recursive step can propagate either with a single `?`.
pub type Recursion<T, E> = ControlFlow<Result<T, E>, ()>;

/// A node from any of this crate's traversable trees, named by which one it is.
#[derive(Clone, Copy, Debug)]
pub enum MixedNode<'a> {
    SortExpression(&'a SortExpression),
    DataExpr(&'a DataExpr),
    ProcessExpr(&'a ProcessExpr),
    StateFrm(&'a StateFrm),
    RegFrm(&'a RegFrm),
    ActFrm(&'a ActFrm),
    PbesExpr(&'a PbesExpr),
    PresExpr(&'a PresExpr),
}

/// Pushes the children of `node`, of any node type, onto `stack` so that they are popped in
/// written order: first the children of another type, then those of the same type.
fn mixed_push_children<'a>(node: MixedNode<'a>, stack: &mut Vec<MixedNode<'a>>) {
    fn extend<'a, N: Traverse>(node: &'a N, into: &mut Vec<MixedNode<'a>>) {
        node.push_mixed_children(into);
        let mut children = Vec::new();
        let _ = node.push_children((), &mut children);
        into.extend(children.into_iter().map(|(child, ())| child.as_mixed()));
    }

    let mut children = Vec::new();
    match node {
        MixedNode::SortExpression(node) => extend(node, &mut children),
        MixedNode::DataExpr(node) => extend(node, &mut children),
        MixedNode::ProcessExpr(node) => extend(node, &mut children),
        MixedNode::StateFrm(node) => extend(node, &mut children),
        MixedNode::RegFrm(node) => extend(node, &mut children),
        MixedNode::ActFrm(node) => extend(node, &mut children),
        MixedNode::PbesExpr(node) => extend(node, &mut children),
        MixedNode::PresExpr(node) => extend(node, &mut children),
    }
    // Reversed, so that the stack pops them in written order.
    stack.extend(children.into_iter().rev());
}

/// A syntax tree node whose children are of the same type.
///
/// An implementation only defines the direct children ([Traverse::push_children] and
/// [Traverse::push_children_mut]); the walks are provided. Top-down walks visit in pre-order and
/// children left to right; [Traverse::try_transform] rewrites in post-order. All walks use an
/// explicit heap stack instead of native recursion, so trees of any depth can be traversed without
/// overflowing the call stack.
///
/// The walks stay within one node type, e.g. they do not enter the [RegFrm] of a [StateFrm]
/// modality, except [Traverse::visit_mixed] and [Traverse::try_visit_mixed], which visit
/// every [MixedNode].
pub trait Traverse: Sized {
    /// Appends each direct child of this node to `stack`, paired with `context`, in written order.
    fn push_children<'a, C: Copy>(
        &'a self,
        context: C,
        stack: &mut Vec<(&'a Self, C)>,
    ) -> Recursion<Infallible, Infallible>;

    /// See [Traverse::push_children]; this variant lets the callback replace nodes in place.
    fn push_children_mut<'a, C: Copy>(
        &'a mut self,
        context: C,
        stack: &mut Vec<(&'a mut Self, C)>,
    ) -> Recursion<Infallible, Infallible>;

    /// Rewrites the subtree of each direct child bottom-up with [Traverse::try_transform], left to
    /// right; this node itself is not passed to `function`. Stops at the first error.
    fn transform_children<E, F>(&mut self, function: &mut F) -> Result<(), E>
    where
        Self: Default,
        F: FnMut(&mut Self) -> Result<(), E>,
    {
        let mut children = Vec::new();
        let _ = self.push_children_mut((), &mut children);
        for (child, ()) in children {
            let mut owned = std::mem::take(child);
            owned.try_transform(function)?;
            *child = owned;
        }
        Ok(())
    }

    /// Wraps `self` in the [MixedNode] variant of its own type.
    fn as_mixed(&self) -> MixedNode<'_>;

    /// Appends the children of this node that are of a different [Traverse] type to `sink`, in
    /// written order. Only [StateFrm] and [RegFrm] have such children.
    fn push_mixed_children<'a>(&'a self, _sink: &mut Vec<MixedNode<'a>>) {}

    /// Calls `step` on the nodes of `stack` and their subtrees in pre-order, popping from the end
    /// of `stack`. Stops at the first break or error.
    fn drive<C, T, E>(
        mut stack: Vec<(&mut Self, C)>,
        mut step: impl FnMut(&mut Self, C) -> Visit<Infallible, C, T, E>,
    ) -> Recursion<T, E>
    where
        C: Copy,
    {
        while let Some((node, context)) = stack.pop() {
            let context = match step(node, context) {
                Err(error) => return ControlFlow::Break(Err(error)),
                Ok(ControlFlow::Break(value)) => return ControlFlow::Break(Ok(value)),
                Ok(ControlFlow::Continue(Step::Prune)) => continue,
                // Uninhabited: a mutating walk replaces the node inside `step`.
                Ok(ControlFlow::Continue(Step::Replace(replacement))) => match replacement {},
                Ok(ControlFlow::Continue(Step::Into(context))) => context,
            };
            let start = stack.len();
            let _ = node.push_children_mut(context, &mut stack);
            stack[start..].reverse();
        }
        ControlFlow::Continue(())
    }

    /// Visits this node and its subtree in pre-order, threading `context` from a node to its
    /// children. Stops at the first break or error; [Step::Prune] skips the children of a node.
    fn visit_subtree<'a, C, T, E, F>(&'a self, context: C, function: &mut F) -> Recursion<T, E>
    where
        C: Copy,
        F: FnMut(&'a Self, C) -> Visit<Infallible, C, T, E>,
    {
        let mut stack: Vec<(&Self, C)> = vec![(self, context)];
        while let Some((node, context)) = stack.pop() {
            let context = match function(node, context) {
                Err(error) => return ControlFlow::Break(Err(error)),
                Ok(ControlFlow::Break(value)) => return ControlFlow::Break(Ok(value)),
                Ok(ControlFlow::Continue(Step::Prune)) => continue,
                Ok(ControlFlow::Continue(Step::Replace(replacement))) => match replacement {},
                Ok(ControlFlow::Continue(Step::Into(context))) => context,
            };
            let start = stack.len();
            let _ = node.push_children(context, &mut stack);
            stack[start..].reverse();
        }
        ControlFlow::Continue(())
    }

    /// See [Traverse::visit_subtree], with `enter` as the callback; additionally calls `exit` once
    /// the subtree of a node is done, for scoped `state`.
    ///
    /// `exit` receives the node and the `context` that `enter` received. It runs for every node
    /// whose `enter` continued, including pruned nodes, innermost first; when the walk stops on a
    /// break or error, `exit` still runs for every node that is still open, but not for the node
    /// that stopped it.
    fn visit_subtree_scoped<'a, C, S, T, E, F, G>(
        &'a self,
        context: C,
        state: &mut S,
        enter: &mut F,
        exit: &mut G,
    ) -> Recursion<T, E>
    where
        C: Copy,
        F: FnMut(&'a Self, C, &mut S) -> Visit<Infallible, C, T, E>,
        G: FnMut(&'a Self, C, &mut S),
    {
        enum Frame<'a, N, C> {
            Enter(&'a N, C),
            Exit(&'a N, C),
        }

        fn unwind<'a, N, C, S>(stack: Vec<Frame<'a, N, C>>, state: &mut S, exit: &mut impl FnMut(&'a N, C, &mut S)) {
            for frame in stack.into_iter().rev() {
                if let Frame::Exit(node, context) = frame {
                    exit(node, context, state);
                }
            }
        }

        let mut stack = vec![Frame::Enter(self, context)];
        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Exit(node, context) => exit(node, context, state),
                Frame::Enter(node, context) => match enter(node, context, state) {
                    Err(error) => {
                        unwind(stack, state, exit);
                        return ControlFlow::Break(Err(error));
                    }
                    Ok(ControlFlow::Break(value)) => {
                        unwind(stack, state, exit);
                        return ControlFlow::Break(Ok(value));
                    }
                    Ok(ControlFlow::Continue(Step::Replace(replacement))) => match replacement {},
                    Ok(ControlFlow::Continue(Step::Prune)) => exit(node, context, state),
                    Ok(ControlFlow::Continue(Step::Into(child_context))) => {
                        stack.push(Frame::Exit(node, context));
                        let mut children = Vec::new();
                        let _ = node.push_children(child_context, &mut children);
                        stack.extend(
                            children
                                .into_iter()
                                .rev()
                                .map(|(child, context)| Frame::Enter(child, context)),
                        );
                    }
                },
            }
        }
        ControlFlow::Continue(())
    }

    /// See [Traverse::visit_subtree_scoped]; returns the value of a break, like
    /// [Traverse::visit_with].
    fn visit_scoped<'a, C, S, T, E, F, G>(
        &'a self,
        context: C,
        state: &mut S,
        mut enter: F,
        mut exit: G,
    ) -> Result<Option<T>, E>
    where
        C: Copy,
        F: FnMut(&'a Self, C, &mut S) -> Visit<Infallible, C, T, E>,
        G: FnMut(&'a Self, C, &mut S),
    {
        match self.visit_subtree_scoped(context, state, &mut enter, &mut exit) {
            ControlFlow::Break(Ok(value)) => Ok(Some(value)),
            ControlFlow::Break(Err(error)) => Err(error),
            ControlFlow::Continue(()) => Ok(None),
        }
    }

    /// See [Traverse::visit_subtree_scoped]; `enter` may modify the node in place before its
    /// children are visited.
    ///
    /// `exit` receives only the `context` and `state`, not the node, since the node is mutably
    /// borrowed to reach its children. An `exit` that needs to know the node can have `enter` record
    /// it in `state`.
    fn apply_subtree_scoped<C, S, T, E, F, G>(
        &mut self,
        context: C,
        state: &mut S,
        enter: &mut F,
        exit: &mut G,
    ) -> Recursion<T, E>
    where
        C: Copy,
        F: FnMut(&mut Self, C, &mut S) -> Visit<Infallible, C, T, E>,
        G: FnMut(C, &mut S),
    {
        enum Frame<'a, N, C> {
            Enter(&'a mut N, C),
            Exit(C),
        }

        fn unwind<N, C, S>(stack: Vec<Frame<'_, N, C>>, state: &mut S, exit: &mut impl FnMut(C, &mut S)) {
            for frame in stack.into_iter().rev() {
                if let Frame::Exit(context) = frame {
                    exit(context, state);
                }
            }
        }

        let mut stack = vec![Frame::Enter(self, context)];
        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Exit(context) => exit(context, state),
                Frame::Enter(node, context) => match enter(node, context, state) {
                    Err(error) => {
                        unwind(stack, state, exit);
                        return ControlFlow::Break(Err(error));
                    }
                    Ok(ControlFlow::Break(value)) => {
                        unwind(stack, state, exit);
                        return ControlFlow::Break(Ok(value));
                    }
                    Ok(ControlFlow::Continue(Step::Replace(replacement))) => match replacement {},
                    Ok(ControlFlow::Continue(Step::Prune)) => exit(context, state),
                    Ok(ControlFlow::Continue(Step::Into(child_context))) => {
                        stack.push(Frame::Exit(context));
                        let mut children = Vec::new();
                        let _ = node.push_children_mut(child_context, &mut children);
                        stack.extend(
                            children
                                .into_iter()
                                .rev()
                                .map(|(child, context)| Frame::Enter(child, context)),
                        );
                    }
                },
            }
        }
        ControlFlow::Continue(())
    }

    /// See [Traverse::apply_subtree_scoped]; returns the value of a break, like
    /// [Traverse::apply_with].
    fn apply_scoped<C, S, T, E, F, G>(
        &mut self,
        context: C,
        state: &mut S,
        mut enter: F,
        mut exit: G,
    ) -> Result<Option<T>, E>
    where
        C: Copy,
        F: FnMut(&mut Self, C, &mut S) -> Visit<Infallible, C, T, E>,
        G: FnMut(C, &mut S),
    {
        match self.apply_subtree_scoped(context, state, &mut enter, &mut exit) {
            ControlFlow::Break(Ok(value)) => Ok(Some(value)),
            ControlFlow::Break(Err(error)) => Err(error),
            ControlFlow::Continue(()) => Ok(None),
        }
    }

    /// See [Traverse::visit_subtree]; [Step::Replace] replaces the node in place, and the
    /// replacement is not descended into.
    fn apply_subtree<C, T, E, F>(&mut self, context: C, function: &mut F) -> Recursion<T, E>
    where
        C: Copy,
        F: FnMut(&Self, C) -> Visit<Self, C, T, E>,
    {
        let stack = vec![(self, context)];
        Self::drive(stack, |node, context| match function(node, context) {
            Err(error) => Err(error),
            Ok(ControlFlow::Break(value)) => Ok(ControlFlow::Break(value)),
            Ok(ControlFlow::Continue(Step::Prune)) => Ok(ControlFlow::Continue(Step::Prune)),
            Ok(ControlFlow::Continue(Step::Into(context))) => Ok(ControlFlow::Continue(Step::Into(context))),
            Ok(ControlFlow::Continue(Step::Replace(replacement))) => {
                *node = replacement;
                Ok(ControlFlow::Continue(Step::Prune))
            }
        })
    }

    /// Visits the subtree rooted at each direct child of this node (not this node itself).
    fn visit_children<'a, C, T, E, F>(&'a self, context: C, function: &mut F) -> Recursion<T, E>
    where
        C: Copy,
        F: FnMut(&'a Self, C) -> Visit<Infallible, C, T, E>,
    {
        let mut stack = Vec::new();
        let _ = self.push_children(context, &mut stack);
        stack.reverse();
        while let Some((node, context)) = stack.pop() {
            let context = match function(node, context) {
                Err(error) => return ControlFlow::Break(Err(error)),
                Ok(ControlFlow::Break(value)) => return ControlFlow::Break(Ok(value)),
                Ok(ControlFlow::Continue(Step::Prune)) => continue,
                Ok(ControlFlow::Continue(Step::Replace(replacement))) => match replacement {},
                Ok(ControlFlow::Continue(Step::Into(context))) => context,
            };
            let start = stack.len();
            let _ = node.push_children(context, &mut stack);
            stack[start..].reverse();
        }
        ControlFlow::Continue(())
    }

    /// See [Traverse::visit_children]; this variant lets the callback replace nodes in place.
    fn apply_children<C, T, E, F>(&mut self, context: C, function: &mut F) -> Recursion<T, E>
    where
        C: Copy,
        F: FnMut(&Self, C) -> Visit<Self, C, T, E>,
    {
        let mut stack = Vec::new();
        let _ = self.push_children_mut(context, &mut stack);
        stack.reverse();
        Self::drive(stack, |node, context| match function(node, context) {
            Err(error) => Err(error),
            Ok(ControlFlow::Break(value)) => Ok(ControlFlow::Break(value)),
            Ok(ControlFlow::Continue(Step::Prune)) => Ok(ControlFlow::Continue(Step::Prune)),
            Ok(ControlFlow::Continue(Step::Into(context))) => Ok(ControlFlow::Continue(Step::Into(context))),
            Ok(ControlFlow::Continue(Step::Replace(replacement))) => {
                *node = replacement;
                Ok(ControlFlow::Continue(Step::Prune))
            }
        })
    }

    /// Visits this node and its subtree top-down, threading `context` from a node to its children.
    ///
    /// Returns the value the callback broke with, or `None` when the whole subtree was visited.
    fn visit_with<'a, C, T, E, F>(&'a self, context: C, mut function: F) -> Result<Option<T>, E>
    where
        C: Copy,
        F: FnMut(&'a Self, C) -> Visit<Infallible, C, T, E>,
    {
        match self.visit_subtree(context, &mut function) {
            ControlFlow::Break(Ok(value)) => Ok(Some(value)),
            ControlFlow::Break(Err(error)) => Err(error),
            ControlFlow::Continue(()) => Ok(None),
        }
    }

    /// See [Traverse::visit_with], for callbacks that need neither a context nor pruning.
    fn try_visit<'a, T, E, F>(&'a self, mut function: F) -> Result<Option<T>, E>
    where
        F: FnMut(&'a Self) -> Result<ControlFlow<T>, E>,
    {
        self.visit_with((), |node, context| {
            Ok(match function(node)? {
                ControlFlow::Break(value) => ControlFlow::Break(value),
                ControlFlow::Continue(()) => ControlFlow::Continue(Step::Into(context)),
            })
        })
    }

    /// See [Traverse::try_visit], for callbacks that cannot fail.
    fn visit<'a, T, F>(&'a self, mut function: F) -> Option<T>
    where
        F: FnMut(&'a Self) -> ControlFlow<T>,
    {
        match self.try_visit::<T, Infallible, _>(|node| Ok(function(node))) {
            Ok(result) => result,
            Err(error) => match error {},
        }
    }

    /// Visits `self` and its subtree in pre-order, including the children of another node type
    /// (e.g. [StateFrm] to [RegFrm] to [ActFrm]). The children of another type of a node are
    /// visited before its children of the same type. Stops at the first break or error.
    ///
    /// There is no context and no pruning or replacement.
    fn try_visit_mixed<'a, T, E>(
        &'a self,
        mut function: impl FnMut(MixedNode<'a>) -> Result<ControlFlow<T>, E>,
    ) -> Result<Option<T>, E> {
        let mut stack = vec![self.as_mixed()];
        while let Some(node) = stack.pop() {
            if let ControlFlow::Break(value) = function(node)? {
                return Ok(Some(value));
            }
            mixed_push_children(node, &mut stack);
        }
        Ok(None)
    }

    /// See [Traverse::try_visit_mixed], for callbacks that cannot fail.
    fn visit_mixed<'a, T>(&'a self, mut function: impl FnMut(MixedNode<'a>) -> ControlFlow<T>) -> Option<T> {
        match self.try_visit_mixed::<T, Infallible>(|node| Ok(function(node))) {
            Ok(result) => result,
            Err(error) => match error {},
        }
    }

    /// Rewrites this node and its subtree top-down, threading `context` from a node to its
    /// children.
    ///
    /// Returns the value the callback broke with, in which case the tree is left partially
    /// rewritten.
    fn apply_with<C, T, E, F>(&mut self, context: C, mut function: F) -> Result<Option<T>, E>
    where
        C: Copy,
        F: FnMut(&Self, C) -> Visit<Self, C, T, E>,
    {
        match self.apply_subtree(context, &mut function) {
            ControlFlow::Break(Ok(value)) => Ok(Some(value)),
            ControlFlow::Break(Err(error)) => Err(error),
            ControlFlow::Continue(()) => Ok(None),
        }
    }

    /// Replaces every node for which `function` returns `Some(replacement)`, in place.
    ///
    /// A replacement is not descended into, so a callback that rewrites a node into a tree
    /// containing that same node terminates.
    fn apply_mut<E, F>(&mut self, mut function: F) -> Result<(), E>
    where
        F: FnMut(&Self) -> Result<Option<Self>, E>,
    {
        let broken = self.apply_with::<(), Infallible, E, _>((), |node, context| {
            Ok(ControlFlow::Continue(match function(node)? {
                Some(replacement) => Step::Replace(replacement),
                None => Step::Into(context),
            }))
        })?;

        match broken {
            Some(value) => match value {},
            None => Ok(()),
        }
    }

    /// See [Traverse::apply_mut], for callers that own the node.
    fn apply<E, F>(mut self, function: F) -> Result<Self, E>
    where
        F: FnMut(&Self) -> Result<Option<Self>, E>,
    {
        self.apply_mut(function)?;
        Ok(self)
    }

    /// Rewrites this node and its subtree in post-order, children left to right: `function` is
    /// called on a node after its children have been rewritten and put back. Stops at the first
    /// error, in which case the tree is left in an unspecified state.
    ///
    /// The children are detached (with [std::mem::take]) while their subtrees are rewritten, since
    /// a node cannot be borrowed mutably while its rewritten children are moved back into it.
    fn try_transform<E, F>(&mut self, function: &mut F) -> Result<(), E>
    where
        Self: Default,
        F: FnMut(&mut Self) -> Result<(), E>,
    {
        enum Frame<N> {
            Enter(N),
            Assemble(N, usize),
        }

        let mut stack = vec![Frame::Enter(std::mem::take(self))];
        let mut results: Vec<Self> = Vec::new();

        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Enter(mut node) => {
                    let mut child_refs = Vec::new();
                    let _ = node.push_children_mut((), &mut child_refs);
                    let children: Vec<Self> = child_refs
                        .into_iter()
                        .map(|(child, ())| std::mem::take(child))
                        .collect();
                    stack.push(Frame::Assemble(node, children.len()));
                    stack.extend(children.into_iter().rev().map(Frame::Enter));
                }
                Frame::Assemble(mut node, count) => {
                    let start = results.len() - count;
                    let mut transformed = results.split_off(start).into_iter();
                    let mut child_refs = Vec::new();
                    let _ = node.push_children_mut((), &mut child_refs);
                    for (child, ()) in child_refs {
                        *child = transformed.next().expect(
                            "Assemble's own child_count matches how many children Enter detached from this same node",
                        );
                    }
                    function(&mut node)?;
                    results.push(node);
                }
            }
        }

        *self = results
            .pop()
            .expect("stack only ever empties with exactly one result: the transformed root");
        Ok(())
    }

    /// See [Traverse::try_transform], for callbacks that cannot fail.
    fn transform<F>(&mut self, mut function: F)
    where
        Self: Default,
        F: FnMut(&mut Self),
    {
        match self.try_transform::<Infallible, _>(&mut |node| {
            function(node);
            Ok(())
        }) {
            Ok(()) => {}
            Err(error) => match error {},
        }
    }
}

/// Implements [Traverse] for a node type from a description of its children.
///
/// Every node type is a [crate::Spanned] wrapper around a `Kind` enum; match arms are written
/// against the kind, with the span carried along untouched.
///
/// The `children` list calls `recurse` on every child; it's shared between the shared and mutable
/// recursion, so it must bind through match ergonomics, never `&x.field`/`&mut x.field`. `Box`
/// fields aren't visible to match ergonomics, so an arm that dereferences one is written twice, in
/// `shared_only`/`mut_only`.
///
/// An optional `mixed: { ... }` section describes children of a *different* node type, for
/// [Traverse::push_mixed_children]; omitting it leaves that method at its empty default.
///
/// It also implements [TakeRecursiveChildren] for `$Kind` from the same arms, so that dropping a deep
/// tree does not recurse; this needs `$Kind: Default`.
macro_rules! define_traversal {
    (
        node: $Node:ident,
        kind: $Kind:ident,
        children: |$recurse:ident| { $($child:tt)* },
    ) => {
        define_traversal! {
            node: $Node,
            kind: $Kind,
            children: |$recurse| { $($child)* },
            shared_only: {},
            mut_only: {},
        }
    };
    (
        node: $Node:ident,
        kind: $Kind:ident,
        children: |$recurse:ident| { $($child:tt)* },
        mixed: { $($mixed_child:tt)* },
    ) => {
        define_traversal! {
            node: $Node,
            kind: $Kind,
            children: |$recurse| { $($child)* },
            shared_only: {},
            mut_only: {},
            mixed: { $($mixed_child)* },
        }
    };
    (
        node: $Node:ident,
        kind: $Kind:ident,
        children: |$recurse:ident| { $($child:tt)* },
        shared_only: { $($shared_child:tt)* },
        mut_only: { $($mut_child:tt)* },
    ) => {
        impl Traverse for $Node {
            fn as_mixed(&self) -> MixedNode<'_> {
                MixedNode::$Node(self)
            }

            fn push_children<'a, C: Copy>(
                &'a self,
                context: C,
                stack: &mut Vec<(&'a Self, C)>,
            ) -> Recursion<Infallible, Infallible> {
                let mut $recurse = |child: &'a $Node| -> Recursion<Infallible, Infallible> {
                    stack.push((child, context));
                    ControlFlow::Continue(())
                };

                match &self.node {
                    $($child)*
                    $($shared_child)*
                }

                ControlFlow::Continue(())
            }

            fn push_children_mut<'a, C: Copy>(
                &'a mut self,
                context: C,
                stack: &mut Vec<(&'a mut Self, C)>,
            ) -> Recursion<Infallible, Infallible> {
                let mut $recurse = |child: &'a mut $Node| -> Recursion<Infallible, Infallible> {
                    stack.push((child, context));
                    ControlFlow::Continue(())
                };

                match &mut self.node {
                    $($child)*
                    $($mut_child)*
                }

                ControlFlow::Continue(())
            }
        }

        impl TakeRecursiveChildren for $Kind {
            fn take_recursive_children(&mut self, stack: &mut Vec<Self>) {
                // `inner` gives the `?` in the arms a function returning `Recursion`.
                fn inner(this: &mut $Kind, stack: &mut Vec<$Kind>) -> Recursion<Infallible, Infallible> {
                    let mut $recurse = |child: &mut $Node| -> Recursion<Infallible, Infallible> {
                        stack.push(std::mem::take(child).into_node());
                        ControlFlow::Continue(())
                    };

                    match this {
                        $($child)*
                        $($mut_child)*
                    }

                    ControlFlow::Continue(())
                }
                let _ = inner(self, stack);
            }
        }
    };
    (
        node: $Node:ident,
        kind: $Kind:ident,
        children: |$recurse:ident| { $($child:tt)* },
        shared_only: { $($shared_child:tt)* },
        mut_only: { $($mut_child:tt)* },
        mixed: { $($mixed_child:tt)* },
    ) => {
        impl Traverse for $Node {
            fn as_mixed(&self) -> MixedNode<'_> {
                MixedNode::$Node(self)
            }

            fn push_children<'a, C: Copy>(
                &'a self,
                context: C,
                stack: &mut Vec<(&'a Self, C)>,
            ) -> Recursion<Infallible, Infallible> {
                let mut $recurse = |child: &'a $Node| -> Recursion<Infallible, Infallible> {
                    stack.push((child, context));
                    ControlFlow::Continue(())
                };

                match &self.node {
                    $($child)*
                    $($shared_child)*
                }

                ControlFlow::Continue(())
            }

            fn push_children_mut<'a, C: Copy>(
                &'a mut self,
                context: C,
                stack: &mut Vec<(&'a mut Self, C)>,
            ) -> Recursion<Infallible, Infallible> {
                let mut $recurse = |child: &'a mut $Node| -> Recursion<Infallible, Infallible> {
                    stack.push((child, context));
                    ControlFlow::Continue(())
                };

                match &mut self.node {
                    $($child)*
                    $($mut_child)*
                }

                ControlFlow::Continue(())
            }

            fn push_mixed_children<'a>(&'a self, sink: &mut Vec<MixedNode<'a>>) {
                let mut $recurse = |child: MixedNode<'a>| sink.push(child);

                match &self.node {
                    $($mixed_child)*
                    #[allow(unreachable_patterns)]
                    _ => {}
                }
            }
        }

        impl TakeRecursiveChildren for $Kind {
            fn take_recursive_children(&mut self, stack: &mut Vec<Self>) {
                // `inner` gives the `?` in the arms a function returning `Recursion`.
                fn inner(this: &mut $Kind, stack: &mut Vec<$Kind>) -> Recursion<Infallible, Infallible> {
                    let mut $recurse = |child: &mut $Node| -> Recursion<Infallible, Infallible> {
                        stack.push(std::mem::take(child).into_node());
                        ControlFlow::Continue(())
                    };

                    match this {
                        $($child)*
                        $($mut_child)*
                    }

                    ControlFlow::Continue(())
                }
                let _ = inner(self, stack);
            }
        }
    };
}

define_traversal! {
    node: SortExpression,
    kind: SortExpressionKind,
    children: |recurse| {
        SortExpressionKind::Product { lhs, rhs } => {
            recurse(lhs)?;
            recurse(rhs)?;
        }
        SortExpressionKind::Function { domain, range } => {
            recurse(domain)?;
            recurse(range)?;
        }
        SortExpressionKind::FlattenedFunction { domain, range } => {
            for sort in domain {
                recurse(sort)?;
            }
            recurse(range)?;
        }
        SortExpressionKind::Struct { inner } => {
            for constructor in inner {
                let ConstructorDecl { args, .. } = constructor;
                for (_name, sort) in args {
                    recurse(sort)?;
                }
            }
        }
        SortExpressionKind::Complex(_complex_sort, sort) => {
            recurse(sort)?;
        }
        SortExpressionKind::Reference(_)
        | SortExpressionKind::TypeVar(_)
        | SortExpressionKind::ResolvedTypeVar(_)
        | SortExpressionKind::Simple(_)
        | SortExpressionKind::Resolved(_, _) => {}
    },
}

define_traversal! {
    node: DataExpr,
    kind: DataExprKind,
    children: |recurse| {
        DataExprKind::Application { function, arguments } => {
            recurse(function)?;
            for argument in arguments {
                recurse(argument)?;
            }
        }
        DataExprKind::List(exprs) | DataExprKind::Set(exprs) => {
            for expr in exprs {
                recurse(expr)?;
            }
        }
        DataExprKind::Bag(elements) => {
            for element in elements {
                let BagElement { expr, multiplicity } = element;
                recurse(expr)?;
                recurse(multiplicity)?;
            }
        }
        DataExprKind::SetBagComp { predicate, .. } => {
            recurse(predicate)?;
        }
        DataExprKind::Lambda { body, .. } | DataExprKind::Quantifier { body, .. } => {
            recurse(body)?;
        }
        DataExprKind::Unary { expr, .. } => {
            recurse(expr)?;
        }
        DataExprKind::Binary { lhs, rhs, .. } => {
            recurse(lhs)?;
            recurse(rhs)?;
        }
        DataExprKind::Whr { expr, assignments } => {
            recurse(expr)?;
            for assignment in assignments {
                let Spanned {
                    node: AssignmentData { expr: value, .. },
                    ..
                } = assignment;
                recurse(value)?;
            }
        }
        DataExprKind::Id(_)
        | DataExprKind::Resolved(_, _)
        | DataExprKind::Number(_)
        | DataExprKind::Bool(_)
        | DataExprKind::EmptyList
        | DataExprKind::EmptySet
        | DataExprKind::EmptyBag => {}
    },
    // The update of a function update sits behind a `Box`, which match ergonomics do not see
    // through, so its two children have to be reached by an explicit dereference.
    shared_only: {
        DataExprKind::FunctionUpdate { expr, update } => {
            recurse(expr)?;
            let DataExprUpdate { expr: index, update: value } = &**update;
            recurse(index)?;
            recurse(value)?;
        }
    },
    mut_only: {
        DataExprKind::FunctionUpdate { expr, update } => {
            recurse(expr)?;
            let DataExprUpdate { expr: index, update: value } = &mut **update;
            recurse(index)?;
            recurse(value)?;
        }
    },
}

define_traversal! {
    node: ProcessExpr,
    kind: ProcessExprKind,
    children: |recurse| {
        ProcessExprKind::Sum { operand, .. }
        | ProcessExprKind::Dist { operand, .. }
        | ProcessExprKind::Hide { operand, .. }
        | ProcessExprKind::Rename { operand, .. }
        | ProcessExprKind::Allow { operand, .. }
        | ProcessExprKind::Block { operand, .. }
        | ProcessExprKind::Comm { operand, .. } => {
            recurse(operand)?;
        }
        ProcessExprKind::Binary { lhs, rhs, .. } => {
            recurse(lhs)?;
            recurse(rhs)?;
        }
        ProcessExprKind::Condition { then, else_, .. } => {
            recurse(then)?;
            if let Some(operand) = else_ {
                recurse(operand)?;
            }
        }
        ProcessExprKind::At { expr, .. } => {
            recurse(expr)?;
        }
        ProcessExprKind::Id(_, _)
        | ProcessExprKind::Action(_, _)
        | ProcessExprKind::Delta
        | ProcessExprKind::Tau => {}
    },
}

define_traversal! {
    node: StateFrm,
    kind: StateFrmKind,
    children: |recurse| {
        StateFrmKind::Binary { lhs, rhs, .. } => {
            recurse(lhs)?;
            recurse(rhs)?;
        }
        StateFrmKind::Unary { expr, .. } | StateFrmKind::Modality { expr, .. } => {
            recurse(expr)?;
        }
        StateFrmKind::FixedPoint { body, .. }
        | StateFrmKind::Bound { body, .. }
        | StateFrmKind::Quantifier { body, .. } => {
            recurse(body)?;
        }
        StateFrmKind::DataValExprRightMult(expr, _data_val) => {
            recurse(expr)?;
        }
        StateFrmKind::DataValExprLeftMult(_data_val, expr) => {
            recurse(expr)?;
        }
        StateFrmKind::True
        | StateFrmKind::False
        | StateFrmKind::Delay(_)
        | StateFrmKind::Yaled(_)
        | StateFrmKind::Id(_, _)
        | StateFrmKind::Resolved(_, _, _)
        | StateFrmKind::DataValExpr(_) => {}
    },
    // A modality's own `expr` is a `StateFrm` (handled above, same as every other child); its
    // `formula` is a `RegFrm`, a different node type entirely, only reachable through `visit_mixed`.
    mixed: {
        StateFrmKind::Modality { formula, .. } => {
            recurse(MixedNode::RegFrm(formula));
        }
    },
}

define_traversal! {
    node: RegFrm,
    kind: RegFrmKind,
    children: |recurse| {
        RegFrmKind::Iteration(inner) | RegFrmKind::Plus(inner) => {
            recurse(inner)?;
        }
        RegFrmKind::Sequence { lhs, rhs } | RegFrmKind::Choice { lhs, rhs } => {
            recurse(lhs)?;
            recurse(rhs)?;
        }
        RegFrmKind::Action(_act_frm) => {}
    },
    // An `Action`'s payload is an `ActFrm`, a different node type, only reachable through
    // `visit_mixed`; every other `RegFrm` variant's children are already `RegFrm` (handled above).
    mixed: {
        RegFrmKind::Action(act_frm) => {
            recurse(MixedNode::ActFrm(act_frm));
        }
    },
}

define_traversal! {
    node: ActFrm,
    kind: ActFrmKind,
    children: |recurse| {
        ActFrmKind::Negation(inner) => {
            recurse(inner)?;
        }
        ActFrmKind::Quantifier { body, .. } => {
            recurse(body)?;
        }
        ActFrmKind::Binary { lhs, rhs, .. } => {
            recurse(lhs)?;
            recurse(rhs)?;
        }
        ActFrmKind::At { expr, .. } => {
            recurse(expr)?;
        }
        ActFrmKind::True | ActFrmKind::False | ActFrmKind::MultAct(_) | ActFrmKind::DataExprVal(_) => {}
    },
}

define_traversal! {
    node: PbesExpr,
    kind: PbesExprKind,
    children: |recurse| {
        PbesExprKind::Quantifier { body, .. } => {
            recurse(body)?;
        }
        PbesExprKind::Negation(inner) => {
            recurse(inner)?;
        }
        PbesExprKind::Binary { lhs, rhs, .. } => {
            recurse(lhs)?;
            recurse(rhs)?;
        }
        PbesExprKind::DataValExpr(_)
        | PbesExprKind::PropVarInst(_)
        | PbesExprKind::True
        | PbesExprKind::False => {}
    },
}

define_traversal! {
    node: PresExpr,
    kind: PresExprKind,
    children: |recurse| {
        PresExprKind::RightConstantMultiply { expr, .. }
        | PresExprKind::LeftConstantMultiply { expr, .. }
        | PresExprKind::Bound { expr, .. } => {
            recurse(expr)?;
        }
        PresExprKind::Equal { body, .. } => {
            recurse(body)?;
        }
        PresExprKind::Condition { lhs, then, else_, .. } => {
            recurse(lhs)?;
            recurse(then)?;
            recurse(else_)?;
        }
        PresExprKind::Negation(inner) => {
            recurse(inner)?;
        }
        PresExprKind::Binary { lhs, rhs, .. } => {
            recurse(lhs)?;
            recurse(rhs)?;
        }
        PresExprKind::DataValExpr(_)
        | PresExprKind::PropVarInst(_)
        | PresExprKind::True
        | PresExprKind::False => {}
    },
}
