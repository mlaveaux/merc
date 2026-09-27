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
/// [Traverse::try_transform]/`transform` share this too, needing a fundamentally different
/// technique to get there: a bottom-up rewrite needs a node's children fully processed *before*
/// touching the node itself, which the worklist-of-borrows trick above cannot do for an owned,
/// `Box`/`Vec`-based tree like this one (as opposed to an arena addressed by index) -- it would mean
/// holding a live mutable borrow of the already-processed children at the same time as the borrow
/// needed to reach the parent again, which the borrow checker rejects. [Traverse::try_transform]'s
/// own doc comment covers the different technique this needed instead: moving *owned* nodes between
/// two worklists (`Self: Default`, via [std::mem::take], stands in for the node a worklist entry
/// took the place of, the same way [`crate::TakeRecursiveChildren`] does and for the same reason).
/// [Traverse::transform_children] is built on top of the now-iterative `try_transform`, one call per
/// direct child, rather than needing its own copy of the same technique. `crates/typecheck`'s
/// `.transform`/`.try_transform` call sites (`ir/desugar.rs`, `ir/lower.rs`,
/// `resolution/name_resolution.rs`) are stack-safe for a pathologically deep tree as a result,
/// with no call-site changes needed.
///
/// None of the above crosses into a *different* node type on its own: a [StateFrm] traversal does
/// not, by itself, descend into the [RegFrm] of a `Modality` or the [ActFrm] inside that. Three more
/// pieces close gaps that used to force a caller to work around this trait rather than through it:
///
/// - [Traverse::visit_mixed]/[Traverse::try_visit_mixed] *do* cross that boundary, visiting every
///   node of every type reachable from `self` (see [MixedNode]) in one walk, for callbacks that
///   need to see a whole formula regardless of which node type each part of it lives in without
///   hand-rolling one recursive function per type and wiring the crossings between them, the way
///   `crates/typecheck`'s `modal::check::collect_scope` does today.
/// - [Traverse::visit_subtree_scoped]/[Traverse::visit_scoped] add the enter-*and*-exit hook that
///   [Visit]'s single pre-order callback cannot express, for a read-only caller that pushes
///   genuinely scoped mutable state on entering a node (e.g. `check_state_formula`'s `state_vars`,
///   pushed for a `mu`/`nu` binder's body) and needs it popped again once that node's subtree, not
///   just the node itself, is done being visited.
/// - [Traverse::apply_subtree_scoped]/[Traverse::apply_scoped] are the same enter/exit hook for a
///   caller that must *rewrite* nodes rather than only read them (e.g.
///   `resolve_in_state_frm`'s `scope`, extended for a bound variable and truncated again once its
///   body is fully resolved) -- see that method's own doc comment for why its `exit` cannot be
///   handed the node the way [Traverse::visit_scoped]'s can.
///
/// None of these three migrates every hand-written checker or resolution pass that could use them --
/// `crates/typecheck`'s `process::check`/`pres::check` and `resolution::variable_resolution` (its
/// `resolve_in_state_frm`, built on the third addition, is the one part of it migrated so far) show
/// what doing so for a given caller takes; see `review/stack-overflow-recursion.md`. A single-type
/// traversal still does not cross node types on its own outside of `visit_mixed`, and everywhere but
/// the crossings it's already wired for, nesting a traversal explicitly at the point it's needed is
/// still exactly how to do it.
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
    ///
    /// Built on [Traverse::push_children_mut] plus [Traverse::try_transform] (itself iterative --
    /// see that method's own doc comment), so this adds no native recursion of its own: it detaches
    /// each of `self`'s own direct children, in turn hands each one wholesale to `try_transform`
    /// (which fully transforms it and everything below it before returning), and puts it back --
    /// one call per *direct* child, never nested inside another, so the native call depth this adds
    /// is bounded by `self`'s own branching factor, not the tree's depth.
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

    /// See [Traverse::apply_subtree]/[Traverse::visit_subtree_scoped]: the mutating counterpart of
    /// the latter, for a caller that must rewrite nodes in place *and* needs the enter/exit split
    /// [Traverse::visit_subtree_scoped]'s own doc comment explains (genuinely scoped mutable state
    /// pushed on entering a node, popped once that node's whole subtree, not just the node itself,
    /// is done) -- e.g. `resolution::variable_resolution::resolve_in_state_frm`'s `scope`, extended
    /// for a `Quantifier`/`Bound`/`FixedPoint`'s own bound variables and truncated again once its
    /// body is fully resolved.
    ///
    /// `exit` does not receive the node it is un-scoping, unlike [Traverse::visit_subtree_scoped]'s
    /// own `exit`: holding any reference to a node here, across the window where its children are
    /// then reached through a *fresh* `&mut` borrow of that same node, is exactly the aliasing this
    /// trait's other mutating methods avoid by construction (an [Self::apply_subtree]-style walk
    /// never needs to hold a node past the one `&mut` borrow it hands to its own step function; a
    /// scoped walk's `exit` runs *after* that borrow was already used to reach every child, so a
    /// second, overlapping one is not available to hand it) -- there is no `unsafe` way around this
    /// in a `#![forbid(unsafe_code)]` crate. `exit` gets only `context` (the same value `enter` was
    /// called with for that node) and `state`. A caller whose `exit` needs to know *what kind* of
    /// node it is un-scoping -- the way [Traverse::visit_scoped]'s callers re-match `node.node`, since
    /// its `exit` still has the node -- instead has `enter` push that decision onto `state` itself
    /// (once per node, even a no-op for the common case, keeping every `enter` paired with exactly
    /// one `exit`), for `exit` to pop and apply from there; see
    /// `resolution::variable_resolution::resolve_in_state_frm`'s local `Undo` enum for the pattern.
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
    ///
    /// A bottom-up rewrite needs a node's children fully processed *before* touching the node
    /// itself, which rules out [Traverse::apply_subtree]'s worklist-of-borrows trick: that stack
    /// holds `&mut` references *into* the still-standing tree, so a node reached later and a node
    /// already finished earlier could otherwise both be live at once when the later one is the
    /// earlier one's own ancestor -- the borrow checker (rightly) refuses this for a `Box`/`Vec`-
    /// owned tree (an arena addressed by index wouldn't have the same conflict, since "reaching a
    /// node" would no longer mean borrowing through its ancestors). This instead moves *owned*
    /// nodes between two worklists, detaching a node's children (via [Traverse::push_children_mut]
    /// and [std::mem::take], needing `Self: Default` the same way [`crate::TakeRecursiveChildren`]
    /// does and for the same underlying reason: extracting a value from a borrowed slot without
    /// leaving it empty needs *something* to leave behind) before it is ever pushed anywhere, so no
    /// two worklist entries ever alias the same storage:
    ///
    /// - `stack` holds `Enter(node)` / `Assemble(node, child_count)` frames, LIFO.
    /// - `results` accumulates each fully-transformed node, in the order it finishes.
    ///
    /// `Enter(node)` detaches `node`'s own children in written order, pushes `Assemble(node,
    /// count)` (to run once they're all done), then pushes each child as its own `Enter` frame in
    /// *reverse* -- so the LIFO stack pops them back out left-to-right, the same order native
    /// recursion would visit them in. `Assemble(node, count)` only ever runs once every one of
    /// `node`'s `count` children has itself been fully transformed and pushed onto `results`: they
    /// are its last `count` entries, in original order, because nothing between them and the top of
    /// `results` at that point can belong to any other node (an `Enter` frame is never pushed for a
    /// node until all of its *earlier* siblings' whole subtrees have already finished and landed in
    /// `results`, and a node's own `Assemble` frame sits right below its children's `Enter` frames on
    /// `stack`, so it cannot run before every one of them has). `Assemble` puts those children back
    /// into `node` (replacing the placeholders `Enter` left), calls `function` on the now-whole node,
    /// and pushes the result onto `results` in turn -- so by the time an ancestor's own `Assemble`
    /// frame runs, its children are already waiting there for it, and once the loop empties `stack`,
    /// `results` holds exactly one value: the fully-transformed root.
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
///
/// `kind: $Kind` (the `Kind` enum `$Node` wraps, e.g. `StateFrmKind` for `StateFrm`) drives a
/// second, unrelated piece this macro generates alongside the `Traverse` impl:
/// `impl TakeRecursiveChildren for $Kind`, reusing the exact same `$child`/`$mut_child` arms
/// (calling `recurse` on each same-type child exactly as `push_children_mut` does) to fix a
/// different bug than `Traverse` itself does -- see [TakeRecursiveChildren]'s own doc comment for
/// why a *second* fix was needed even after `Traverse`'s traversals became stack-safe. It relies on
/// `$Kind: Default` (see each `*Kind` enum's own `#[default]`-marked variant) for a cheap,
/// non-recursive value to detach a child in favor of.
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

    #[test]
    fn test_transform_visits_bottom_up_left_to_right() {
        // A leaf's own children (none) finish before it does, each operand of `&&` finishes before
        // the `&&` node itself, and the left operand before the right one.
        let mut formula = state_formula("X && Y");

        let mut order = Vec::new();
        formula.transform(|formula| match &formula.node {
            StateFrmKind::Id(name, _) => order.push(name.clone()),
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
                visited.push(name.clone());
                if name == "Y" {
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

#[cfg(test)]
mod stack_depth_probe {
    //! Proves the claim in [Traverse]'s own doc comment: `visit_subtree`/`apply_subtree` (and
    //! everything built on them) walk with an explicit heap stack, not native recursion, so their
    //! depth is bounded by available memory rather than the call stack. The tree here is built
    //! directly, bypassing the parser (which has its own, unrelated recursion limit), at a depth
    //! an order of magnitude past what any native-recursive walk over this codebase's own ASTs used
    //! to survive (see e.g. `merc_typecheck::modal::check::stack_depth_probe`, migrated onto this
    //! trait rather than its own recursive-descent walk for exactly that reason).
    //!
    //! Each test here also lets its million-deep tree actually drop, rather than leaking it: that
    //! exercises a *second*, separate bug the traversal fix alone does not touch -- the compiler's
    //! own default field-by-field drop glue recursing through the same `Box`-chain, one node at a
    //! time, on the way down. See [TakeRecursiveChildren]'s and [Spanned]'s own `Drop` impl's doc
    //! comments, and `review/stack-overflow-recursion.md`, for that fix.
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

        // Dropping a million-deep `Box` chain used to recurse through the default drop glue one
        // `Box` at a time and overflow the stack on its own, completely independent of (and not
        // fixed by) the traversal above -- a separate bug from the one this module's own doc
        // comment covers, now fixed by `TakeRecursiveChildren`/`Spanned`'s own `Drop` impl (see
        // `review/stack-overflow-recursion.md`). Letting `formula` actually drop here, instead of
        // leaking it, is this test's proof that fix holds at the same depth.
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

        // See the comment in the `visit` test above: `formula` dropping here, rather than being
        // leaked, is the proof that the separate recursive-`Drop` bug is also fixed.
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

        // `formula` drops here -- see `visit_a_million_deep_negation_does_not_overflow_the_stack`.
    }

    /// [Traverse::apply_scoped] is the mutating counterpart of `visit_scoped` (added for
    /// `resolution::variable_resolution::resolve_in_state_frm`'s migration); it gets its own probe
    /// for the same reason `visit_scoped` does, and also exercises that `enter` really does get a
    /// mutable reference this deep by rewriting every node's own operator field in place.
    #[test]
    fn apply_scoped_a_million_deep_negation_does_not_overflow_the_stack() {
        let mut formula = deep_negation(1_000_000);

        let mut depth = 0usize;
        let mut max_depth = 0usize;
        let result: Result<Option<Infallible>, Infallible> = formula.apply_scoped(
            (),
            &mut depth,
            |formula, context, depth| {
                if let StateFrmKind::Unary { op, .. } = &mut formula.node {
                    *op = StateFrmUnaryOp::Negation;
                }
                *depth += 1;
                max_depth = max_depth.max(*depth);
                Ok(ControlFlow::Continue(Step::Into(context)))
            },
            |_, depth| *depth -= 1,
        );
        assert_eq!(result, Ok(None));
        assert_eq!(max_depth, 1_000_001);
        assert_eq!(depth, 0); // every `enter` was matched by an `exit`.

        // `formula` drops here -- see `visit_a_million_deep_negation_does_not_overflow_the_stack`.
    }

    /// [Traverse::try_transform] (and `transform`) use a fundamentally different, two-worklist
    /// technique from every walk above -- see that method's own doc comment -- so it gets its own
    /// probe rather than relying on any of them to also exercise it. Rewriting every node's own
    /// operator field in place (the same as `apply_scoped`'s probe) also proves the reassembled
    /// tree is the *same* tree, not just one of the same depth: a node landing in the wrong slot
    /// during reassembly would still count correctly here, but would carry the wrong operator.
    #[test]
    fn transform_a_million_deep_negation_does_not_overflow_the_stack() {
        let mut formula = deep_negation(1_000_000);

        let mut count = 0usize;
        formula.transform(|formula| {
            if let StateFrmKind::Unary { op, .. } = &mut formula.node {
                *op = StateFrmUnaryOp::Negation;
                count += 1;
            }
        });
        assert_eq!(count, 1_000_000); // every `Unary` node, not the `True` leaf at the bottom.

        // `formula` drops here -- see `visit_a_million_deep_negation_does_not_overflow_the_stack`.
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

        // `formula` drops here -- see `visit_a_million_deep_negation_does_not_overflow_the_stack`.
    }
}
