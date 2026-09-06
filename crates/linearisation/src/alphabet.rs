//! Alphabet-based simplification, porting mCRL2's `process::alphabet_operations`
//! / `process::alphabet_reduce` (and the book's — Groote & Mousavi §4 — laws
//! for eliminating a `hide`/`block`/`allow`/`rename`/`comm` wrapper that
//! provably has no effect on its operand).
//!
//! # The alphabet of a process
//!
//! The *alphabet* of a process expression is the set of multi-actions
//! (multisets of action names, ignoring data arguments — [`MultiActionName`])
//! it can ever perform. It is computed structurally: `delta`'s alphabet is
//! empty, `tau`'s is `{tau}`, an action's is itself, choice/sequence union
//! both sides, and so on — mirroring `translate::translate`'s own per-operator
//! semantics one level up in abstraction (names and multiplicities only, no
//! data, no control flow).
//!
//! [`alphabet`] computes this *exactly* for the fragment this crate
//! currently linearises (see the crate README): a process call back to a
//! name already being computed — direct self-recursion, or, more generally,
//! any cycle through several equations — contributes nothing *further*,
//! because a second pass through an equation can only ever produce multi-actions
//! already found on the first pass. This is sound (an over-approximation) for
//! *any* process graph, cyclic or not — `translate::translate` will reject
//! genuine mutual recursion with [`crate::LinearisationError::MutualRecursionUnsupported`]
//! regardless — and it happens to be *exact* for the single-equation
//! self-recursive processes that crate currently accepts. `Comm` and
//! `Parallel`/`LeftMerge` are likewise given a sound alphabet here even
//! though `translate::translate` does not support them yet, purely so
//! [`simplify`] can already recognise a `comm(C, P)` or nested `P || Q` that
//! turns out to need none of its operator's machinery — infrastructure
//! Milestone 3 (full parallel composition) will lean on more heavily.
//!
//! # Why over-approximation is always safe here
//!
//! [`simplify`] only ever *removes* a `hide`/`block`/`allow`/`rename`/`comm`
//! wrapper, and only when [`alphabet`] proves a stated property of the
//! *whole* alphabet (e.g. "no name in `I` occurs in any multi-action of
//! `alphabet(P)`"). Overestimating the alphabet can at worst make such a
//! proof fail when the wrapper actually was redundant — a missed
//! simplification, never an incorrect one. `Comm`'s contribution
//! ([`Alphabet::with_communications`]) is ported faithfully from mCRL2's own
//! `alphabet_operations::comm`, so it needs no approximation of its own. The
//! one place this module *does* round up rather than compute the exact
//! answer is `Parallel`/`LeftMerge`/`CommMerge`'s cross product
//! ([`Alphabet::combined`]), which multiplies set sizes at *every* nesting
//! level and would be exponential in the number of parallel components for
//! exactly the specifications this matters most for (see
//! `test_alphabet_of_a_wide_parallel_composition_stays_bounded` in
//! `tests/alphabet_test.rs`). [`Alphabet::Unknown`] is the escape hatch: a
//! safe "could be anything" top value substituted in whenever computing
//! precisely would be too expensive, against which every axiom's check
//! conservatively fails (never fires incorrectly, may simply miss an
//! available simplification). This module's own test suite (bottom of this
//! file) checks the lazy, bound-respecting [`Alphabet::combined_with_limit`]
//! against a deliberately naive, always-exact reference implementation of
//! the same cross product, on inputs small enough for "always exact" to be
//! affordable.

use std::collections::HashMap;
use std::collections::HashSet;

use merc_syntax::CommExpr;
use merc_syntax::MultiActionLabel;
use merc_syntax::ProcDecl;
use merc_syntax::ProcExprBinaryOp;
use merc_syntax::ProcessExpr;
use merc_syntax::ProcessExprKind;
use merc_syntax::respan;

use crate::ActionInstance;

/// A multiset of action names — what `hide`, `block`, `allow`, `rename`, and
/// `comm` all actually match against, ignoring data arguments entirely.
/// Stored as a sorted vector so two multisets compare/hash equal regardless
/// of the order their actions were written or collected in.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct MultiActionName(Vec<String>);

impl MultiActionName {
    /// The empty multiset: `tau`.
    pub(crate) fn tau() -> Self {
        MultiActionName(Vec::new())
    }

    pub(crate) fn is_tau(&self) -> bool {
        self.0.is_empty()
    }

    fn from_names(mut names: Vec<String>) -> Self {
        names.sort_unstable();
        MultiActionName(names)
    }

    /// The multiset performed by an already-built [`ActionInstance`] list —
    /// a finished summand's actual multi-action.
    pub(crate) fn from_actions(actions: &[ActionInstance]) -> Self {
        Self::from_names(actions.iter().map(|action| action.name.node.clone()).collect())
    }

    /// The multiset denoted by an `allow`/`comm` multi-action pattern such as
    /// `a|b`.
    pub(crate) fn from_label(label: &MultiActionLabel) -> Self {
        Self::from_names(label.actions.iter().map(|name| name.node.clone()).collect())
    }

    /// `self ∪ other`, as multisets (so `{a} ∪ {a} = {a,a}`, not `{a}`).
    fn merged(&self, other: &Self) -> Self {
        let mut names = self.0.clone();
        names.extend(other.0.iter().cloned());
        Self::from_names(names)
    }

    /// True iff any element of `self` is one of `names` — the test `block`
    /// and the no-op axioms for `hide`/`rename` need.
    fn intersects(&self, names: &HashSet<&str>) -> bool {
        self.0.iter().any(|name| names.contains(name.as_str()))
    }

    /// `self` with every occurrence of a name in `hidden` removed.
    fn hidden(&self, hidden: &HashSet<&str>) -> Self {
        MultiActionName(
            self.0
                .iter()
                .filter(|name| !hidden.contains(name.as_str()))
                .cloned()
                .collect(),
        )
    }

    /// `self` with every name renamed per `table`, names not mentioned kept
    /// as-is.
    fn renamed(&self, table: &HashMap<&str, &str>) -> Self {
        Self::from_names(
            self.0
                .iter()
                .map(|name| table.get(name.as_str()).copied().unwrap_or(name).to_string())
                .collect(),
        )
    }

    /// True iff `pattern` occurs in `self` as a sub-multiset (every name in
    /// `pattern`, with at least its multiplicity, is present in `self`) —
    /// whether one of a `comm` expression's left-hand sides could ever
    /// actually fire against this multi-action.
    fn contains_submultiset(&self, pattern: &MultiActionName) -> bool {
        let mut remaining = self.0.clone();
        for name in &pattern.0 {
            match remaining.iter().position(|n| n == name) {
                Some(index) => {
                    remaining.remove(index);
                }
                None => return false,
            }
        }
        true
    }

    /// `self` with one occurrence of `pattern` (assumed a sub-multiset, see
    /// [`Self::contains_submultiset`]) replaced by `replacement`.
    fn communicated(&self, pattern: &MultiActionName, replacement: &str) -> Self {
        let mut names = self.0.clone();
        for name in &pattern.0 {
            if let Some(index) = names.iter().position(|n| n == name) {
                names.remove(index);
            }
        }
        names.push(replacement.to_string());
        Self::from_names(names)
    }
}

/// Above this many entries, [`Alphabet::combined`] gives up on computing the
/// exact cross product of a `||`/`|`'s two operand alphabets and returns
/// [`Alphabet::Unknown`] instead — see the module documentation.
const MAX_PARALLEL_ALPHABET: usize = 4096;

/// The set of multi-actions a process expression can ever perform — see the
/// module documentation for the precision guarantees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Alphabet {
    Finite(HashSet<MultiActionName>),
    /// "Could be any multiset of action names": a safe top value, standing
    /// in for a precise set this module chose not to compute (see the module
    /// documentation). Every axiom check below conservatively refuses to
    /// fire against it.
    Unknown,
}

impl Alphabet {
    fn empty() -> Self {
        Alphabet::Finite(HashSet::new())
    }

    fn single(multi_action: MultiActionName) -> Self {
        Alphabet::Finite([multi_action].into_iter().collect())
    }

    /// `self ∪ other`.
    fn union(self, other: Self) -> Self {
        match (self, other) {
            (Alphabet::Finite(mut a), Alphabet::Finite(b)) => {
                a.extend(b);
                Alphabet::Finite(a)
            }
            _ => Alphabet::Unknown,
        }
    }

    /// The alphabet of two operands run in parallel: interleaving
    /// (`self ∪ other`) plus synchronisation (every pairwise merge). See
    /// [`Self::combined_with_limit`] for the bound this applies.
    fn combined(self, other: Self) -> Self {
        self.combined_with_limit(other, MAX_PARALLEL_ALPHABET)
    }

    /// [`Self::combined`], parameterized by how many distinct multi-actions
    /// to allow before giving up and returning [`Alphabet::Unknown`] —
    /// [`Self::combined`] fixes this at [`MAX_PARALLEL_ALPHABET`]; the test
    /// suite also calls this directly with `usize::MAX` (effectively no
    /// limit) as a naive, obviously-correct-by-inspection reference to
    /// check the lazy, early-exiting version below against.
    ///
    /// Built as a single lazy iterator chain (`self`'s elements, then
    /// `other`'s, then every pairwise merge — nothing eagerly collected in
    /// between) fed to [`collect_bounded`], so a wide composition whose
    /// *distinct* combined alphabet is actually small stays fast even if the
    /// raw number of candidate pairs is large (many merges commonly collide
    /// to the same multiset), and one whose alphabet genuinely explodes
    /// stops pulling further pairs the moment the limit is crossed, rather
    /// than first computing the full cross product and discovering it was
    /// too big only afterwards.
    fn combined_with_limit(self, other: Self, limit: usize) -> Self {
        match (self, other) {
            (Alphabet::Finite(a), Alphabet::Finite(b)) => {
                let candidates = a
                    .iter()
                    .cloned()
                    .chain(b.iter().cloned())
                    .chain(a.iter().flat_map(|x| b.iter().map(move |y| x.merged(y))));
                collect_bounded(candidates, limit)
            }
            _ => Alphabet::Unknown,
        }
    }

    fn map(self, f: impl Fn(&MultiActionName) -> MultiActionName) -> Self {
        match self {
            Alphabet::Finite(set) => Alphabet::Finite(set.iter().map(f).collect()),
            Alphabet::Unknown => Alphabet::Unknown,
        }
    }

    fn filter(self, f: impl Fn(&MultiActionName) -> bool) -> Self {
        match self {
            Alphabet::Finite(set) => Alphabet::Finite(set.into_iter().filter(f).collect()),
            Alphabet::Unknown => Alphabet::Unknown,
        }
    }

    /// The property `simplify`'s `hide`/`block`/`rename` no-op axioms need:
    /// no multi-action in `self` contains any name in `names`. Conservatively
    /// `false` for [`Alphabet::Unknown`] — can't prove disjointness, so the
    /// axiom simply won't fire, never incorrectly.
    fn disjoint_from(&self, names: &HashSet<&str>) -> bool {
        matches!(self, Alphabet::Finite(set) if set.iter().all(|m| !m.intersects(names)))
    }

    /// The property the `allow` no-op axiom needs: every multi-action in
    /// `self` is `tau` or a member of `allowed`. Conservatively `false` for
    /// [`Alphabet::Unknown`].
    fn subset_of_allowed(&self, allowed: &HashSet<MultiActionName>) -> bool {
        matches!(self, Alphabet::Finite(set) if set.iter().all(|m| m.is_tau() || allowed.contains(m)))
    }

    /// The negation of the property the `comm` no-op axiom needs: some
    /// multi-action in `self` contains one of `comm`'s left-hand sides as a
    /// sub-multiset. Conservatively `true` (can't prove it never fires) for
    /// [`Alphabet::Unknown`].
    fn can_communicate(&self, comm: &[CommExpr]) -> bool {
        match self {
            Alphabet::Finite(set) => comm.iter().any(|c| {
                set.iter()
                    .any(|m| m.contains_submultiset(&MultiActionName::from_label(&c.from)))
            }),
            Alphabet::Unknown => true,
        }
    }

    /// `Comm`'s own contribution, ported from mCRL2's
    /// `process::alphabet_operations::comm`/`apply_comm`: apply each
    /// communication expression in `comm` *in order* (so a later expression
    /// can act on multi-actions an earlier one just produced — no separate
    /// fixed-point loop needed, one left-to-right pass over `comm` already
    /// is one), and for each, repeatedly consume every occurrence of its
    /// left-hand side within a single multi-action (so `a|a|b|b` under
    /// `comm(a|b -> c)` yields `c|c`, not just `a|b|c`). Unlike
    /// [`Self::combined`] this isn't an approximation of `comm`'s semantics
    /// at all — it's the same computation mCRL2 itself does on a
    /// `multi_action_name_set` — it only *approximates* the real per-summand
    /// `comm` operator because it runs on the whole alphabet rather than
    /// tracking which multi-action came from which summand.
    fn with_communications(self, comm: &[CommExpr]) -> Self {
        match self {
            Alphabet::Finite(mut result) => {
                for c in comm {
                    apply_comm(c, &mut result);
                }
                Alphabet::Finite(result)
            }
            Alphabet::Unknown => Alphabet::Unknown,
        }
    }
}

/// Applies one communication expression to every multi-action already in
/// `result`, inserting each newly derived one — see
/// [`Alphabet::with_communications`].
fn apply_comm(c: &CommExpr, result: &mut HashSet<MultiActionName>) {
    let pattern = MultiActionName::from_label(&c.from);
    let mut to_add = HashSet::new();
    for multi_action in result.iter() {
        let mut current = multi_action.clone();
        while current.contains_submultiset(&pattern) {
            current = current.communicated(&pattern, &c.to.node);
            to_add.insert(current.clone());
        }
    }
    result.extend(to_add);
}

impl FromIterator<MultiActionName> for Alphabet {
    fn from_iter<T: IntoIterator<Item = MultiActionName>>(iter: T) -> Self {
        Alphabet::Finite(iter.into_iter().collect())
    }
}

/// Drains a lazily-produced sequence of multi-actions into a set, bailing
/// out to [`Alphabet::Unknown`] the moment the *distinct* count would exceed
/// `limit` — deliberately checked against the growing result, not some
/// upfront estimate of the input size, so duplicate candidates (common once
/// several `||`s are nested) never cost more than the one `HashSet::insert`
/// that discovers they're duplicates. `items` is only ever pulled from until
/// the bail-out (or exhaustion), so a caller building `items` from further
/// lazy sources (as [`Alphabet::combined_with_limit`] does) never pays for
/// candidates beyond the one that crosses the line.
fn collect_bounded(items: impl Iterator<Item = MultiActionName>, limit: usize) -> Alphabet {
    let mut result = HashSet::new();
    for item in items {
        result.insert(item);
        if result.len() > limit {
            return Alphabet::Unknown;
        }
    }
    Alphabet::Finite(result)
}

/// Computes the alphabet of `body`, the (possibly absent) currently-being-linearised
/// process's body. `own_name` seeds the cycle guard described in the module
/// documentation, exactly matching [`crate::translate::Context::own_name`]'s
/// role.
pub(crate) fn alphabet(body: &ProcessExpr, own_name: Option<&str>, processes: &HashMap<&str, &ProcDecl>) -> Alphabet {
    let mut stack = HashSet::new();
    if let Some(name) = own_name {
        stack.insert(name);
    }
    alphabet_of(body, processes, &mut stack)
}

fn alphabet_of<'a>(
    expr: &ProcessExpr,
    processes: &HashMap<&str, &'a ProcDecl>,
    stack: &mut HashSet<&'a str>,
) -> Alphabet {
    match &expr.node {
        ProcessExprKind::Delta => Alphabet::empty(),
        ProcessExprKind::Tau => Alphabet::single(MultiActionName::tau()),

        ProcessExprKind::Action(name, _) => match processes.get(name.node.as_str()) {
            Some(&decl) => call_alphabet(decl, processes, stack),
            None => Alphabet::single(MultiActionName::from_names(vec![name.node.clone()])),
        },
        ProcessExprKind::Id(name, _) => {
            let &decl = processes
                .get(name.node.as_str())
                .expect("`Id` always names a declared process once type checking has succeeded");
            call_alphabet(decl, processes, stack)
        }

        ProcessExprKind::Sum { operand, .. }
        | ProcessExprKind::At { expr: operand, .. }
        | ProcessExprKind::Dist { operand, .. } => alphabet_of(operand, processes, stack),

        ProcessExprKind::Condition { then, else_, .. } => {
            let then_alphabet = alphabet_of(then, processes, stack);
            match else_ {
                Some(operand) => then_alphabet.union(alphabet_of(operand, processes, stack)),
                None => then_alphabet, // no `else`: the implicit branch is `delta`, contributing nothing
            }
        }

        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Choice | ProcExprBinaryOp::Sequence | ProcExprBinaryOp::Until,
            lhs,
            rhs,
        } => {
            // Sound over-approximation for `Sequence`/`Until` too: every
            // multi-action either side can ever perform is certainly in the
            // whole expression's alphabet, whichever order they run in.
            alphabet_of(lhs, processes, stack).union(alphabet_of(rhs, processes, stack))
        }

        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Parallel | ProcExprBinaryOp::LeftMerge | ProcExprBinaryOp::CommMerge,
            lhs,
            rhs,
        } => {
            // The same rule covers all three: interleaving (either side
            // alone) plus synchronisation (both sides' multi-actions
            // combined). `CommMerge` (`a|b`) only ever legally combines
            // actions in `translate::collect_multi_action`'s restricted
            // sense, but this general rule is still a sound
            // over-approximation of that restricted case.
            alphabet_of(lhs, processes, stack).combined(alphabet_of(rhs, processes, stack))
        }

        ProcessExprKind::Rename { renames, operand } => {
            let table: HashMap<&str, &str> = renames
                .iter()
                .map(|r| (r.from.node.as_str(), r.to.node.as_str()))
                .collect();
            alphabet_of(operand, processes, stack).map(|multi_action| multi_action.renamed(&table))
        }

        ProcessExprKind::Hide { actions, operand } => {
            let hidden: HashSet<&str> = actions.iter().map(|name| name.node.as_str()).collect();
            alphabet_of(operand, processes, stack).map(|multi_action| multi_action.hidden(&hidden))
        }

        ProcessExprKind::Block { actions, operand } => {
            let blocked: HashSet<&str> = actions.iter().map(|name| name.node.as_str()).collect();
            alphabet_of(operand, processes, stack).filter(|multi_action| !multi_action.intersects(&blocked))
        }

        ProcessExprKind::Allow { actions, operand } => {
            let allowed: HashSet<MultiActionName> = actions.iter().map(MultiActionName::from_label).collect();
            alphabet_of(operand, processes, stack)
                .filter(|multi_action| multi_action.is_tau() || allowed.contains(multi_action))
        }

        ProcessExprKind::Comm { comm, operand } => alphabet_of(operand, processes, stack).with_communications(comm),
    }
}

/// The alphabet of a call to `decl`, guarding against infinite recursion:
/// see the module documentation for why treating a repeated name as
/// contributing nothing further is sound (and, for the fragment this crate
/// currently linearises, exact).
fn call_alphabet<'a>(
    decl: &'a ProcDecl,
    processes: &HashMap<&str, &'a ProcDecl>,
    stack: &mut HashSet<&'a str>,
) -> Alphabet {
    let name = decl.identifier.node.as_str();
    if !stack.insert(name) {
        return Alphabet::empty();
    }
    let result = alphabet_of(&decl.body, processes, stack);
    stack.remove(name);
    result
}

/// Applies the book's `hide`/`block`/`allow`/`rename`/`comm` no-op axioms
/// bottom-up over `expr`, dropping a wrapper node wherever [`alphabet`]
/// proves it has no effect on its (already-simplified) operand — mCRL2's
/// `t_lin_options::apply_alphabet_axioms`, run unconditionally rather than
/// behind a flag, since it can only ever remove redundant nodes, never
/// change behaviour (see the module documentation).
///
/// A pure, non-destructive rewrite of a *borrowed* tree: `expr` (and, for a
/// compositional operand, the process's own declared body) come from the
/// already-typed-checked `ProcessSpecification`, which this crate has no
/// business mutating, so this clones its way to a new, simplified tree
/// rather than editing in place.
pub(crate) fn simplify(
    expr: &ProcessExpr,
    own_name: Option<&str>,
    processes: &HashMap<&str, &ProcDecl>,
) -> ProcessExpr {
    let node = match &expr.node {
        ProcessExprKind::Action(..) | ProcessExprKind::Id(..) | ProcessExprKind::Delta | ProcessExprKind::Tau => {
            return expr.clone();
        }

        ProcessExprKind::Sum { variables, operand } => ProcessExprKind::Sum {
            variables: variables.clone(),
            operand: Box::new(simplify(operand, own_name, processes)),
        },
        ProcessExprKind::Dist {
            variables,
            expr: weight,
            operand,
        } => ProcessExprKind::Dist {
            variables: variables.clone(),
            expr: weight.clone(),
            operand: Box::new(simplify(operand, own_name, processes)),
        },
        ProcessExprKind::At {
            expr: operand,
            operand: time,
        } => ProcessExprKind::At {
            expr: Box::new(simplify(operand, own_name, processes)),
            operand: time.clone(),
        },
        ProcessExprKind::Condition { condition, then, else_ } => ProcessExprKind::Condition {
            condition: condition.clone(),
            then: Box::new(simplify(then, own_name, processes)),
            else_: else_
                .as_ref()
                .map(|operand| Box::new(simplify(operand, own_name, processes))),
        },
        ProcessExprKind::Binary { op, lhs, rhs } => ProcessExprKind::Binary {
            op: op.clone(),
            lhs: Box::new(simplify(lhs, own_name, processes)),
            rhs: Box::new(simplify(rhs, own_name, processes)),
        },

        ProcessExprKind::Rename { renames, operand } => {
            let operand = simplify(operand, own_name, processes);
            let renamed_names: HashSet<&str> = renames.iter().map(|r| r.from.node.as_str()).collect();
            if alphabet(&operand, own_name, processes).disjoint_from(&renamed_names) {
                return operand; // axiom: rename(R, P) = P if none of R's sources occur in alphabet(P)
            }
            ProcessExprKind::Rename {
                renames: renames.clone(),
                operand: Box::new(operand),
            }
        }

        ProcessExprKind::Hide { actions, operand } => {
            let operand = simplify(operand, own_name, processes);
            let hidden: HashSet<&str> = actions.iter().map(|name| name.node.as_str()).collect();
            if alphabet(&operand, own_name, processes).disjoint_from(&hidden) {
                return operand; // axiom: hide(I, P) = P if I ∩ alphabet(P) = ∅
            }
            ProcessExprKind::Hide {
                actions: actions.clone(),
                operand: Box::new(operand),
            }
        }

        ProcessExprKind::Block { actions, operand } => {
            let operand = simplify(operand, own_name, processes);
            let blocked: HashSet<&str> = actions.iter().map(|name| name.node.as_str()).collect();
            if alphabet(&operand, own_name, processes).disjoint_from(&blocked) {
                return operand; // axiom: block(B, P) = P if B ∩ alphabet(P) = ∅
            }
            ProcessExprKind::Block {
                actions: actions.clone(),
                operand: Box::new(operand),
            }
        }

        ProcessExprKind::Allow { actions, operand } => {
            let operand = simplify(operand, own_name, processes);
            let allowed: HashSet<MultiActionName> = actions.iter().map(MultiActionName::from_label).collect();
            if alphabet(&operand, own_name, processes).subset_of_allowed(&allowed) {
                return operand; // axiom: allow(A, P) = P if alphabet(P) ⊆ A ∪ {tau}
            }
            ProcessExprKind::Allow {
                actions: actions.clone(),
                operand: Box::new(operand),
            }
        }

        ProcessExprKind::Comm { comm, operand } => {
            let operand = simplify(operand, own_name, processes);
            if !alphabet(&operand, own_name, processes).can_communicate(comm) {
                return operand; // axiom: comm(C, P) = P if no expression in C can ever fire in P
            }
            ProcessExprKind::Comm {
                comm: comm.clone(),
                operand: Box::new(operand),
            }
        }
    };

    respan(expr.span.clone(), node)
}

#[cfg(test)]
mod tests {
    use merc_syntax::ActionName;
    use merc_syntax::Span;

    use super::*;

    fn action(name: &str) -> MultiActionName {
        multi(&[name])
    }

    fn multi(names: &[&str]) -> MultiActionName {
        MultiActionName::from_names(names.iter().map(|name| name.to_string()).collect())
    }

    fn set(actions: impl IntoIterator<Item = MultiActionName>) -> HashSet<MultiActionName> {
        actions.into_iter().collect()
    }

    fn name(text: &str) -> ActionName {
        respan(Span::default(), text.to_string())
    }

    fn comm_expr(from: &[&str], to: &str) -> CommExpr {
        CommExpr::new(MultiActionLabel::new(from.iter().map(|n| name(n)).collect()), name(to))
    }

    /// The obviously-correct (no laziness, no early exit, no bound) reference
    /// implementation of the `||`/`|` cross product — what
    /// [`Alphabet::combined_with_limit`] should compute whenever its limit
    /// never actually gets hit.
    fn naive_combined(a: &HashSet<MultiActionName>, b: &HashSet<MultiActionName>) -> HashSet<MultiActionName> {
        let mut result = a.clone();
        result.extend(b.iter().cloned());
        for x in a {
            for y in b {
                result.insert(x.merged(y));
            }
        }
        result
    }

    #[test]
    fn test_combined_matches_the_naive_reference_when_unbounded() {
        let cases: Vec<(HashSet<MultiActionName>, HashSet<MultiActionName>)> = vec![
            (set([action("a")]), set([action("b")])),
            (set([action("a"), action("b")]), set([action("c")])),
            (
                set([action("a"), multi(&["a", "b"])]),
                set([action("b"), MultiActionName::tau()]),
            ),
            (set([MultiActionName::tau()]), set([action("a")])),
            (set([]), set([action("a")])),
        ];

        for (a, b) in cases {
            let expected = Alphabet::Finite(naive_combined(&a, &b));
            let actual = Alphabet::Finite(a.clone()).combined_with_limit(Alphabet::Finite(b.clone()), usize::MAX);
            assert_eq!(actual, expected, "a={a:?}, b={b:?}");
        }
    }

    #[test]
    fn test_combined_stays_exact_below_the_limit() {
        let a = set((0..5).map(|i| action(&format!("a{i}"))));
        let b = set((0..5).map(|i| action(&format!("b{i}"))));
        let expected = Alphabet::Finite(naive_combined(&a, &b));

        // 5 + 5 + 5*5 = 35 distinct multi-actions: comfortably under 100.
        assert_eq!(
            Alphabet::Finite(a).combined_with_limit(Alphabet::Finite(b), 100),
            expected
        );
    }

    #[test]
    fn test_combined_gives_up_above_the_limit_instead_of_returning_a_wrong_answer() {
        let a = set((0..10).map(|i| action(&format!("a{i}"))));
        let b = set((0..10).map(|i| action(&format!("b{i}"))));

        // 10 * 10 = 100 merged pairs alone already exceeds a limit of 50, and
        // every one of those 100 pairs is a distinct multi-action (10
        // distinct names on each side), so this cannot come out `Finite`.
        assert_eq!(
            Alphabet::Finite(a).combined_with_limit(Alphabet::Finite(b), 50),
            Alphabet::Unknown
        );
    }

    #[test]
    fn test_combined_with_unknown_operand_is_always_unknown() {
        let a = Alphabet::Finite(set([action("a")]));
        assert_eq!(a.clone().combined(Alphabet::Unknown), Alphabet::Unknown);
        assert_eq!(Alphabet::Unknown.combined(a), Alphabet::Unknown);
    }

    #[test]
    fn test_with_communications_keeps_the_original_multi_actions() {
        // mCRL2's `comm` *adds* newly derived multi-actions to the input set,
        // it never removes the ones that don't match any expression.
        let alphabet = Alphabet::Finite(set([action("d")]));
        assert_eq!(
            alphabet.with_communications(&[comm_expr(&["a", "b"], "c")]),
            Alphabet::Finite(set([action("d")]))
        );
    }

    #[test]
    fn test_with_communications_fires_once_for_a_single_match() {
        let alphabet = Alphabet::Finite(set([multi(&["a", "b"])]));
        let result = alphabet.with_communications(&[comm_expr(&["a", "b"], "c")]);
        assert_eq!(result, Alphabet::Finite(set([multi(&["a", "b"]), action("c")])));
    }

    #[test]
    fn test_with_communications_repeats_within_a_single_multi_action() {
        // Ported from mCRL2's `apply_comm`: a multi-action containing the
        // left-hand side *twice* communicates twice, not once.
        let alphabet = Alphabet::Finite(set([multi(&["a", "a", "b", "b"])]));
        let result = alphabet.with_communications(&[comm_expr(&["a", "b"], "c")]);
        match result {
            Alphabet::Finite(set) => {
                assert!(
                    set.contains(&multi(&["a", "a", "b", "b"])),
                    "original multi-action is kept"
                );
                assert!(set.contains(&multi(&["a", "b", "c"])), "one communication fired");
                assert!(set.contains(&multi(&["c", "c"])), "both communications fired");
            }
            Alphabet::Unknown => panic!("expected a finite result"),
        }
    }

    #[test]
    fn test_with_communications_applies_expressions_in_order() {
        // The result of the first expression is visible to the second.
        let alphabet = Alphabet::Finite(set([multi(&["a", "b"])]));
        let comm = [comm_expr(&["a", "b"], "c"), comm_expr(&["c", "d"], "e")];
        let with_first_only = alphabet.clone().with_communications(&comm[..1]);
        assert_eq!(
            with_first_only,
            Alphabet::Finite(set([multi(&["a", "b"]), action("c")]))
        );

        let alphabet = Alphabet::Finite(set([multi(&["a", "b", "d"])]));
        let result = alphabet.with_communications(&comm);
        match result {
            Alphabet::Finite(set) => assert!(
                set.contains(&action("e")),
                "the second expression should fire on the multi-action the first one produced: {set:?}"
            ),
            Alphabet::Unknown => panic!("expected a finite result"),
        }
    }
}
