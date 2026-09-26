use std::collections::HashMap;

use ena::unify::InPlace;
use ena::unify::InPlaceUnificationTable;
use ena::unify::NoError;
use ena::unify::Snapshot;
use ena::unify::UnifyKey;
use ena::unify::UnifyValue;
use merc_syntax::ComplexSort;
use merc_syntax::Sort;
use merc_utilities::TagIndex;

use crate::ResolvedSort;
use crate::ResolvedSortId;
use crate::SortInterner;
use crate::number_generality;
use crate::number_sort_from_generality;

/// A unification variable, the [UnifyKey] of the underlying `ena` table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SortVar(u32);

impl UnifyKey for SortVar {
    type Value = SortVarValue;

    fn index(&self) -> u32 {
        self.0
    }

    fn from_index(index: u32) -> Self {
        SortVar(index)
    }

    fn tag() -> &'static str {
        "SortVar"
    }
}

/// The binding of a unification variable: `None` while free, `Some` once bound
/// to a sort.
#[derive(Clone, Debug)]
pub(crate) struct SortVarValue(Option<InferSortId>);

impl UnifyValue for SortVarValue {
    type Error = NoError;

    fn unify_values(lhs: &Self, rhs: &Self) -> Result<Self, NoError> {
        // [Unifier::unify] only merges two variables when both are unbound, so
        // at most one side carries a binding and keeping the first `Some` never
        // discards information.
        debug_assert!(
            lhs.0.is_none() || rhs.0.is_none(),
            "two bound variables must be unified structurally before merging"
        );
        Ok(SortVarValue(lhs.0.or(rhs.0)))
    }
}

/// A unique type for sorts built during inference.
pub(crate) struct InferSortTag;

/// An index into the arena of a [Unifier], identifying a sort under inference.
///
/// Unlike [ResolvedSortId], these are not interned: two different ids may
/// denote the same sort, and equality of sorts is decided by [Unifier::unify].
pub(crate) type InferSortId = TagIndex<usize, InferSortTag>;

/// A sort that may still contain unification variables.
///
/// Fully-known sorts appear as a single [InferSort::Resolved] leaf; structure
/// is only spelled out around the variables that inference still has to solve,
/// such as `List(?t)` for an empty-list literal.
#[derive(Clone, Debug)]
pub(crate) enum InferSort {
    /// A fully resolved sort, interned in the [SortInterner].
    Resolved(ResolvedSortId),
    /// A container sort whose element sort may contain variables.
    Generic { op: ComplexSort, subsort: InferSortId },
    /// A function sort whose argument or result sorts may contain variables.
    Function {
        domain: Vec<InferSortId>,
        range: InferSortId,
    },
    /// A unification variable.
    Var(SortVar),
}

/// A checkpoint of the variable bindings of a [Unifier], for backtracking.
///
/// Rolling back frees the variables bound since the checkpoint. Creating a
/// variable between a snapshot and its rollback is forbidden (and asserted):
/// the rollback would destroy it, leaving any [InferSortId] that mentions it
/// dangling.
pub(crate) struct UnifierSnapshot {
    snapshot: Snapshot<InPlace<SortVar>>,
    /// The number of variables at the checkpoint, to assert that no variable
    /// created after the snapshot outlives the rollback.
    variables: usize,
}

/// Solves sort equality constraints by structural unification, backed by
/// `ena`'s union-find table.
///
/// Sorts under inference live in an append-only arena; only the variable
/// bindings participate in [Unifier::snapshot] / [Unifier::rollback_to], so
/// arena nodes created inside a rolled-back branch remain as harmless garbage.
pub(crate) struct Unifier {
    table: InPlaceUnificationTable<SortVar>,
    arena: Vec<InferSort>,
    /// Memoizes [Unifier::resolved_node] so repeated references to the same
    /// resolved sort share one arena node.
    resolved_nodes: HashMap<ResolvedSortId, InferSortId>,
}

impl Unifier {
    pub(crate) fn new() -> Self {
        Unifier {
            table: InPlaceUnificationTable::new(),
            arena: Vec::new(),
            resolved_nodes: HashMap::new(),
        }
    }

    fn push(&mut self, sort: InferSort) -> InferSortId {
        let id = InferSortId::new(self.arena.len());
        self.arena.push(sort);
        id
    }

    /// Creates a fresh, unbound unification variable.
    pub(crate) fn fresh_var(&mut self) -> InferSortId {
        let var = self.table.new_key(SortVarValue(None));
        self.push(InferSort::Var(var))
    }

    /// The arena node denoting the fully resolved sort `id`.
    pub(crate) fn resolved_node(&mut self, id: ResolvedSortId) -> InferSortId {
        if let Some(node) = self.resolved_nodes.get(&id) {
            return *node;
        }
        let node = self.push(InferSort::Resolved(id));
        self.resolved_nodes.insert(id, node);
        node
    }

    /// Builds the container sort `op(subsort)`.
    pub(crate) fn generic(&mut self, op: ComplexSort, subsort: InferSortId) -> InferSortId {
        self.push(InferSort::Generic { op, subsort })
    }

    /// Builds the function sort `domain -> range`.
    pub(crate) fn function(&mut self, domain: Vec<InferSortId>, range: InferSortId) -> InferSortId {
        self.push(InferSort::Function { domain, range })
    }

    /// Follows variable bindings until the head of `id` is either a non-variable
    /// node or an unbound variable.
    fn shallow_normalize(&mut self, mut id: InferSortId) -> InferSortId {
        loop {
            let InferSort::Var(var) = self.arena[id] else {
                return id;
            };
            match self.table.probe_value(var).0 {
                Some(next) => id = next,
                None => return id,
            }
        }
    }

    /// The head node of `id` after following variable bindings: an unbound
    /// [InferSort::Var] or a non-variable node, whose sub-sorts may in turn be
    /// bound variables.
    pub(crate) fn head(&mut self, id: InferSortId) -> InferSort {
        let id = self.shallow_normalize(id);
        self.arena[id].clone()
    }

    /// The union-find root of `id` when it is still an unbound variable, or
    /// `None` once it is bound to a concrete sort. Two nodes eagerly unified to
    /// the same free variable (the shared parameter of a scheme, the two
    /// operands of a comparison, the branches of `if`, a set/bag element, the
    /// equation's LHS/RHS join) return the same root, so a caller can group the
    /// `Sub`s that widen into one join target.
    pub(crate) fn free_root(&mut self, id: InferSortId) -> Option<u32> {
        let id = self.shallow_normalize(id);
        match self.arena[id] {
            InferSort::Var(var) => Some(self.table.find(var).index()),
            _ => None,
        }
    }

    /// Makes `lhs` and `rhs` denote the same sort, binding variables as needed,
    /// and returns whether they are unifiable.
    ///
    /// On failure the table may retain bindings made before the mismatch was
    /// found; callers backtrack over failed attempts with [Unifier::snapshot]
    /// and [Unifier::rollback_to].
    pub(crate) fn unify(&mut self, interner: &SortInterner, lhs: InferSortId, rhs: InferSortId) -> bool {
        let lhs = self.shallow_normalize(lhs);
        let rhs = self.shallow_normalize(rhs);
        if lhs == rhs {
            return true;
        }

        match (self.arena[lhs].clone(), self.arena[rhs].clone()) {
            (InferSort::Var(lhs_var), InferSort::Var(rhs_var)) => {
                // Both are unbound after normalization, so no bindings can
                // conflict when the variables are merged.
                self.table
                    .unify_var_var(lhs_var, rhs_var)
                    .expect("unifying two unbound variables cannot fail");
                true
            }
            (InferSort::Var(var), _) => self.bind(var, rhs),
            (_, InferSort::Var(var)) => self.bind(var, lhs),
            // Interning is canonical, so two resolved sorts are equal exactly
            // when their ids are.
            (InferSort::Resolved(lhs_id), InferSort::Resolved(rhs_id)) => lhs_id == rhs_id,
            (InferSort::Resolved(resolved), InferSort::Generic { op, subsort })
            | (InferSort::Generic { op, subsort }, InferSort::Resolved(resolved)) => match interner.get(resolved) {
                ResolvedSort::Container {
                    op: resolved_op,
                    subsort: resolved_subsort,
                } if *resolved_op == op => {
                    let resolved_subsort = *resolved_subsort;
                    let subsort_node = self.resolved_node(resolved_subsort);
                    self.unify(interner, subsort_node, subsort)
                }
                _ => false,
            },
            (InferSort::Resolved(resolved), InferSort::Function { domain, range })
            | (InferSort::Function { domain, range }, InferSort::Resolved(resolved)) => match interner.get(resolved) {
                ResolvedSort::Function {
                    domain: resolved_domain,
                    range: resolved_range,
                } if resolved_domain.len() == domain.len() => {
                    let resolved_domain = resolved_domain.clone();
                    let resolved_range = *resolved_range;
                    for (resolved_argument, argument) in resolved_domain.into_iter().zip(domain) {
                        let argument_node = self.resolved_node(resolved_argument);
                        if !self.unify(interner, argument_node, argument) {
                            return false;
                        }
                    }
                    let range_node = self.resolved_node(resolved_range);
                    self.unify(interner, range_node, range)
                }
                _ => false,
            },
            (
                InferSort::Generic {
                    op: lhs_op,
                    subsort: lhs_subsort,
                },
                InferSort::Generic {
                    op: rhs_op,
                    subsort: rhs_subsort,
                },
            ) => lhs_op == rhs_op && self.unify(interner, lhs_subsort, rhs_subsort),
            (
                InferSort::Function {
                    domain: lhs_domain,
                    range: lhs_range,
                },
                InferSort::Function {
                    domain: rhs_domain,
                    range: rhs_range,
                },
            ) => {
                if lhs_domain.len() != rhs_domain.len() {
                    return false;
                }
                for (lhs_argument, rhs_argument) in lhs_domain.into_iter().zip(rhs_domain) {
                    if !self.unify(interner, lhs_argument, rhs_argument) {
                        return false;
                    }
                }
                self.unify(interner, lhs_range, rhs_range)
            }
            (InferSort::Generic { .. }, InferSort::Function { .. })
            | (InferSort::Function { .. }, InferSort::Generic { .. }) => false,
        }
    }

    /// Binds `var` to `value` after the occurs check, which rejects the cyclic
    /// (infinite) sort a binding like `?t := List(?t)` would create.
    fn bind(&mut self, var: SortVar, value: InferSortId) -> bool {
        if self.occurs(var, value) {
            return false;
        }
        self.table
            .unify_var_value(var, SortVarValue(Some(value)))
            .expect("binding an unbound variable cannot fail");
        true
    }

    /// Returns whether `var` occurs in the sort denoted by `id`.
    fn occurs(&mut self, var: SortVar, id: InferSortId) -> bool {
        let id = self.shallow_normalize(id);
        match self.arena[id].clone() {
            InferSort::Var(other) => self.table.unioned(var, other),
            InferSort::Resolved(_) => false,
            InferSort::Generic { op: _, subsort } => self.occurs(var, subsort),
            InferSort::Function { domain, range } => {
                domain.into_iter().any(|argument| self.occurs(var, argument)) || self.occurs(var, range)
            }
        }
    }

    /// Extracts the fully resolved sort denoted by `id`, or `None` when a free
    /// variable remains, i.e. the sort is underdetermined.
    pub(crate) fn resolve(&mut self, interner: &mut SortInterner, id: InferSortId) -> Option<ResolvedSortId> {
        let id = self.shallow_normalize(id);
        match self.arena[id].clone() {
            InferSort::Var(_) => None,
            InferSort::Resolved(resolved) => Some(resolved),
            InferSort::Generic { op, subsort } => {
                let subsort = self.resolve(interner, subsort)?;
                Some(interner.generic(op, subsort))
            }
            InferSort::Function { domain, range } => {
                let mut resolved_domain = Vec::with_capacity(domain.len());
                for argument in domain {
                    resolved_domain.push(self.resolve(interner, argument)?);
                }
                let range = self.resolve(interner, range)?;
                Some(interner.function(resolved_domain, range))
            }
        }
    }

    /// Like [`Unifier::resolve`] but substitutes any remaining free variable with
    /// `default` rather than returning `None`. Used by the solver to accept
    /// equations whose auxiliary sorts (e.g. the element sort of an empty-list
    /// literal in `n = #[]`) are never constrained.
    pub(crate) fn resolve_or_default(
        &mut self,
        interner: &mut SortInterner,
        id: InferSortId,
        default: ResolvedSortId,
    ) -> ResolvedSortId {
        let id = self.shallow_normalize(id);
        match self.arena[id].clone() {
            InferSort::Var(_) => default,
            InferSort::Resolved(resolved) => resolved,
            InferSort::Generic { op, subsort } => {
                let subsort = self.resolve_or_default(interner, subsort, default);
                interner.generic(op, subsort)
            }
            InferSort::Function { domain, range } => {
                let domain = domain
                    .iter()
                    .map(|&arg| self.resolve_or_default(interner, arg, default))
                    .collect();
                let range = self.resolve_or_default(interner, range, default);
                interner.function(domain, range)
            }
        }
    }

    /// The strict supersorts of `id` in ascending distance (`Pos` yields
    /// `[Nat, Int, Real]`), or `None` for an unbound variable, whose supersorts
    /// cannot be enumerated. Only the head constructor is widened: `Nat` has
    /// supersorts, `List(Nat)` has none.
    pub(crate) fn strict_super_sorts(
        &mut self,
        interner: &mut SortInterner,
        id: InferSortId,
    ) -> Option<Vec<InferSortId>> {
        self.strict_related_sorts(interner, id, Direction::Super)
    }

    /// The strict subsorts of `id` in ascending distance (`Real` yields
    /// `[Int, Nat, Pos]`), or `None` for an unbound variable.
    pub(crate) fn strict_sub_sorts(
        &mut self,
        interner: &mut SortInterner,
        id: InferSortId,
    ) -> Option<Vec<InferSortId>> {
        self.strict_related_sorts(interner, id, Direction::Sub)
    }

    fn strict_related_sorts(
        &mut self,
        interner: &mut SortInterner,
        id: InferSortId,
        direction: Direction,
    ) -> Option<Vec<InferSortId>> {
        let id = self.shallow_normalize(id);
        match self.arena[id].clone() {
            InferSort::Var(_) => None,
            InferSort::Resolved(resolved) => Some(match interner.get(resolved).clone() {
                ResolvedSort::Primitive(sort) => related_numbers(sort, direction)
                    .into_iter()
                    .map(|sort| {
                        let resolved = interner.primitive(sort);
                        self.resolved_node(resolved)
                    })
                    .collect(),
                ResolvedSort::Container { op, subsort } => match related_container(op, direction) {
                    Some(op) => {
                        let resolved = interner.generic(op, subsort);
                        vec![self.resolved_node(resolved)]
                    }
                    None => Vec::new(),
                },
                _ => Vec::new(),
            }),
            InferSort::Generic { op, subsort } => Some(match related_container(op, direction) {
                Some(op) => vec![self.generic(op, subsort)],
                None => Vec::new(),
            }),
            // A function sort itself never widens: there is no way to lower a
            // materialized coercion between two function *values* of
            // different sort (unlike a number or a container, `coerce` has no
            // wrapper to build here). Widening a lambda's range instead
            // happens one level down, at its *body*, which is a plain value
            // and so coercible the normal way -- see the `Lambda` case in
            // `ConstraintGenerator::visit` and `Lowering::lower_lambda`.
            InferSort::Function { .. } => Some(Vec::new()),
        }
    }

    /// Starts a checkpoint; every snapshot must be finished with
    /// [Unifier::rollback_to], innermost first.
    pub(crate) fn snapshot(&mut self) -> UnifierSnapshot {
        UnifierSnapshot {
            snapshot: self.table.snapshot(),
            variables: self.table.len(),
        }
    }

    /// Undoes all bindings made since the snapshot was taken.
    pub(crate) fn rollback_to(&mut self, snapshot: UnifierSnapshot) {
        debug_assert_eq!(
            self.table.len(),
            snapshot.variables,
            "no variables may be created between a snapshot and its rollback"
        );
        self.table.rollback_to(snapshot.snapshot);
    }
}

#[derive(Clone, Copy)]
enum Direction {
    Super,
    Sub,
}

/// The number sorts strictly above or below `sort`, in ascending distance.
fn related_numbers(sort: Sort, direction: Direction) -> Vec<Sort> {
    let Some(generality) = number_generality(sort) else {
        return Vec::new();
    };
    match direction {
        Direction::Super => (generality + 1..=3).map(number_sort_from_generality).collect(),
        Direction::Sub => (0..generality).rev().map(number_sort_from_generality).collect(),
    }
}

/// The container constructor strictly above or below `op` in the finiteness
/// ordering `FSet <= Set`, `FBag <= Bag`.
fn related_container(op: ComplexSort, direction: Direction) -> Option<ComplexSort> {
    match direction {
        Direction::Super => match op {
            ComplexSort::FSet => Some(ComplexSort::Set),
            ComplexSort::FBag => Some(ComplexSort::Bag),
            _ => None,
        },
        Direction::Sub => match op {
            ComplexSort::Set => Some(ComplexSort::FSet),
            ComplexSort::Bag => Some(ComplexSort::FBag),
            _ => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use merc_syntax::ComplexSort;

    use crate::SortInterner;
    use crate::Unifier;

    #[test]
    fn test_unify_variable_with_resolved() {
        let mut interner = SortInterner::new();
        let mut unifier = Unifier::new();

        let var = unifier.fresh_var();
        let nat = unifier.resolved_node(interner.nat_sort());
        assert!(unifier.unify(&interner, var, nat));
        assert_eq!(unifier.resolve(&mut interner, var), Some(interner.nat_sort()));
    }

    #[test]
    fn test_unify_variables_transitively() {
        let mut interner = SortInterner::new();
        let mut unifier = Unifier::new();

        // Merging two free variables and later binding one binds both.
        let first = unifier.fresh_var();
        let second = unifier.fresh_var();
        assert!(unifier.unify(&interner, first, second));
        assert_eq!(unifier.resolve(&mut interner, first), None);

        let bool_sort = unifier.resolved_node(interner.bool_sort());
        assert!(unifier.unify(&interner, second, bool_sort));
        assert_eq!(unifier.resolve(&mut interner, first), Some(interner.bool_sort()));
    }

    #[test]
    fn test_unify_resolved_against_structure() {
        let mut interner = SortInterner::new();
        let mut unifier = Unifier::new();

        // `List(Nat)` as a resolved leaf against `List(?t)` binds `?t := Nat`.
        let nat_list = interner.generic(ComplexSort::List, interner.nat_sort());
        let resolved = unifier.resolved_node(nat_list);
        let element = unifier.fresh_var();
        let structural = unifier.generic(ComplexSort::List, element);

        assert!(unifier.unify(&interner, resolved, structural));
        assert_eq!(unifier.resolve(&mut interner, element), Some(interner.nat_sort()));
        assert_eq!(unifier.resolve(&mut interner, structural), Some(nat_list));
    }

    #[test]
    fn test_unify_resolved_against_function_structure() {
        let mut interner = SortInterner::new();
        let mut unifier = Unifier::new();

        let function = interner.function(vec![interner.nat_sort(), interner.bool_sort()], interner.real_sort());
        let resolved = unifier.resolved_node(function);

        let first = unifier.fresh_var();
        let second = unifier.fresh_var();
        let range = unifier.fresh_var();
        let structural = unifier.function(vec![first, second], range);

        assert!(unifier.unify(&interner, resolved, structural));
        assert_eq!(unifier.resolve(&mut interner, first), Some(interner.nat_sort()));
        assert_eq!(unifier.resolve(&mut interner, second), Some(interner.bool_sort()));
        assert_eq!(unifier.resolve(&mut interner, range), Some(interner.real_sort()));
    }

    #[test]
    fn test_unify_rejects_mismatches() {
        let interner = SortInterner::new();
        let mut unifier = Unifier::new();

        let nat = unifier.resolved_node(interner.nat_sort());
        let bool_sort = unifier.resolved_node(interner.bool_sort());
        assert!(!unifier.unify(&interner, nat, bool_sort));

        // Distinct container constructors do not unify, even with FSet <= Set.
        let element = unifier.fresh_var();
        let fset = unifier.generic(ComplexSort::FSet, element);
        let set = unifier.generic(ComplexSort::Set, element);
        assert!(!unifier.unify(&interner, fset, set));

        // Function arity is part of the sort.
        let unary = unifier.function(vec![nat], bool_sort);
        let binary = unifier.function(vec![nat, nat], bool_sort);
        assert!(!unifier.unify(&interner, unary, binary));

        let list = unifier.generic(ComplexSort::List, nat);
        assert!(!unifier.unify(&interner, list, unary));
    }

    #[test]
    fn test_occurs_check_rejects_cyclic_sort() {
        let interner = SortInterner::new();
        let mut unifier = Unifier::new();

        let var = unifier.fresh_var();
        let list = unifier.generic(ComplexSort::List, var);
        assert!(!unifier.unify(&interner, var, list));

        // Also through a merged variable: ?a = ?b, then ?a with List(?b).
        let alias = unifier.fresh_var();
        assert!(unifier.unify(&interner, var, alias));
        let alias_list = unifier.generic(ComplexSort::List, alias);
        assert!(!unifier.unify(&interner, var, alias_list));
    }

    #[test]
    fn test_rollback_frees_bindings() {
        let mut interner = SortInterner::new();
        let mut unifier = Unifier::new();

        let var = unifier.fresh_var();
        let nat = unifier.resolved_node(interner.nat_sort());

        let snapshot = unifier.snapshot();
        assert!(unifier.unify(&interner, var, nat));
        unifier.rollback_to(snapshot);
        assert_eq!(unifier.resolve(&mut interner, var), None);

        // The variable is free again, so a different binding must succeed.
        let bool_sort = unifier.resolved_node(interner.bool_sort());
        assert!(unifier.unify(&interner, var, bool_sort));
        assert_eq!(unifier.resolve(&mut interner, var), Some(interner.bool_sort()));
    }

    #[test]
    fn test_resolve_extracts_nested_structure() {
        let mut interner = SortInterner::new();
        let mut unifier = Unifier::new();

        // ?f := ?t -> List(?t) with ?t := Pos resolves to Pos -> List(Pos).
        let element = unifier.fresh_var();
        let list = unifier.generic(ComplexSort::List, element);
        let function = unifier.function(vec![element], list);
        assert_eq!(unifier.resolve(&mut interner, function), None);

        let pos = unifier.resolved_node(interner.pos_sort());
        assert!(unifier.unify(&interner, element, pos));

        let pos_list = interner.generic(ComplexSort::List, interner.pos_sort());
        let expected = interner.function(vec![interner.pos_sort()], pos_list);
        assert_eq!(unifier.resolve(&mut interner, function), Some(expected));
    }

    #[test]
    fn test_strict_super_sorts() {
        let mut interner = SortInterner::new();
        let mut unifier = Unifier::new();

        let pos = unifier.resolved_node(interner.pos_sort());
        let supers = unifier.strict_super_sorts(&mut interner, pos).unwrap();
        let resolved: Vec<_> = supers
            .into_iter()
            .map(|id| unifier.resolve(&mut interner, id).unwrap())
            .collect();
        assert_eq!(
            resolved,
            vec![interner.nat_sort(), interner.int_sort(), interner.real_sort()]
        );

        let bool_sort = unifier.resolved_node(interner.bool_sort());
        assert_eq!(unifier.strict_super_sorts(&mut interner, bool_sort), Some(Vec::new()));

        let var = unifier.fresh_var();
        assert_eq!(unifier.strict_super_sorts(&mut interner, var), None);

        // The head constructor of a container is widened, its element is not.
        let fset = unifier.generic(ComplexSort::FSet, var);
        let supers = unifier.strict_super_sorts(&mut interner, fset).unwrap();
        assert_eq!(supers.len(), 1);
        let nat = unifier.resolved_node(interner.nat_sort());
        assert!(unifier.unify(&interner, var, nat));
        let nat_set = interner.generic(ComplexSort::Set, interner.nat_sort());
        assert_eq!(unifier.resolve(&mut interner, supers[0]), Some(nat_set));
    }

    #[test]
    fn test_strict_sub_sorts() {
        let mut interner = SortInterner::new();
        let mut unifier = Unifier::new();

        let real = unifier.resolved_node(interner.real_sort());
        let subs = unifier.strict_sub_sorts(&mut interner, real).unwrap();
        let resolved: Vec<_> = subs
            .into_iter()
            .map(|id| unifier.resolve(&mut interner, id).unwrap())
            .collect();
        assert_eq!(
            resolved,
            vec![interner.int_sort(), interner.nat_sort(), interner.pos_sort()]
        );

        let nat_bag = interner.generic(ComplexSort::Bag, interner.nat_sort());
        let bag = unifier.resolved_node(nat_bag);
        let subs = unifier.strict_sub_sorts(&mut interner, bag).unwrap();
        assert_eq!(subs.len(), 1);
        let nat_fbag = interner.generic(ComplexSort::FBag, interner.nat_sort());
        assert_eq!(unifier.resolve(&mut interner, subs[0]), Some(nat_fbag));

        let pos = unifier.resolved_node(interner.pos_sort());
        assert_eq!(unifier.strict_sub_sorts(&mut interner, pos), Some(Vec::new()));
    }

    #[test]
    fn test_unify_bound_variables_structurally() {
        let mut interner = SortInterner::new();
        let mut unifier = Unifier::new();

        // Two variables bound to compatible structures unify by unifying the
        // structures underneath, binding the nested variables.
        let first_element = unifier.fresh_var();
        let first = unifier.fresh_var();
        let first_list = unifier.generic(ComplexSort::List, first_element);
        assert!(unifier.unify(&interner, first, first_list));

        let second = unifier.fresh_var();
        let nat_list = interner.generic(ComplexSort::List, interner.nat_sort());
        let resolved_list = unifier.resolved_node(nat_list);
        assert!(unifier.unify(&interner, second, resolved_list));

        assert!(unifier.unify(&interner, first, second));
        assert_eq!(unifier.resolve(&mut interner, first_element), Some(interner.nat_sort()));
    }
}
