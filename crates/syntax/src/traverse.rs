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

/// The shared step of [Traverse::visit_mixed]/[Traverse::try_visit_mixed]: appends `node`'s
/// children to `stack`, in the order they should be visited in.
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
    // Reproduces the same "push in written order, then reverse so the LIFO stack pops them back
    // out in that order" trick every other stack-based walk in this file uses.
    stack.extend(children.into_iter().rev());
}

/// A syntax tree node whose children are the same type, traversed top-down.
///
/// [Traverse::push_children]/[Traverse::push_children_mut] define a node type's direct children;
/// everything else (iteration, early exit, errors, context, substitution) is shared here. Walks use
/// an explicit heap stack rather than native recursion, so depth is bounded by memory, not the call
/// stack (see `stack_depth_probe` in this module's tests). [Traverse::try_transform] does the same
/// for bottom-up rewriting via two worklists of owned nodes instead, since a bottom-up rewrite needs
/// a node's children finished before the node itself, which the borrow-based walk above can't do for
/// a `Box`/`Vec`-owned tree.
///
/// None of this crosses into a different node type on its own (e.g. a [StateFrm] into the [RegFrm]
/// of a `Modality`). [Traverse::visit_mixed]/[Traverse::try_visit_mixed] do cross node types in one
/// walk (see [MixedNode]). [Traverse::visit_subtree_scoped]/[Traverse::apply_subtree_scoped] add an
/// exit hook alongside [Visit]'s entry callback, for scoped mutable state that must be popped once a
/// node's whole subtree, not just the node, is done.
pub trait Traverse: Sized {
    /// Appends each direct child of this node to `stack`, paired with `context`, in written order.
    ///
    /// The only part of the traversal that knows a node's shape; [Self::drive] and
    /// [Self::visit_subtree] drain the stack generically.
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

    /// See [Traverse::apply_children]; this variant rewrites each child bottom-up.
    ///
    /// Built on [Traverse::try_transform], one call per direct child, so this adds no native
    /// recursion of its own — call depth is bounded by branching factor, not tree depth.
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

    /// Wraps `self` in the [MixedNode] variant naming its own type, so [Traverse::visit_mixed] can
    /// hold nodes of every traversable type on one stack.
    fn as_mixed(&self) -> MixedNode<'_>;

    /// Appends this node's children of a *different* [Traverse] type to `sink`, for
    /// [Traverse::visit_mixed] to follow. Defaults to nothing; only [StateFrm] and [RegFrm]
    /// override it.
    fn push_mixed_children<'a>(&'a self, _sink: &mut Vec<MixedNode<'a>>) {}

    /// Drains an explicit stack of pending `(node, context)` pairs depth-first, in the order they
    /// would be visited by native pre-order recursion — the shared core of [Self::visit_subtree]
    /// and [Self::apply_subtree], parameterized only by how a single node is handled.
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
                // `Step::Replace` is uninhabited for a read-only walk, which is how it rules
                // substitution out without a second callback type; a mutating walk replaces the
                // node itself inside `step` and never calls back in here for it.
                Ok(ControlFlow::Continue(Step::Replace(replacement))) => match replacement {},
                Ok(ControlFlow::Continue(Step::Into(context))) => context,
            };
            let start = stack.len();
            let _ = node.push_children_mut(context, &mut stack);
            stack[start..].reverse();
        }
        ControlFlow::Continue(())
    }

    /// Visits this node and then, unless the callback breaks or prunes, its children.
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

    /// See [Traverse::visit_subtree]; additionally calls `exit` right after a node's children have
    /// all been visited — the enter/exit pair a plain [Visit] callback cannot express, for scoped
    /// mutable `state` that must be popped once a node's whole subtree is done, not merely once
    /// `enter` returns.
    ///
    /// `enter` and `exit` are separate `FnMut`s (so each can borrow `state` independently) that
    /// typically re-match the same node kinds, pushing or popping only where scoping is needed.
    /// `exit` always gets the same `context` `enter` was given, and still runs — innermost first —
    /// for a pruned node or one still open when the walk stops early, so scoped state never dangles.
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

    /// See [Traverse::visit_subtree_scoped]; the ergonomic top-level entry point, mirroring
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

    /// The mutating counterpart of [Traverse::visit_subtree_scoped], for callers that must rewrite
    /// nodes in place while still needing the enter/exit split for scoped state.
    ///
    /// Unlike [Traverse::visit_subtree_scoped], `exit` does not receive the node it is un-scoping —
    /// holding a reference to it across the later `&mut` borrow used to reach its children would
    /// alias. `exit` gets only `context` and `state`; a caller whose `exit` needs to know what kind
    /// of node it's un-scoping has `enter` push that onto `state` for `exit` to pop.
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

    /// See [Traverse::apply_subtree_scoped]; the ergonomic top-level entry point, mirroring
    /// [Traverse::visit_scoped]/[Traverse::apply_with].
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

    /// See [Traverse::visit_subtree]; a replaced node is not descended into.
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

    /// Visits `self` and its subtree, crossing into a different [Traverse] type wherever
    /// [Traverse::push_mixed_children] reports one (e.g. [StateFrm] → [RegFrm] → [ActFrm]) — unlike
    /// every other traversal here, which stays within a single node type.
    ///
    /// The callback receives a [MixedNode]. There is no shared `context`, [Step::Prune], or
    /// [Step::Replace]: those need a `Copy` context shared across every node type, which a walk
    /// spanning several unrelated types can't offer without type erasure.
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

    /// Rewrites this node and its subtree bottom-up: children are rewritten before the node itself,
    /// so the callback always sees final children — the counterpart of [Traverse::apply_mut]
    /// (top-down). The callback rewrites through `&mut`, so nothing is cloned.
    ///
    /// A bottom-up rewrite needs a node's children finished before the node, which the borrow-based
    /// stack in [Traverse::apply_subtree] can't do for a `Box`/`Vec`-owned tree. This instead moves
    /// *owned* nodes between two worklists: `stack` holds `Enter`/`Assemble` frames and `results`
    /// accumulates finished nodes. `Enter(node)` detaches and pushes its children (via
    /// [std::mem::take]), then an `Assemble` frame to run once they're done; `Assemble` pulls its
    /// node's children off the end of `results`, puts them back, calls `function`, and pushes the
    /// result.
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
        match self.try_transform::<Infallible, _>(&mut |node| Ok(function(node))) {
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
/// `kind: $Kind` also generates `impl TakeRecursiveChildren for $Kind`, reusing the same arms to fix
/// a related but distinct recursion issue (see [TakeRecursiveChildren]'s own doc comment); it needs
/// `$Kind: Default` to detach a child cheaply.
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
                // The shared `$child`/`$mut_child` arms end each of their statements with `?`,
                // which needs an enclosing function whose return type supports it; wrapped here so
                // the trait method itself can keep the plain `()` its default impl already has.
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
                // See the other arm's identical block for why this is wrapped in `inner`.
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
        | DataExprKind::Resolved(_, _, _)
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

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::ops::ControlFlow;

    use merc_utilities::Step;

    use crate::ActFrm;
    use crate::ActFrmKind;
    use crate::DataExpr;
    use crate::DataExprKind;
    use crate::PbesExprKind;
    use crate::PresExprKind;
    use crate::ProcessExprKind;
    use crate::RegFrm;
    use crate::RegFrmKind;
    use crate::SortExpression;
    use crate::SortExpressionKind;
    use crate::StateFrm;
    use crate::StateFrmKind;
    use crate::Traverse;
    use crate::UntypedDataSpecification;
    use crate::UntypedPbes;
    use crate::UntypedPres;
    use crate::UntypedProcessSpecification;
    use crate::UntypedStateFrmSpec;
    use crate::traverse::MixedNode;
    use crate::traverse::Recursion;

    /// Parses a state formula, for example `mu X. [a]X`.
    fn state_formula(input: &str) -> StateFrm {
        UntypedStateFrmSpec::parse(input)
            .expect("the state formula should parse")
            .formula
    }

    /// Parses a regular formula by putting it inside a modality, for example `a . b*`.
    fn regular_formula(input: &str) -> RegFrm {
        let formula = state_formula(&format!("[{input}]true"));
        match formula.into_node() {
            StateFrmKind::Modality { formula, .. } => formula,
            _ => panic!("expected a modality"),
        }
    }

    /// Parses an action formula, for example `a && b`.
    fn action_formula(input: &str) -> ActFrm {
        match regular_formula(input).into_node() {
            RegFrmKind::Action(act_frm) => act_frm,
            _ => panic!("expected an action formula"),
        }
    }

    /// Parses a sort expression by declaring it as an alias, for example `A # B -> C`.
    fn sort_expression(input: &str) -> SortExpression {
        UntypedDataSpecification::parse(&format!("sort S = {input};"))
            .expect("the sort expression should parse")
            .sort_declarations
            .remove(0)
            .expr
            .expect("the declaration is an alias")
    }

    /// Collects the identifiers of a state formula in the order in which they are visited.
    fn identifiers(formula: &StateFrm) -> Vec<String> {
        let mut result = Vec::new();

        formula.visit::<(), _>(|formula| {
            if let StateFrmKind::Id(name, _) = &formula.node {
                result.push(name.node.clone());
            }

            ControlFlow::Continue(())
        });

        result
    }

    #[test]
    fn test_visit_is_top_down_and_left_to_right() {
        let formula = state_formula("mu X. [a]X && mu Y. Y && Z");

        assert_eq!(identifiers(&formula), ["X", "Y", "Z"]);
    }

    #[test]
    fn test_visit_state_formula_breaks_from_nested_node() {
        // `Z` only occurs below the top-level conjunction.
        let formula = state_formula("true && (mu X. (X && Z))");

        let found = formula.visit(|formula| match &formula.node {
            StateFrmKind::Id(name, _) if name.node == "Z" => ControlFlow::Break(name.node.clone()),
            _ => ControlFlow::Continue(()),
        });

        assert_eq!(found.as_deref(), Some("Z"));
    }

    #[test]
    fn test_visit_regular_formula_breaks_from_nested_node() {
        let formula = regular_formula("a . (b* + c)");

        let found = formula.visit(|formula| match &formula.node {
            RegFrmKind::Iteration(_) => ControlFlow::Break("iteration"),
            _ => ControlFlow::Continue(()),
        });

        assert_eq!(found, Some("iteration"));
    }

    #[test]
    fn test_visit_action_formula_breaks_from_nested_node() {
        let formula = action_formula("a && (b || !c)");

        let found = formula.visit(|formula| match &formula.node {
            ActFrmKind::Negation(_) => ControlFlow::Break("negation"),
            _ => ControlFlow::Continue(()),
        });

        assert_eq!(found, Some("negation"));
    }

    #[test]
    fn test_visit_sort_expression_breaks_from_nested_node() {
        let sort = sort_expression("A # List(B) -> C");

        let found = sort.visit(|sort| match &sort.node {
            SortExpressionKind::Reference(name) if name == "B" => ControlFlow::Break(name.clone()),
            _ => ControlFlow::Continue(()),
        });

        assert_eq!(found.as_deref(), Some("B"));
    }

    #[test]
    fn test_visit_data_expression_breaks_from_nested_node() {
        let expr = DataExpr::parse("f(g(a), b)").expect("the data expression should parse");

        let found = expr.visit(|expr| match &expr.node {
            DataExprKind::Id(name) if name == "a" => ControlFlow::Break(name.clone()),
            _ => ControlFlow::Continue(()),
        });

        assert_eq!(found.as_deref(), Some("a"));
    }

    /// The children of a function update sit behind a `Box`, which the traversal has to reach
    /// through explicitly.
    #[test]
    fn test_visit_data_expression_descends_into_function_update() {
        let expr = DataExpr::parse("f[a -> b]").expect("the data expression should parse");

        let mut names = Vec::new();
        expr.visit::<(), _>(|expr| {
            if let DataExprKind::Id(name) = &expr.node {
                names.push(name.clone());
            }

            ControlFlow::Continue(())
        });

        assert_eq!(names, ["f", "a", "b"]);
    }

    #[test]
    fn test_visit_process_expression_breaks_from_nested_node() {
        let spec = UntypedProcessSpecification::parse("init a . (sum n: Nat . b(n)) + delta;")
            .expect("the process specification should parse");
        let process = spec.init.expect("the specification has an initial process");

        let found = process.visit(|process| match &process.node {
            ProcessExprKind::Delta => ControlFlow::Break("delta"),
            _ => ControlFlow::Continue(()),
        });

        assert_eq!(found, Some("delta"));
    }

    #[test]
    fn test_visit_pbes_expression_breaks_from_nested_node() {
        let pbes = UntypedPbes::parse("pbes mu X = forall n: Nat . (val(n < 3) => !X); init X;")
            .expect("the PBES should parse");

        let found = pbes.equations[0].formula.visit(|expr| match &expr.node {
            PbesExprKind::Negation(_) => ControlFlow::Break("negation"),
            _ => ControlFlow::Continue(()),
        });

        assert_eq!(found, Some("negation"));
    }

    #[test]
    fn test_visit_pres_expression_breaks_from_nested_node() {
        let pres =
            UntypedPres::parse("pres mu X = sup n: Nat . (val(n < 3) + X); init X;").expect("the PRES should parse");

        // `X` is only reachable through the bound and the addition below it.
        let found = pres.equations[0].formula.visit(|expr| match &expr.node {
            PresExprKind::PropVarInst(instantiation) => ControlFlow::Break(instantiation.identifier.node.clone()),
            _ => ControlFlow::Continue(()),
        });

        assert_eq!(found.as_deref(), Some("X"));
    }

    #[test]
    fn test_visit_prune_skips_the_children() {
        let formula = state_formula("true && (mu X. (X && Z))");

        // Everything below the fixpoint is skipped, so `Z` is never reached.
        let found = formula.visit_with::<(), String, Infallible, _>((), |formula, context| {
            Ok(match &formula.node {
                StateFrmKind::Id(name, _) if name.node == "Z" => ControlFlow::Break(name.node.clone()),
                StateFrmKind::FixedPoint { .. } => ControlFlow::Continue(Step::Prune),
                _ => ControlFlow::Continue(Step::Into(context)),
            })
        });

        assert_eq!(found, Ok(None));
    }

    #[test]
    fn test_visit_threads_the_context() {
        let formula = state_formula("true && (mu X. (X && Z))");

        // The context is the depth of the node, which is one more than that of its parent.
        let mut depths = Vec::new();
        let found = formula.visit_with::<usize, Infallible, Infallible, _>(0, |formula, depth| {
            if let StateFrmKind::Id(name, _) = &formula.node {
                depths.push((name.node.clone(), depth));
            }

            Ok(ControlFlow::Continue(Step::Into(depth + 1)))
        });

        assert_eq!(found, Ok(None));
        assert_eq!(depths, [("X".to_string(), 3), ("Z".to_string(), 3)]);
    }

    #[test]
    fn test_visit_reports_the_error_of_the_callback() {
        let formula = state_formula("mu X. X");

        let result: Result<Option<Infallible>, &str> = formula.try_visit(|_formula| Err("failed"));

        assert_eq!(result, Err("failed"));
    }

    #[test]
    fn test_apply_collects_variables() {
        let formula = state_formula("mu X. [a]X && mu X. X && Y");

        let mut variables = Vec::new();
        let result = formula.apply::<Infallible, _>(|formula| {
            if let StateFrmKind::Id(name, _) = &formula.node {
                variables.push(name.node.clone());
            }

            Ok(None)
        });

        assert!(result.is_ok());
        assert_eq!(variables, ["X", "X", "Y"]);
    }

    #[test]
    fn test_apply_without_replacement_is_the_identity() {
        for input in [
            "mu X. [a . b*]X && nu Y. <c>Y",
            "forall n: Nat . val(n < 3) => [a(n)]false",
            "true && (mu X. (X && Z))",
        ] {
            let formula = state_formula(input);

            let result = formula.clone().apply::<Infallible, _>(|_formula| Ok(None));

            assert_eq!(result.as_ref(), Ok(&formula));
        }
    }

    #[test]
    fn test_apply_does_not_descend_into_the_replacement() {
        let formula = state_formula("X && Y");

        // The replacement of `X` contains an `X` again, which must not be replaced a second time.
        let mut replacements = 0;
        let result = formula.apply::<Infallible, _>(|formula| {
            if let StateFrmKind::Id(name, _) = &formula.node
                && name.node == "X"
            {
                replacements += 1;
                return Ok(Some(state_formula("mu X0. X")));
            }

            Ok(None)
        });

        assert_eq!(replacements, 1);
        assert_eq!(
            format!("{}", result.expect("the callback cannot fail")),
            "((mu X0 . X) && Y)"
        );
    }

    #[test]
    fn test_apply_with_breaks_and_keeps_what_was_rewritten() {
        let mut formula = state_formula("X && Y");

        let found = formula.apply_with::<(), &str, Infallible, _>((), |formula, context| {
            Ok(match &formula.node {
                StateFrmKind::Id(name, _) if name.node == "X" => {
                    ControlFlow::Continue(Step::Replace(StateFrmKind::True.into()))
                }
                StateFrmKind::Id(name, _) if name.node == "Y" => ControlFlow::Break("stopped"),
                _ => ControlFlow::Continue(Step::Into(context)),
            })
        });

        assert_eq!(found, Ok(Some("stopped")));
        assert_eq!(format!("{formula}"), "(true && Y)");
    }

    #[test]
    fn test_visit_children_reaches_every_child() {
        let formula = state_formula("[a]X && (nu Z0. Z0)");

        // Pruning each child keeps only the direct children of the conjunction.
        let mut children = Vec::new();
        let outcome: Recursion<Infallible, Infallible> = formula.visit_children((), &mut |child, _context| {
            children.push(format!("{child}"));
            Ok(ControlFlow::Continue(Step::Prune))
        });

        assert!(matches!(outcome, ControlFlow::Continue(())));
        assert_eq!(children, ["[a]X", "(nu Z0 . Z0)"]);
    }

    /// A plain `.visit()` over a [StateFrm] cannot see the actions inside a modality's regular
    /// formula: `RegFrm`/`ActFrm` are a different node type. `visit_mixed` is the traversal that
    /// crosses into them.
    #[test]
    fn test_visit_mixed_crosses_from_state_formula_into_its_regular_and_action_formulas() {
        let formula = state_formula("[a . b*]X");

        let mut labels = Vec::new();
        formula.visit_mixed::<Infallible>(|node| {
            labels.push(match node {
                MixedNode::StateFrm(_) => "state",
                MixedNode::RegFrm(_) => "reg",
                MixedNode::ActFrm(_) => "act",
                _ => "other",
            });
            ControlFlow::Continue(())
        });

        // Pre-order, left to right, and crossing every boundary: the modality itself, then its
        // whole `RegFrm` subtree (`a . b*`, i.e. `Sequence(Action(a), Iteration(Action(b)))`, each
        // `Action` node's `ActFrm` payload included), and only then the modality's own `StateFrm`
        // child `X` -- not reachable at all without the crossing.
        assert_eq!(labels, ["state", "reg", "reg", "act", "reg", "reg", "act", "state"]);
    }

    #[test]
    fn test_try_visit_mixed_reports_the_error_of_the_callback() {
        let formula = state_formula("[a]true");

        let result: Result<Option<Infallible>, &str> = formula.try_visit_mixed(|_node| Err("failed"));

        assert_eq!(result, Err("failed"));
    }

    /// `state_vars` grows for a `FixedPoint`'s body and shrinks once that body is visited, so an
    /// inner fixpoint's variable is invisible outside it, including to a sibling formula.
    #[test]
    fn test_visit_scoped_pushes_and_pops_fixpoint_variables_like_a_real_checker() {
        let formula = state_formula("mu X. (mu Y. X) && Z");

        let mut state_vars: Vec<String> = Vec::new();
        let mut scope_at_each_id = Vec::new();

        let result = formula.visit_scoped::<(), Vec<String>, Infallible, Infallible, _, _>(
            (),
            &mut state_vars,
            |formula, context, state_vars| {
                if let StateFrmKind::FixedPoint { variable, .. } = &formula.node {
                    state_vars.push(variable.identifier.node.clone());
                }
                if let StateFrmKind::Id(name, _) = &formula.node {
                    scope_at_each_id.push((name.node.clone(), state_vars.clone()));
                }
                Ok(ControlFlow::Continue(Step::Into(context)))
            },
            |formula, _context, state_vars| {
                if let StateFrmKind::FixedPoint { .. } = &formula.node {
                    state_vars.pop();
                }
            },
        );

        assert_eq!(result, Ok(None));
        // `X` is visited while both `mu X` and `mu Y` are open; `Z` only while `mu X` still is,
        // `mu Y` (and its own variable) having already been exited by the time its sibling `&&`
        // operand is reached.
        assert_eq!(
            scope_at_each_id,
            [
                ("X".to_string(), vec!["X".to_string(), "Y".to_string()]),
                ("Z".to_string(), vec!["X".to_string()]),
            ]
        );
        assert!(state_vars.is_empty());
    }

    /// The scoped state may borrow from the tree being walked (`&str` names here), which a callback
    /// bound over an anonymous `&Self` lifetime would reject.
    #[test]
    fn test_visit_scoped_state_can_borrow_from_the_tree() {
        let formula = state_formula("mu X. (mu Y. X) && Z");

        let mut state_vars: Vec<&str> = Vec::new();
        let mut scope_at_each_id: Vec<(&str, Vec<&str>)> = Vec::new();

        let result = formula.visit_scoped::<(), Vec<&str>, Infallible, Infallible, _, _>(
            (),
            &mut state_vars,
            |formula, context, state_vars| {
                if let StateFrmKind::FixedPoint { variable, .. } = &formula.node {
                    state_vars.push(variable.identifier.node.as_str());
                }
                if let StateFrmKind::Id(name, _) = &formula.node {
                    scope_at_each_id.push((name.node.as_str(), state_vars.clone()));
                }
                Ok(ControlFlow::Continue(Step::Into(context)))
            },
            |formula, _context, state_vars| {
                if let StateFrmKind::FixedPoint { .. } = &formula.node {
                    state_vars.pop();
                }
            },
        );

        assert_eq!(result, Ok(None));
        assert_eq!(scope_at_each_id, [("X", vec!["X", "Y"]), ("Z", vec!["X"])]);
    }

    /// `exit` must still run, innermost first, for every scope still open when the walk breaks
    /// early, or scoped state is left corrupted.
    #[test]
    fn test_visit_scoped_unwinds_open_scopes_on_break() {
        let formula = state_formula("mu X. (mu Y. Z)");

        let mut state_vars: Vec<String> = Vec::new();
        let mut exited_in_order = Vec::new();

        let found = formula.visit_scoped::<(), Vec<String>, &str, Infallible, _, _>(
            (),
            &mut state_vars,
            |formula, context, state_vars| {
                if let StateFrmKind::FixedPoint { variable, .. } = &formula.node {
                    state_vars.push(variable.identifier.node.clone());
                }
                if let StateFrmKind::Id(name, _) = &formula.node
                    && name.node == "Z"
                {
                    return Ok(ControlFlow::Break("stopped"));
                }
                Ok(ControlFlow::Continue(Step::Into(context)))
            },
            |formula, _context, state_vars| {
                if let StateFrmKind::FixedPoint { variable, .. } = &formula.node {
                    exited_in_order.push(variable.identifier.node.clone());
                    state_vars.pop();
                }
            },
        );

        assert_eq!(found, Ok(Some("stopped")));
        assert_eq!(exited_in_order, ["Y", "X"]);
        assert!(state_vars.is_empty());
    }

    #[test]
    fn test_transform_visits_bottom_up_left_to_right() {
        // A leaf's own children (none) finish before it does, each operand of `&&` finishes before
        // the `&&` node itself, and the left operand before the right one.
        let mut formula = state_formula("X && Y");

        let mut order = Vec::new();
        formula.transform(|formula| match &formula.node {
            StateFrmKind::Id(name, _) => order.push(name.node.clone()),
            StateFrmKind::Binary { .. } => order.push("&&".to_string()),
            _ => {}
        });

        assert_eq!(order, ["X", "Y", "&&"]);
    }

    #[test]
    fn test_transform_no_op_reassembles_the_exact_same_tree() {
        // Exercises the take-apart/reassemble round trip across every shape in one formula: a
        // `FixedPoint`'s own variable, a `Modality`'s `RegFrm`, a `Quantifier` binder, and a
        // `Binary` -- if any child landed back in the wrong slot, or the wrong number of results
        // were consumed for some node, this would no longer structurally equal the original
        // (`Spanned`'s `PartialEq` compares only `.node`, so the placeholder spans transform
        // briefly leaves behind can never hide a misplaced *value*).
        let text = "mu X. [a](exists n: Nat . val(n == n) && X)";
        let original = state_formula(text);
        let mut formula = state_formula(text);

        formula.transform(|_| {});

        assert_eq!(formula, original);
    }

    #[test]
    fn test_transform_rewrites_every_node_including_the_root() {
        let mut formula = state_formula("X && Y");

        let mut count = 0usize;
        formula.transform(|_| count += 1);

        // `X`, `Y`, and the `&&` node itself: `transform` (unlike `transform_children`) also
        // rewrites the root.
        assert_eq!(count, 3);
    }

    #[test]
    fn test_transform_children_rewrites_children_but_not_the_root() {
        let mut formula = state_formula("X && Y");

        let mut count = 0usize;
        let result: Result<(), Infallible> = formula.transform_children(&mut |_| {
            count += 1;
            Ok(())
        });

        assert!(result.is_ok());
        assert_eq!(count, 2); // `X` and `Y`, not the `&&` node itself.
    }

    #[test]
    fn test_try_transform_propagates_an_error_without_rewriting_further() {
        let mut formula = state_formula("X && Y");

        let mut visited = Vec::new();
        let result: Result<(), &str> = formula.try_transform(&mut |formula| {
            if let StateFrmKind::Id(name, _) = &formula.node {
                visited.push(name.node.clone());
                if name.node == "Y" {
                    return Err("stopped at Y");
                }
            }
            Ok(())
        });

        assert_eq!(result, Err("stopped at Y"));
        // `X` (whose own subtree finished first) was rewritten before the error on `Y` stopped the
        // walk; the enclosing `&&` never was, since it comes after both operands in bottom-up order.
        assert_eq!(visited, ["X", "Y"]);
    }
}
