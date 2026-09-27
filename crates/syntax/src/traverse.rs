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

/// The outcome of descending into a subtree: `Continue(())` when the whole subtree was traversed,
/// and `Break(Ok(value))` / `Break(Err(error))` when the traversal stopped early.
///
/// Both interruptions are carried in the break arm so that a recursive step can propagate them
/// with a single `?`, which is what keeps the generated recursion free of the per-child
/// `if let Some(result) = ... { return }` boilerplate that a hand-written traversal repeats once
/// per variant.
pub type Recursion<T, E> = ControlFlow<Result<T, E>, ()>;

/// A node from any of this crate's traversable trees, named by which one it is.
///
/// [Traverse::visit_mixed] holds these on a single stack to walk across the boundary between node
/// types that every other method on [Traverse] stays within by design (see the trait's own doc
/// comment). Cheap to pass around: every variant is a plain reference.
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
///
/// A node's foreign-type children ([Traverse::push_mixed_children]) are listed before its
/// same-type ones ([Traverse::push_children], called with a throwaway `()` context since a mixed
/// walk has no shared context type to thread across node types). That ordering happens to match
/// source order for the one node currently wired with both kinds of child in the same place
/// (`StateFrmKind::Modality`'s `formula: RegFrm` precedes its `expr: Box<StateFrm>`, i.e.
/// `[formula]expr`); see [Traverse::push_mixed_children]'s own doc comment.
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

/// A syntax tree node whose children are of its own type, traversed top-down.
///
/// The traversal is defined once per node type by [Traverse::push_children] and
/// [Traverse::push_children_mut], which enqueue a node's direct children and nothing else;
/// deciding what to do with a node is entirely up to the callback, which pattern matches on it.
/// Everything else — early exit, errors, context threading, substitution, and the descent itself —
/// is provided here and is therefore identical for every node type.
///
/// [Traverse::visit_subtree]/[Traverse::apply_subtree] (and everything built on them: `visit`,
/// `try_visit`, `visit_with`, `visit_children`, `apply`, `apply_mut`, `apply_with`,
/// `apply_children`) walk with an explicit, heap-allocated stack rather than native recursion, so
/// a tree's depth is bounded only by available memory, not the call stack — see `stack_depth_probe`
/// in this module's tests for a 100,000-deep regression case per node type.
/// [Traverse::transform_children] (and `try_transform`/`transform`) do **not** share this: a
/// bottom-up rewrite needs a node's children fully processed *before* touching the node itself,
/// which — for an owned, `Box`/`Vec`-based tree like this one, as opposed to an arena addressed by
/// index — means holding a live mutable borrow of the already-processed children at the same time
/// as the borrow needed to reach the parent again, which the borrow checker rejects (there is no
/// such conflict top-down: once a node has been visited/replaced, its old borrow is dropped before
/// its children are ever reached). Making that iterative too needs a genuinely different
/// representation (an arena, or owned nodes moved in and out of a worklist behind a placeholder),
/// not just this same worklist trick; it remains recursive, and is a known, separate gap in
/// covering the review finding: `crates/typecheck`'s `.transform`/`.try_transform` call sites
/// (`ir/desugar.rs`, `ir/lower.rs`, `resolution/name_resolution.rs`) are not stack-safe for a
/// pathologically deep tree.
///
/// None of the above crosses into a *different* node type on its own: a [StateFrm] traversal does
/// not, by itself, descend into the [RegFrm] of a `Modality` or the [ActFrm] inside that. Two more
/// pieces close gaps that used to force a caller to work around this trait rather than through it:
///
/// - [Traverse::visit_mixed]/[Traverse::try_visit_mixed] *do* cross that boundary, visiting every
///   node of every type reachable from `self` (see [MixedNode]) in one walk, for callbacks that
///   need to see a whole formula regardless of which node type each part of it lives in without
///   hand-rolling one recursive function per type and wiring the crossings between them, the way
///   e.g. `crates/typecheck`'s `modal::check::{collect_scope, collect_scope_regfrm,
///   collect_scope_actfrm}` currently do.
/// - [Traverse::visit_subtree_scoped]/[Traverse::visit_scoped] add the enter-*and*-exit hook that
///   [Visit]'s single pre-order callback cannot express, for a caller that pushes genuinely scoped
///   mutable state on entering a node (e.g. `check_state_formula`'s `state_vars`, pushed for a
///   `mu`/`nu` binder's body) and needs it popped again once that node's subtree, not just the
///   node itself, is done being visited.
///
/// Neither of these migrates the hand-written checkers named above -- doing so is real, separate
/// follow-up work, since they thread several more parameters than a `Traverse` callback's `context`
/// slot has room for -- they only remove the two capability gaps that were blocking it; see
/// `review/stack-overflow-recursion.md`. A single-type traversal still does not cross node types on
/// its own outside of `visit_mixed`, and everywhere but the one node currently wired for it,
/// nesting a traversal explicitly at the point it's needed is still exactly how to do it.
pub trait Traverse: Sized {
    /// Appends each direct child of this node to `stack`, paired with `context`, in the order in
    /// which they are written. Always returns `ControlFlow::Continue(())`; the return type only
    /// exists so the macro-generated body can share the same `recurse(child)?;`-per-child shape as
    /// every other method here (there is nothing to break out of or fail with while just pushing).
    ///
    /// This is the only part of the traversal that knows the shape of the node; [Self::drive] and
    /// [Self::visit_subtree] drain the resulting stack generically, so this never recurses itself.
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
    fn transform_children<E, F>(&mut self, function: &mut F) -> Result<(), E>
    where
        F: FnMut(&mut Self) -> Result<(), E>;

    /// Wraps `self` in the [MixedNode] variant naming its own type, so [Traverse::visit_mixed] can
    /// hold nodes of every traversable type on one stack.
    fn as_mixed(&self) -> MixedNode<'_>;

    /// Appends this node's children that are of a *different* [Traverse] type to `sink`, in the
    /// order they are written -- the crossing [Traverse::push_children] deliberately does not make
    /// (see the trait's own doc comment) -- for [Traverse::visit_mixed] to follow.
    ///
    /// The default pushes nothing, correct for a node type with no such crossing. Only
    /// [StateFrm] (into its `Modality`'s [RegFrm]) and [RegFrm] (into an `Action`'s [ActFrm])
    /// currently override it; see `review/stack-overflow-recursion.md` for what descending further
    /// (e.g. into the [DataExpr] inside an `ActFrm::DataExprVal`, or the `val(...)` of a
    /// `StateFrmKind::DataValExpr`) would take.
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
    fn visit_subtree<C, T, E, F>(&self, context: C, function: &mut F) -> Recursion<T, E>
    where
        C: Copy,
        F: FnMut(&Self, C) -> Visit<Infallible, C, T, E>,
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

    /// See [Traverse::visit_subtree]; additionally calls `exit` on a node right after all of its
    /// children have finished being visited -- the "children are done, un-scope now" hook a plain
    /// [Visit] callback cannot express, for a caller that pushes scoped mutable state in `enter`
    /// (e.g. a `mu`/`nu` binder's own variable, as `check_state_formula`'s `state_vars` does) and
    /// must pop it again once the node's whole subtree is done, not merely once `enter` returns.
    ///
    /// The scoped state itself is `state`, threaded through as an explicit `&mut S` rather than
    /// captured by either closure: `enter` and `exit` are two separate `FnMut`s, and two closures
    /// cannot both capture the *same* variable mutably (each borrows it only for the one call it's
    /// made with here, never at the same time as the other). `enter` and `exit` typically re-match
    /// the same node kinds against `state`, each only pushing or popping for the ones that need
    /// scoping — the "push on entry, truncate on exit" pattern [Visit]'s own doc comment
    /// recommends, now with a real place to put the truncate.
    ///
    /// `exit` is always given the same `context` `enter` was for that node, not whatever `enter`
    /// returned for its children: it names the node being un-scoped, not its children's context.
    /// It still runs for a pruned node (immediately, since it has no children to wait for) and for
    /// every node still open when the walk stops early on a [ControlFlow::Break] or an error, in
    /// the same innermost-first order a normal return from the bottom of the tree would give — so
    /// a `Break`/error never leaves scoped state pushed by `enter` dangling in `state`.
    fn visit_subtree_scoped<C, S, T, E, F, G>(
        &self,
        context: C,
        state: &mut S,
        enter: &mut F,
        exit: &mut G,
    ) -> Recursion<T, E>
    where
        C: Copy,
        F: FnMut(&Self, C, &mut S) -> Visit<Infallible, C, T, E>,
        G: FnMut(&Self, C, &mut S),
    {
        enum Frame<'a, N, C> {
            Enter(&'a N, C),
            Exit(&'a N, C),
        }

        fn unwind<N, C, S>(stack: Vec<Frame<'_, N, C>>, state: &mut S, exit: &mut impl FnMut(&N, C, &mut S)) {
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
    fn visit_scoped<C, S, T, E, F, G>(
        &self,
        context: C,
        state: &mut S,
        mut enter: F,
        mut exit: G,
    ) -> Result<Option<T>, E>
    where
        C: Copy,
        F: FnMut(&Self, C, &mut S) -> Visit<Infallible, C, T, E>,
        G: FnMut(&Self, C, &mut S),
    {
        match self.visit_subtree_scoped(context, state, &mut enter, &mut exit) {
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
    fn visit_children<C, T, E, F>(&self, context: C, function: &mut F) -> Recursion<T, E>
    where
        C: Copy,
        F: FnMut(&Self, C) -> Visit<Infallible, C, T, E>,
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
    fn visit_with<C, T, E, F>(&self, context: C, mut function: F) -> Result<Option<T>, E>
    where
        C: Copy,
        F: FnMut(&Self, C) -> Visit<Infallible, C, T, E>,
    {
        match self.visit_subtree(context, &mut function) {
            ControlFlow::Break(Ok(value)) => Ok(Some(value)),
            ControlFlow::Break(Err(error)) => Err(error),
            ControlFlow::Continue(()) => Ok(None),
        }
    }

    /// See [Traverse::visit_with], for callbacks that need neither a context nor pruning.
    fn try_visit<T, E, F>(&self, mut function: F) -> Result<Option<T>, E>
    where
        F: FnMut(&Self) -> Result<ControlFlow<T>, E>,
    {
        self.visit_with((), |node, context| {
            Ok(match function(node)? {
                ControlFlow::Break(value) => ControlFlow::Break(value),
                ControlFlow::Continue(()) => ControlFlow::Continue(Step::Into(context)),
            })
        })
    }

    /// See [Traverse::try_visit], for callbacks that cannot fail.
    fn visit<T, F>(&self, mut function: F) -> Option<T>
    where
        F: FnMut(&Self) -> ControlFlow<T>,
    {
        match self.try_visit::<T, Infallible, _>(|node| Ok(function(node))) {
            Ok(result) => result,
            Err(error) => match error {},
        }
    }

    /// Visits `self` and its subtree, *crossing* into a different [Traverse] type wherever
    /// [Traverse::push_mixed_children] reports one — e.g. from a [StateFrm]'s `Modality` into its
    /// [RegFrm], and from there into the [ActFrm] of an `Action` — unlike every other traversal on
    /// this trait, which by design stays within a single node type (see the trait's own doc
    /// comment).
    ///
    /// The callback receives a [MixedNode] and pattern-matches on which type it names. Unlike the
    /// single-type traversals above there is no shared `context`, no [Step::Prune], and no
    /// [Step::Replace] here: those need a `Copy` context type shared across every node type at
    /// once, which a walk spanning eight different node types cannot offer without erasing it to
    /// something like `Box<dyn Any>`. A caller that needs them nests a single-type traversal
    /// manually at the point it's needed, same as without this method.
    fn try_visit_mixed<T, E>(
        &self,
        mut function: impl FnMut(MixedNode) -> Result<ControlFlow<T>, E>,
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
    fn visit_mixed<T>(&self, mut function: impl FnMut(MixedNode) -> ControlFlow<T>) -> Option<T> {
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

    /// Rewrites this node and its subtree *bottom-up*: the children of a node are rewritten before
    /// the node itself, so the callback always sees a node whose children are final.
    ///
    /// This is the counterpart of [Traverse::apply_mut], which rewrites *top-down* and therefore
    /// hands the callback a node whose children are still the original ones. Rewriting a node into
    /// a tree that contains that same node terminates here too, since every node is handed to the
    /// callback exactly once. The callback rewrites through `&mut`, so nothing is cloned; take the
    /// node apart with [std::mem::replace] when its parts have to be moved into the replacement.
    fn try_transform<E, F>(&mut self, function: &mut F) -> Result<(), E>
    where
        F: FnMut(&mut Self) -> Result<(), E>,
    {
        self.transform_children(function)?;
        function(self)
    }

    /// See [Traverse::try_transform], for callbacks that cannot fail.
    fn transform<F>(&mut self, mut function: F)
    where
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
/// Every node type is a [crate::Spanned] wrapper around a `Kind` enum, so the match arms are
/// written against the kind and the span is carried along untouched.
///
/// The description is a list of match arms that call `recurse` on every child of the node. It is
/// used for both the shared and the mutable recursion, so it must be spelled in a way that is
/// valid under both: bind children through match ergonomics and destructure nested structs with
/// `let`, never through `&x.field` or `&mut x.field`.
///
/// `Box` fields are the one thing match ergonomics cannot see through. An arm that has to
/// dereference a box therefore has to be written twice, once in each of the optional
/// `shared_only` and `mut_only` sections; the compiler still checks that each of the two
/// resulting matches is exhaustive.
///
/// An optional trailing `mixed: { ... }` section, in the same match-arm shape but calling
/// `recurse` on a [MixedNode] rather than a `?`-able child, describes this node type's foreign-type
/// children for [Traverse::push_mixed_children] (see that method's own doc comment); omitting it
/// leaves that method at the trait's empty default, correct for a node type with no such children.
macro_rules! define_traversal {
    (
        node: $Node:ident,
        children: |$recurse:ident| { $($child:tt)* },
    ) => {
        define_traversal! {
            node: $Node,
            children: |$recurse| { $($child)* },
            shared_only: {},
            mut_only: {},
        }
    };
    (
        node: $Node:ident,
        children: |$recurse:ident| { $($child:tt)* },
        mixed: { $($mixed_child:tt)* },
    ) => {
        define_traversal! {
            node: $Node,
            children: |$recurse| { $($child)* },
            shared_only: {},
            mut_only: {},
            mixed: { $($mixed_child)* },
        }
    };
    (
        node: $Node:ident,
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

            fn transform_children<E, F>(&mut self, function: &mut F) -> Result<(), E>
            where
                F: FnMut(&mut Self) -> Result<(), E>,
            {
                let mut $recurse = |child: &mut $Node| child.try_transform(function);

                match &mut self.node {
                    $($child)*
                    $($mut_child)*
                }

                Ok(())
            }
        }
    };
    (
        node: $Node:ident,
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

            fn transform_children<E, F>(&mut self, function: &mut F) -> Result<(), E>
            where
                F: FnMut(&mut Self) -> Result<(), E>,
            {
                let mut $recurse = |child: &mut $Node| child.try_transform(function);

                match &mut self.node {
                    $($child)*
                    $($mut_child)*
                }

                Ok(())
            }
        }
    };
}

define_traversal! {
    node: SortExpression,
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
        match formula.node {
            StateFrmKind::Modality { formula, .. } => formula,
            _ => panic!("expected a modality"),
        }
    }

    /// Parses an action formula, for example `a && b`.
    fn action_formula(input: &str) -> ActFrm {
        match regular_formula(input).node {
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
                result.push(name.clone());
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
            StateFrmKind::Id(name, _) if name == "Z" => ControlFlow::Break(name.clone()),
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
                StateFrmKind::Id(name, _) if name == "Z" => ControlFlow::Break(name.clone()),
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
                depths.push((name.clone(), depth));
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
                variables.push(name.clone());
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
                && name == "X"
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
                StateFrmKind::Id(name, _) if name == "X" => {
                    ControlFlow::Continue(Step::Replace(StateFrmKind::True.into()))
                }
                StateFrmKind::Id(name, _) if name == "Y" => ControlFlow::Break("stopped"),
                _ => ControlFlow::Continue(Step::Into(context)),
            })
        });

        assert_eq!(found, Ok(Some("stopped")));
        assert_eq!(format!("{formula}"), "(true && Y)");
    }

    /// The recursive step is the only place that knows the shape of a node, so a node type whose
    /// children it forgets would silently lose them everywhere at once.
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
    /// formula at all: `RegFrm`/`ActFrm` are a different node type, outside what a single-type
    /// traversal crosses into. `visit_mixed` is the one traversal that does, following exactly the
    /// crossing `crates/typecheck/src/modal/check.rs`'s `collect_scope`/`collect_scope_regfrm`/
    /// `collect_scope_actfrm` currently make by hand.
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

    /// `try_visit_mixed` propagates a callback's error immediately, the same as every other
    /// `try_*`/fallible traversal on this trait.
    #[test]
    fn test_try_visit_mixed_reports_the_error_of_the_callback() {
        let formula = state_formula("[a]true");

        let result: Result<Option<Infallible>, &str> = formula.try_visit_mixed(|_node| Err("failed"));

        assert_eq!(result, Err("failed"));
    }

    /// Mirrors `crates/typecheck/src/modal/check.rs`'s `check_state_formula`/`check_fixed_point`:
    /// `state_vars` grows for a `FixedPoint`'s body and must shrink again once that whole body has
    /// been visited, so an inner fixpoint's variable is invisible to whatever follows it at the
    /// *same* level as the fixpoint that bound it, not just to formulas nested inside a sibling.
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
                    state_vars.push(variable.identifier.clone());
                }
                if let StateFrmKind::Id(name, _) = &formula.node {
                    scope_at_each_id.push((name.clone(), state_vars.clone()));
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

    /// `exit` must still run for every scope still open when the walk stops early, in innermost-
    /// first order, or a caller's scoped state (e.g. `state_vars` above) is left corrupted for
    /// whatever it does next -- exactly the failure mode a `Break`/error mid-traversal risks if the
    /// unwind in [Traverse::visit_subtree_scoped] were missing.
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
                    state_vars.push(variable.identifier.clone());
                }
                if let StateFrmKind::Id(name, _) = &formula.node
                    && name == "Z"
                {
                    return Ok(ControlFlow::Break("stopped"));
                }
                Ok(ControlFlow::Continue(Step::Into(context)))
            },
            |formula, _context, state_vars| {
                if let StateFrmKind::FixedPoint { variable, .. } = &formula.node {
                    exited_in_order.push(variable.identifier.clone());
                    state_vars.pop();
                }
            },
        );

        assert_eq!(found, Ok(Some("stopped")));
        assert_eq!(exited_in_order, ["Y", "X"]);
        assert!(state_vars.is_empty());
    }
}

#[cfg(test)]
mod stack_depth_probe {
    //! Proves the claim in [Traverse]'s own doc comment: `visit_subtree`/`apply_subtree` (and
    //! everything built on them) walk with an explicit heap stack, not native recursion, so their
    //! depth is bounded by available memory rather than the call stack. The tree here is built
    //! directly, bypassing the parser (which has its own, unrelated recursion limit), at a depth
    //! an order of magnitude past what any native-recursive walk over this codebase's own ASTs
    //! survives (see e.g. `merc_typecheck::modal::check::stack_depth_probe`, which SIGABRTs at a
    //! tenth of this depth for exactly that reason).
    use merc_utilities::Span;

    use super::*;
    use crate::StateFrmUnaryOp;

    fn deep_negation(depth: usize) -> StateFrm {
        let mut formula = StateFrmKind::True.spanned(Span::default());
        for _ in 0..depth {
            formula = StateFrmKind::Unary {
                op: StateFrmUnaryOp::Negation,
                expr: Box::new(formula),
            }
            .spanned(Span::default());
        }
        formula
    }

    #[test]
    fn visit_a_million_deep_negation_does_not_overflow_the_stack() {
        let formula = deep_negation(1_000_000);

        let mut count = 0usize;
        formula.visit::<Infallible, _>(|_| {
            count += 1;
            ControlFlow::Continue(())
        });
        assert_eq!(count, 1_000_001); // the million `Unary` nodes, plus the `True` at the bottom.

        // Dropping a million-deep `Box` chain recurses through the default drop glue one `Box` at
        // a time and overflows the stack on its own, completely independent of (and not fixed by)
        // the traversal above -- a known, separate gap (see
        // `review/stack-overflow-recursion.md`). Leak it so *this* test only exercises what it
        // means to.
        std::mem::forget(formula);
    }

    #[test]
    fn apply_mut_a_million_deep_negation_does_not_overflow_the_stack() {
        let mut formula = deep_negation(1_000_000);

        let mut count = 0usize;
        let result: Result<(), Infallible> = formula.apply_mut(|_| {
            count += 1;
            Ok(None)
        });
        assert!(result.is_ok());
        assert_eq!(count, 1_000_001);

        // See the comment in the `visit` test above: recursive `Drop` is a separate, un-fixed gap.
        std::mem::forget(formula);
    }

    /// [Traverse::visit_scoped] is a second, independent stack-based loop (`enter`/`exit` frames
    /// rather than plain nodes) added alongside the ones the doc comment above already covers; it
    /// gets its own probe rather than relying on the two above to also exercise it.
    #[test]
    fn visit_scoped_a_million_deep_negation_does_not_overflow_the_stack() {
        let formula = deep_negation(1_000_000);

        let mut depth = 0usize;
        let mut max_depth = 0usize;
        let result: Result<Option<Infallible>, Infallible> = formula.visit_scoped(
            (),
            &mut depth,
            |_, context, depth| {
                *depth += 1;
                max_depth = max_depth.max(*depth);
                Ok(ControlFlow::Continue(Step::Into(context)))
            },
            |_, _, depth| *depth -= 1,
        );
        assert_eq!(result, Ok(None));
        assert_eq!(max_depth, 1_000_001);
        assert_eq!(depth, 0); // every `enter` was matched by an `exit`.

        std::mem::forget(formula);
    }

    /// [Traverse::visit_mixed] is a third, independent stack-based loop (over [MixedNode] rather
    /// than a single node type); a chain of plain `Unary` negations never leaves [StateFrm], so
    /// this exercises the same "no native recursion" property for that loop specifically, even
    /// though it does not exercise crossing into a different node type (covered separately by
    /// `test_visit_mixed_crosses_from_state_formula_into_its_regular_and_action_formulas`).
    #[test]
    fn visit_mixed_a_million_deep_negation_does_not_overflow_the_stack() {
        let formula = deep_negation(1_000_000);

        let mut count = 0usize;
        formula.visit_mixed::<Infallible>(|_| {
            count += 1;
            ControlFlow::Continue(())
        });
        assert_eq!(count, 1_000_001);

        std::mem::forget(formula);
    }
}
