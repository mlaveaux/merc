use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

use merc_syntax::ComplexSort;
use merc_syntax::Sort;
use merc_syntax::SortId;
use merc_syntax::TypeVarId;
use merc_syntax::UntypedDataSpecification;
use merc_utilities::TagIndex;

use crate::TypeCheckContext;

/// A unique type for interned resolved sorts.
pub(crate) struct ResolvedSortTag;

/// An index into a [SortInterner], identifying a unique resolved sort.
///
/// Because sorts are interned, two ids are equal if and only if the sorts they
/// denote are equal, so equality of sorts is a comparison of two integers.
pub(crate) type ResolvedSortId = TagIndex<usize, ResolvedSortTag>;

/// A type in the mCRL2 type system, called a *sort*.
///
/// Sub-sorts are stored as [ResolvedSortId] indices into the [SortInterner]
/// rather than by value, so a `ResolvedSort` is small and structural equality
/// coincides with id equality.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ResolvedSort {
    /// The sort with a single element, used internally for the result of an
    /// action. It has no surface syntax, which is why it is a variant here
    /// rather than a member of [merc_syntax::Sort].
    Unit,
    /// A built-in primitive sort such as `Bool` or `Nat`.
    Primitive(Sort),
    /// A container sort such as `List(S)` or `Set(S)`.
    Container { op: ComplexSort, subsort: ResolvedSortId },
    /// A function sort `A_0 # ... # A_n -> B`.
    Function {
        domain: Vec<ResolvedSortId>,
        range: ResolvedSortId,
    },
    /// A user-defined (nominal) sort, identified by the declaration it resolves
    /// to. Two `Def` sorts are equal only when they refer to the same
    /// declaration, and otherwise incomparable.
    Def(SortId),
    /// A bound type variable, scoped to the polymorphic specification that
    /// introduces it.
    TypeVar(TypeVarId),
}

/// Folds two sub-relation results the way mCRL2 book Definition 15.1.8's `⊆`
/// combines independent positions: `Equal` is the unit (a position that
/// agrees contributes nothing), two equal non-`Equal` orderings agree, and a
/// `Less`/`Greater` clash — or either side being incomparable — makes the
/// whole comparison incomparable.
fn combine(lhs: Option<Ordering>, rhs: Option<Ordering>) -> Option<Ordering> {
    match (lhs?, rhs?) {
        (Ordering::Equal, other) | (other, Ordering::Equal) => Some(other),
        (lhs, rhs) if lhs == rhs => Some(lhs),
        _ => None,
    }
}

/// Returns the generality of a number sort (`Pos` = 0, `Nat` = 1, `Int` = 2,
/// `Real` = 3), or `None` for the non-number sorts.
pub(crate) fn number_generality(sort: Sort) -> Option<u32> {
    match sort {
        Sort::Pos => Some(0),
        Sort::Nat => Some(1),
        Sort::Int => Some(2),
        Sort::Real => Some(3),
        Sort::Bool => None,
    }
}

/// The inverse of [number_generality].
pub(crate) fn number_sort_from_generality(generality: u32) -> Sort {
    match generality {
        0 => Sort::Pos,
        1 => Sort::Nat,
        2 => Sort::Int,
        3 => Sort::Real,
        _ => panic!("{generality} is not a number sort generality"),
    }
}

/// Renders a resolved sort for debug logging. Nominal sorts take their name
/// from [TypeCheckContext::sort_name] (a user or system-internal sort such as
/// `@NatPair`), falling back to a bare index.
pub(crate) struct DisplaySortContext<'a> {
    ctx: &'a TypeCheckContext,
    spec: &'a UntypedDataSpecification,
    id: ResolvedSortId,
}

impl<'a> DisplaySortContext<'a> {
    pub(crate) fn new(ctx: &'a TypeCheckContext, spec: &'a UntypedDataSpecification, id: ResolvedSortId) -> Self {
        DisplaySortContext { ctx, spec, id }
    }

    /// A [DisplaySortContext] for a sub-sort of `self`, reusing the same context
    /// and specification.
    fn sub(&self, id: ResolvedSortId) -> Self {
        DisplaySortContext {
            ctx: self.ctx,
            spec: self.spec,
            id,
        }
    }
}

impl fmt::Display for DisplaySortContext<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ctx.sorts.get(self.id) {
            ResolvedSort::Unit => write!(f, "@Unit"),
            ResolvedSort::Primitive(sort) => write!(f, "{sort}"),
            ResolvedSort::Container { op, subsort } => {
                write!(f, "{op}({})", self.sub(*subsort))
            }
            ResolvedSort::Function { domain, range } => {
                let domain: Vec<String> = domain.iter().map(|sort| self.sub(*sort).to_string()).collect();
                write!(f, "{} -> {}", domain.join(" # "), self.sub(*range))
            }
            ResolvedSort::Def(def) => {
                write!(f, "{}", self.ctx.sort_display_name(self.spec, *def))
            }
            // Debug logging only (per this struct's doc comment).
            ResolvedSort::TypeVar(id) => write!(f, "@S_{id}"),
        }
    }
}

/// Orders the primitive sorts by the number-sort hierarchy. Distinct sorts that
/// are not both numbers are incomparable.
fn primitive_partial_cmp(lhs: Sort, rhs: Sort) -> Option<Ordering> {
    if lhs == rhs {
        Some(Ordering::Equal)
    } else if let (Some(lhs), Some(rhs)) = (number_generality(lhs), number_generality(rhs)) {
        lhs.partial_cmp(&rhs)
    } else {
        None
    }
}

/// Panics: neither `Unit` nor `TypeVar` ever denotes a data-expression's own resolved sort..
pub(crate) fn unreachable_not_a_value_sort(variant: &str) -> ! {
    unreachable!("{variant} never denotes a data-expression's own resolved sort")
}

/// Returns whether the container constructor is `Set` or `FSet`.
fn is_any_set(op: ComplexSort) -> bool {
    matches!(op, ComplexSort::Set | ComplexSort::FSet)
}

/// Returns whether the container constructor is `Bag` or `FBag`.
fn is_any_bag(op: ComplexSort) -> bool {
    matches!(op, ComplexSort::Bag | ComplexSort::FBag)
}

/// Orders the container constructors by their finiteness marker: `FSet <= Set`
/// and `FBag <= Bag`. All other distinct constructors are incomparable.
fn generic_op_partial_cmp(lhs: ComplexSort, rhs: ComplexSort) -> Option<Ordering> {
    match (lhs, rhs) {
        (lhs, rhs) if lhs == rhs => Some(Ordering::Equal),
        (ComplexSort::FBag, ComplexSort::Bag) => Some(Ordering::Less),
        (ComplexSort::Bag, ComplexSort::FBag) => Some(Ordering::Greater),
        (ComplexSort::FSet, ComplexSort::Set) => Some(Ordering::Less),
        (ComplexSort::Set, ComplexSort::FSet) => Some(Ordering::Greater),
        _ => None,
    }
}

/// The container constructor of the least upper bound of `lhs` and `rhs` in
/// the finiteness ordering, or `None` when neither is a widening of the
/// other (a `Set`/`Bag` mismatch).
fn join_op(lhs: ComplexSort, rhs: ComplexSort) -> Option<ComplexSort> {
    match (lhs, rhs) {
        (lhs, rhs) if lhs == rhs => Some(lhs),
        (lhs, rhs) if is_any_bag(lhs) && is_any_bag(rhs) => Some(ComplexSort::Bag),
        (lhs, rhs) if is_any_set(lhs) && is_any_set(rhs) => Some(ComplexSort::Set),
        _ => None,
    }
}

/// The dual of [join_op]: the container constructor of the greatest lower
/// bound.
fn meet_op(lhs: ComplexSort, rhs: ComplexSort) -> Option<ComplexSort> {
    match (lhs, rhs) {
        (lhs, rhs) if lhs == rhs => Some(lhs),
        (lhs, rhs) if is_any_bag(lhs) && is_any_bag(rhs) => Some(ComplexSort::FBag),
        (lhs, rhs) if is_any_set(lhs) && is_any_set(rhs) => Some(ComplexSort::FSet),
        _ => None,
    }
}

/// Interns [ResolvedSort]s so that equality is an integer comparison and each
/// distinct sort is stored once.
///
/// The primitive sorts are interned eagerly and returned by the `*_sort`
/// accessors; every other sort is created on demand through [SortInterner::generic],
/// [SortInterner::function] and [SortInterner::def].
#[derive(Clone)]
pub(crate) struct SortInterner {
    arena: Vec<ResolvedSort>,
    dedup: HashMap<ResolvedSort, ResolvedSortId>,

    unit_sort: ResolvedSortId,
    bool_sort: ResolvedSortId,
    pos_sort: ResolvedSortId,
    nat_sort: ResolvedSortId,
    int_sort: ResolvedSortId,
    real_sort: ResolvedSortId,
}

impl SortInterner {
    pub(crate) fn new() -> Self {
        let mut interner = SortInterner {
            arena: Vec::new(),
            dedup: HashMap::new(),
            unit_sort: ResolvedSortId::new(0),
            bool_sort: ResolvedSortId::new(0),
            pos_sort: ResolvedSortId::new(0),
            nat_sort: ResolvedSortId::new(0),
            int_sort: ResolvedSortId::new(0),
            real_sort: ResolvedSortId::new(0),
        };

        interner.unit_sort = interner.intern(ResolvedSort::Unit);
        interner.bool_sort = interner.intern(ResolvedSort::Primitive(Sort::Bool));
        interner.pos_sort = interner.intern(ResolvedSort::Primitive(Sort::Pos));
        interner.nat_sort = interner.intern(ResolvedSort::Primitive(Sort::Nat));
        interner.int_sort = interner.intern(ResolvedSort::Primitive(Sort::Int));
        interner.real_sort = interner.intern(ResolvedSort::Primitive(Sort::Real));

        interner
    }

    fn intern(&mut self, sort: ResolvedSort) -> ResolvedSortId {
        if let Some(id) = self.dedup.get(&sort) {
            return *id;
        }

        let id = ResolvedSortId::new(self.arena.len());
        self.arena.push(sort.clone());
        self.dedup.insert(sort, id);
        id
    }

    pub(crate) fn primitive(&self, sort: Sort) -> ResolvedSortId {
        match sort {
            Sort::Bool => self.bool_sort,
            Sort::Pos => self.pos_sort,
            Sort::Int => self.int_sort,
            Sort::Nat => self.nat_sort,
            Sort::Real => self.real_sort,
        }
    }

    /// Interns the container sort `op(subsort)`.
    pub(crate) fn generic(&mut self, op: ComplexSort, subsort: ResolvedSortId) -> ResolvedSortId {
        self.intern(ResolvedSort::Container { op, subsort })
    }

    /// Interns the function sort `domain -> range`.
    pub(crate) fn function(&mut self, domain: Vec<ResolvedSortId>, range: ResolvedSortId) -> ResolvedSortId {
        self.intern(ResolvedSort::Function { domain, range })
    }

    /// Interns the nominal sort for the given declaration.
    pub(crate) fn def(&mut self, def: SortId) -> ResolvedSortId {
        self.intern(ResolvedSort::Def(def))
    }

    /// Interns the bound type variable `id`. Two calls with the same `id`
    /// return the same [ResolvedSortId], which is what makes two occurrences
    /// of the same `type_var` inside one declaration denote the same sort.
    pub(crate) fn var(&mut self, id: TypeVarId) -> ResolvedSortId {
        self.intern(ResolvedSort::TypeVar(id))
    }
}

impl SortInterner {
    /// Returns the resolved sort denoted by an id.
    ///
    /// The id must have been produced by this same interner; ids from a
    /// different [SortInterner] index into an unrelated arena and would return a
    /// wrong sort or panic.
    pub(crate) fn get(&self, id: ResolvedSortId) -> &ResolvedSort {
        debug_assert!(
            *id < self.arena.len(),
            "id {id:?} does not originate from this interner"
        );
        &self.arena[*id]
    }

    // Renders the `Unit` sort and the `Int`/`Real` literals of an inserted
    // cast. Exercised by tests only for now.
    #[allow(dead_code)]
    pub(crate) fn unit_sort(&self) -> ResolvedSortId {
        self.unit_sort
    }

    pub(crate) fn bool_sort(&self) -> ResolvedSortId {
        self.bool_sort
    }

    pub(crate) fn pos_sort(&self) -> ResolvedSortId {
        self.pos_sort
    }

    pub(crate) fn nat_sort(&self) -> ResolvedSortId {
        self.nat_sort
    }

    #[allow(dead_code)]
    pub(crate) fn int_sort(&self) -> ResolvedSortId {
        self.int_sort
    }

    #[allow(dead_code)]
    pub(crate) fn real_sort(&self) -> ResolvedSortId {
        self.real_sort
    }

    /// Compares two sorts by the sub-sort ordering of mCRL2 book Definition
    /// 15.1.8: reflexive and transitive, generated by the number tower
    /// (`Pos ⊆ Nat ⊆ Int ⊆ Real`), the FSet/Set and FBag/Bag finiteness step,
    /// element-wise container covariance, and function-sort variance
    /// (contravariant domain, covariant range). `lowering.rs` and
    /// `process/check.rs` gate any use of this on
    /// [SortInterner::is_materializable] before treating a comparison as an
    /// accepted typing — this method alone says nothing about whether
    /// lowering can actually build the corresponding term.
    pub(crate) fn partial_cmp(&self, lhs: ResolvedSortId, rhs: ResolvedSortId) -> Option<Ordering> {
        if lhs == rhs {
            return Some(Ordering::Equal);
        }
        match (self.get(lhs), self.get(rhs)) {
            (ResolvedSort::Primitive(lhs), ResolvedSort::Primitive(rhs)) => primitive_partial_cmp(*lhs, *rhs),
            (
                ResolvedSort::Container {
                    op: lhs_op,
                    subsort: lhs_sub,
                },
                ResolvedSort::Container {
                    op: rhs_op,
                    subsort: rhs_sub,
                },
            ) => combine(
                generic_op_partial_cmp(*lhs_op, *rhs_op),
                self.partial_cmp(*lhs_sub, *rhs_sub),
            ),
            (
                ResolvedSort::Function {
                    domain: lhs_domain,
                    range: lhs_range,
                },
                ResolvedSort::Function {
                    domain: rhs_domain,
                    range: rhs_range,
                },
            ) if lhs_domain.len() == rhs_domain.len() => {
                let mut ordering = Some(Ordering::Equal);
                for (&lhs_arg, &rhs_arg) in lhs_domain.iter().zip(rhs_domain.iter()) {
                    // The domain is contravariant: swapping the operand order
                    // here.
                    ordering = combine(ordering, self.partial_cmp(rhs_arg, lhs_arg));
                }
                combine(ordering, self.partial_cmp(*lhs_range, *rhs_range))
            }
            _ => None,
        }
    }

    /// Finds the least common supersort of two sorts, or `None` when they are
    /// incomparable.
    ///
    /// This operation is commutative, associative and idempotent. It does not
    /// report errors, it simply returns `None`. As with [SortInterner::partial_cmp],
    /// a caller that will materialize the result must check
    /// [SortInterner::is_materializable] on every source first.
    pub(crate) fn join(&mut self, lhs: ResolvedSortId, rhs: ResolvedSortId) -> Option<ResolvedSortId> {
        if lhs == rhs {
            return Some(lhs);
        }

        match (self.get(lhs).clone(), self.get(rhs).clone()) {
            (ResolvedSort::Primitive(lhs), ResolvedSort::Primitive(rhs)) => {
                let lhs = number_generality(lhs)?;
                let rhs = number_generality(rhs)?;
                Some(self.primitive(number_sort_from_generality(lhs.max(rhs))))
            }
            (
                ResolvedSort::Container {
                    op: lhs_op,
                    subsort: lhs_sub,
                },
                ResolvedSort::Container {
                    op: rhs_op,
                    subsort: rhs_sub,
                },
            ) => {
                let op = join_op(lhs_op, rhs_op)?;
                let subsort = self.join(lhs_sub, rhs_sub)?;
                Some(self.generic(op, subsort))
            }
            (
                ResolvedSort::Function {
                    domain: lhs_domain,
                    range: lhs_range,
                },
                ResolvedSort::Function {
                    domain: rhs_domain,
                    range: rhs_range,
                },
            ) if lhs_domain.len() == rhs_domain.len() => {
                // The domain is contravariant, so the join's domain is the
                // *meet* of the two domains.
                let mut domain = Vec::with_capacity(lhs_domain.len());
                for (lhs_arg, rhs_arg) in lhs_domain.into_iter().zip(rhs_domain) {
                    domain.push(self.meet(lhs_arg, rhs_arg)?);
                }
                let range = self.join(lhs_range, rhs_range)?;
                Some(self.function(domain, range))
            }
            _ => None,
        }
    }

    /// Substitutes `with` for every occurrence of `ResolvedSort::TypeVar(var)`
    /// inside `sort`, recursively.
    pub(crate) fn substitute_var(
        &mut self,
        sort: ResolvedSortId,
        var: TypeVarId,
        with: ResolvedSortId,
    ) -> ResolvedSortId {
        match self.get(sort).clone() {
            ResolvedSort::TypeVar(id) if id == var => with,
            ResolvedSort::Container { op, subsort } => {
                let subsort = self.substitute_var(subsort, var, with);
                self.generic(op, subsort)
            }
            ResolvedSort::Function { domain, range } => {
                let domain = domain
                    .iter()
                    .map(|&sort| self.substitute_var(sort, var, with))
                    .collect();
                let range = self.substitute_var(range, var, with);
                self.function(domain, range)
            }
            // Already ground, or a distinct bound variable never introduced by
            // this substitution's own template — no occurrence of `var` can
            // occur any deeper.
            ResolvedSort::Unit | ResolvedSort::Primitive(_) | ResolvedSort::Def(_) | ResolvedSort::TypeVar(_) => sort,
        }
    }

    /// Finds the greatest common subsort of two sorts, or `None` when they are
    /// incomparable.
    ///
    /// The dual of [SortInterner::join]; used by `join`'s own `Function` case
    /// (the domain of a join is the meet of the two domains) as well as by
    /// tests.
    pub(crate) fn meet(&mut self, lhs: ResolvedSortId, rhs: ResolvedSortId) -> Option<ResolvedSortId> {
        if lhs == rhs {
            return Some(lhs);
        }

        match (self.get(lhs).clone(), self.get(rhs).clone()) {
            (ResolvedSort::Primitive(lhs), ResolvedSort::Primitive(rhs)) => {
                let lhs = number_generality(lhs)?;
                let rhs = number_generality(rhs)?;
                Some(self.primitive(number_sort_from_generality(lhs.min(rhs))))
            }
            (
                ResolvedSort::Container {
                    op: lhs_op,
                    subsort: lhs_sub,
                },
                ResolvedSort::Container {
                    op: rhs_op,
                    subsort: rhs_sub,
                },
            ) => {
                let op = meet_op(lhs_op, rhs_op)?;
                let subsort = self.meet(lhs_sub, rhs_sub)?;
                Some(self.generic(op, subsort))
            }
            (
                ResolvedSort::Function {
                    domain: lhs_domain,
                    range: lhs_range,
                },
                ResolvedSort::Function {
                    domain: rhs_domain,
                    range: rhs_range,
                },
            ) if lhs_domain.len() == rhs_domain.len() => {
                // Dual of `join`'s `Function` case: the meet's domain is the
                // join of the two domains.
                let mut domain = Vec::with_capacity(lhs_domain.len());
                for (lhs_arg, rhs_arg) in lhs_domain.into_iter().zip(rhs_domain) {
                    domain.push(self.join(lhs_arg, rhs_arg)?);
                }
                let range = self.meet(lhs_range, rhs_range)?;
                Some(self.function(domain, range))
            }
            _ => None,
        }
    }

    /// The compositional widening distance from `from` up to `to`, or `None`
    /// unless `from` is `to` or a strict subsort of it.
    ///
    ///  - `head` counts steps taken at the sorts' own head position
    /// (number-sort generality, or the FSet/Set — FBag/Bag — finiteness step);
    ///
    ///  - `interior` counts every step taken anywhere underneath it (a
    /// container's element, a function's domain or range).
    ///
    /// The split matters for ranking: a caller that only ever reaches a nonzero
    /// `interior` through [SortInterner::is_materializable]-gated pairs is
    /// guaranteed `interior == 0` in practice today.
    pub(crate) fn widening_distance(&self, from: ResolvedSortId, to: ResolvedSortId) -> Option<(u8, u8)> {
        if from == to {
            return Some((0, 0));
        }

        match (self.get(from), self.get(to)) {
            (ResolvedSort::Primitive(from), ResolvedSort::Primitive(to)) => {
                let from = number_generality(*from)?;
                let to = number_generality(*to)?;
                (to >= from).then(|| ((to - from) as u8, 0))
            }
            (
                ResolvedSort::Container {
                    op: from_op,
                    subsort: from_sub,
                },
                ResolvedSort::Container {
                    op: to_op,
                    subsort: to_sub,
                },
            ) => {
                let head = match generic_op_partial_cmp(*from_op, *to_op)? {
                    Ordering::Equal => 0,
                    Ordering::Less => 1,
                    Ordering::Greater => return None,
                };
                let interior = if from_sub == to_sub {
                    0
                } else {
                    let (sub_head, sub_interior) = self.widening_distance(*from_sub, *to_sub)?;
                    sub_head.saturating_add(sub_interior)
                };
                Some((head, interior))
            }
            (
                ResolvedSort::Function {
                    domain: from_domain,
                    range: from_range,
                },
                ResolvedSort::Function {
                    domain: to_domain,
                    range: to_range,
                },
            ) if from_domain.len() == to_domain.len() => {
                let mut interior: u8 = 0;
                for (&from_arg, &to_arg) in from_domain.iter().zip(to_domain.iter()) {
                    if from_arg != to_arg {
                        // Contravariant, like `partial_cmp`'s `Function` arm: the
                        // target's domain widens up to the source's.
                        let (head, sub_interior) = self.widening_distance(to_arg, from_arg)?;
                        interior = interior.saturating_add(head).saturating_add(sub_interior);
                    }
                }
                if *from_range != *to_range {
                    let (head, sub_interior) = self.widening_distance(*from_range, *to_range)?;
                    interior = interior.saturating_add(head).saturating_add(sub_interior);
                }
                Some((0, interior))
            }
            _ => None,
        }
    }

    /// Whether there is actually a coercion of `from` to `to` in the language.
    pub(crate) fn is_materializable(&self, from: ResolvedSortId, to: ResolvedSortId) -> bool {
        if from == to {
            return true;
        }
        match (self.get(from), self.get(to)) {
            (ResolvedSort::Primitive(from), ResolvedSort::Primitive(to)) => {
                matches!((number_generality(*from), number_generality(*to)), (Some(from), Some(to)) if to > from)
            }
            (
                ResolvedSort::Container {
                    op: from_op,
                    subsort: from_sub,
                },
                ResolvedSort::Container {
                    op: to_op,
                    subsort: to_sub,
                },
            ) => from_sub == to_sub && generic_op_partial_cmp(*from_op, *to_op) == Some(Ordering::Less),
            // A function value coerces to a wider range at the *same* domain by
            // eta-expansion: `f` of sort `D -> S` becomes `lambda d: D. coerce(f(d))`
            // of sort `D -> T` when `S` materializes to `T` -- see
            // `Lowering::coerce`'s `(Function, Function)` case, which builds exactly
            // that wrapper. The domain is not widened/narrowed here (unlike
            // `partial_cmp`'s fully general contravariant comparison used for
            // ordering): only the range side is ever actually coerced.
            (
                ResolvedSort::Function {
                    domain: from_domain,
                    range: from_range,
                },
                ResolvedSort::Function {
                    domain: to_domain,
                    range: to_range,
                },
            ) => from_domain == to_domain && self.is_materializable(*from_range, *to_range),
            _ => false,
        }
    }
}

impl Default for SortInterner {
    fn default() -> Self {
        SortInterner::new()
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering::Equal;
    use std::cmp::Ordering::Greater;
    use std::cmp::Ordering::Less;

    use merc_syntax::ComplexSort;
    use merc_syntax::SortId;

    use crate::ResolvedSortId;
    use crate::SortInterner;

    /// A set of example sorts covering the different sort forms.
    struct ExampleSorts {
        function_sort1: ResolvedSortId, // Pos # Nat -> Real
        function_sort2: ResolvedSortId, // Real # Real -> Pos
        function_sort3: ResolvedSortId, // Pos # Nat # Nat -> Real
        function_sort4: ResolvedSortId, // Pos # Nat -> List(Real)
        def_sort1: ResolvedSortId,
        def_sort2: ResolvedSortId,
        fbag_sort1: ResolvedSortId, // FBag(Real # Real -> Pos)
        fbag_sort2: ResolvedSortId, // FBag(Pos # Nat -> Real)
        bag_sort1: ResolvedSortId,  // Bag(Pos # Nat -> Real)
        bag_sort2: ResolvedSortId,  // Bag(Pos # Nat # Nat -> Real)
        fset_sort1: ResolvedSortId, // FSet(Int)
        set_sort1: ResolvedSortId,  // Set(Int)
        set_sort2: ResolvedSortId,  // Set(Pos # Nat -> Real)
    }

    impl ExampleSorts {
        fn new(c: &mut SortInterner) -> ExampleSorts {
            let function_sort1 = c.function(vec![c.pos_sort(), c.nat_sort()], c.real_sort());
            let function_sort2 = c.function(vec![c.real_sort(), c.real_sort()], c.pos_sort());
            let function_sort3 = c.function(vec![c.pos_sort(), c.nat_sort(), c.nat_sort()], c.real_sort());
            let real_list = c.generic(ComplexSort::List, c.real_sort());
            let function_sort4 = c.function(vec![c.pos_sort(), c.nat_sort()], real_list);

            ExampleSorts {
                function_sort1,
                function_sort2,
                function_sort3,
                function_sort4,
                def_sort1: c.def(SortId::new(0)),
                def_sort2: c.def(SortId::new(1)),
                fbag_sort1: c.generic(ComplexSort::FBag, function_sort2),
                fbag_sort2: c.generic(ComplexSort::FBag, function_sort1),
                bag_sort1: c.generic(ComplexSort::Bag, function_sort1),
                bag_sort2: c.generic(ComplexSort::Bag, function_sort3),
                fset_sort1: c.generic(ComplexSort::FSet, c.int_sort()),
                set_sort1: c.generic(ComplexSort::Set, c.int_sort()),
                set_sort2: c.generic(ComplexSort::Set, function_sort1),
            }
        }
    }

    #[test]
    fn test_partial_ord() {
        let mut c = SortInterner::new();
        let e = ExampleSorts::new(&mut c);

        assert_eq!(c.partial_cmp(c.bool_sort(), c.pos_sort()), None);
        assert_eq!(c.partial_cmp(c.bool_sort(), c.bool_sort()), Some(Equal));
        assert_eq!(c.partial_cmp(c.nat_sort(), c.real_sort()), Some(Less));
        assert_eq!(c.partial_cmp(c.real_sort(), c.nat_sort()), Some(Greater));
        assert_eq!(c.partial_cmp(c.nat_sort(), c.nat_sort()), Some(Equal));
        assert_eq!(c.partial_cmp(c.pos_sort(), c.real_sort()), Some(Less));
        assert_eq!(c.partial_cmp(c.unit_sort(), c.real_sort()), None);
        assert_eq!(c.partial_cmp(c.unit_sort(), c.unit_sort()), Some(Equal));
        assert_eq!(c.partial_cmp(c.unit_sort(), c.bool_sort()), None);
        assert_eq!(c.partial_cmp(c.real_sort(), c.real_sort()), Some(Equal));
        assert_eq!(c.partial_cmp(c.real_sort(), c.int_sort()), Some(Greater));

        // `function_sort2` (Real # Real -> Pos) is below `function_sort1`
        // (Pos # Nat -> Real): its domain is wider (Real >= Pos, Real >= Nat,
        // required contravariantly) and its range is narrower (Pos <= Real).
        // This is the book's own worked contravariance example (`(Nat -> Int)
        // <= (Pos -> Real)`), so unlike today this pair is now comparable; see
        // `test_function_variance_direction` for the case spelled out fully.
        assert_eq!(c.partial_cmp(e.function_sort1, e.function_sort2), Some(Greater));
        assert_eq!(c.partial_cmp(e.function_sort2, e.function_sort1), Some(Less));
        assert_eq!(c.partial_cmp(e.function_sort1, e.function_sort1), Some(Equal));
        assert_eq!(c.partial_cmp(e.function_sort1, e.function_sort3), None);
        assert_eq!(c.partial_cmp(e.function_sort3, e.function_sort1), None);
        assert_eq!(c.partial_cmp(e.function_sort4, e.function_sort1), None);
        assert_eq!(c.partial_cmp(e.function_sort4, e.function_sort4), Some(Equal));

        // Same shift as above, now nested under a container: the elements
        // (function_sort1/function_sort2) are related the same way, combined
        // with the FBag <= Bag finiteness step.
        assert_eq!(c.partial_cmp(e.bag_sort1, e.fbag_sort1), Some(Greater));
        assert_eq!(c.partial_cmp(e.fbag_sort1, e.bag_sort1), Some(Less));
        assert_eq!(c.partial_cmp(e.bag_sort1, e.fbag_sort2), Some(Greater));
        assert_eq!(c.partial_cmp(e.bag_sort1, e.bag_sort2), None);
        assert_eq!(c.partial_cmp(e.fset_sort1, e.set_sort1), Some(Less));
        assert_eq!(c.partial_cmp(e.set_sort1, e.set_sort2), None);

        assert_eq!(c.partial_cmp(e.def_sort1, e.def_sort1), Some(Equal));
        assert_eq!(c.partial_cmp(e.def_sort1, e.def_sort2), None);
        assert_eq!(c.partial_cmp(e.def_sort1, c.real_sort()), None);
        assert_eq!(c.partial_cmp(e.function_sort1, e.def_sort2), None);
    }

    #[test]
    fn test_container_covariance() {
        let mut c = SortInterner::new();
        let pos_list = c.generic(ComplexSort::List, c.pos_sort());
        let nat_list = c.generic(ComplexSort::List, c.nat_sort());
        let int_list = c.generic(ComplexSort::List, c.int_sort());
        let real_list = c.generic(ComplexSort::List, c.real_sort());
        let bool_list = c.generic(ComplexSort::List, c.bool_sort());

        assert_eq!(c.partial_cmp(pos_list, nat_list), Some(Less));
        assert_eq!(c.partial_cmp(nat_list, pos_list), Some(Greater));
        assert_eq!(c.partial_cmp(pos_list, int_list), Some(Less));
        assert_eq!(c.partial_cmp(pos_list, real_list), Some(Less));
        assert_eq!(c.partial_cmp(pos_list, bool_list), None);
    }

    #[test]
    fn test_container_covariance_with_finiteness() {
        let mut c = SortInterner::new();
        let fset_pos = c.generic(ComplexSort::FSet, c.pos_sort());
        let set_nat = c.generic(ComplexSort::Set, c.nat_sort());
        let fset_nat = c.generic(ComplexSort::FSet, c.nat_sort());
        let set_pos = c.generic(ComplexSort::Set, c.pos_sort());
        let fbag_pos = c.generic(ComplexSort::FBag, c.pos_sort());
        let bag_real = c.generic(ComplexSort::Bag, c.real_sort());

        // Both the head (FSet <= Set) and the element (Pos <= Nat) widen
        // together.
        assert_eq!(c.partial_cmp(fset_pos, set_nat), Some(Less));
        // The head says `Less` but the element says `Greater`: mixed signs
        // are incomparable, not a coincidental match.
        assert_eq!(c.partial_cmp(fset_nat, set_pos), None);
        assert_eq!(c.partial_cmp(fbag_pos, bag_real), Some(Less));
    }

    #[test]
    fn test_function_variance_direction() {
        let mut c = SortInterner::new();
        let nat_to_int = c.function(vec![c.nat_sort()], c.int_sort());
        let pos_to_real = c.function(vec![c.pos_sort()], c.real_sort());
        let pos_to_int = c.function(vec![c.pos_sort()], c.int_sort());
        let nat_to_real = c.function(vec![c.nat_sort()], c.real_sort());

        // (Nat -> Int) <= (Pos -> Real): the domain narrows from Nat to Pos
        // (contravariant — `Pos <= Nat`) while the range widens from Int to
        // Real (covariant).
        assert_eq!(c.partial_cmp(nat_to_int, pos_to_real), Some(Less));
        assert_eq!(c.partial_cmp(pos_to_real, nat_to_int), Some(Greater));
        // (Pos -> Int) and (Nat -> Real) would need `Nat <= Pos`, which is
        // false, so they stay incomparable even though each position alone
        // looks related.
        assert_eq!(c.partial_cmp(pos_to_int, nat_to_real), None);
        // Range-only and domain-only differences still compare.
        assert_eq!(c.partial_cmp(nat_to_int, nat_to_real), Some(Less));
        assert_eq!(c.partial_cmp(nat_to_int, pos_to_int), Some(Less));

        let nat_int_to_nat = c.function(vec![c.nat_sort(), c.int_sort()], c.nat_sort());
        let pos_nat_to_real = c.function(vec![c.pos_sort(), c.nat_sort()], c.real_sort());
        assert_eq!(c.partial_cmp(nat_int_to_nat, pos_nat_to_real), Some(Less));
    }

    #[test]
    fn test_function_variance_double_flip() {
        let mut c = SortInterner::new();
        let bool_sort = c.bool_sort();
        let pos_to_bool = c.function(vec![c.pos_sort()], bool_sort);
        let nat_to_bool = c.function(vec![c.nat_sort()], bool_sort);
        let outer_pos = c.function(vec![pos_to_bool], bool_sort);
        let outer_nat = c.function(vec![nat_to_bool], bool_sort);

        // A domain position nested inside a domain position is covariant
        // again: comparing `(Pos -> Bool) -> Bool` against `(Nat -> Bool) ->
        // Bool` flips once at the outer domain and back at the inner one, so
        // the outcome follows `Pos <= Nat` in the *same* direction, exactly
        // as if the two positions were not function domains at all — this is
        // the case that catches a `flip: bool` parameter threaded wrongly
        // (which would report `None` or the opposite ordering here).
        assert_eq!(c.partial_cmp(outer_pos, outer_nat), Some(Less));
        assert_eq!(c.partial_cmp(outer_nat, outer_pos), Some(Greater));
    }

    #[test]
    fn test_curried_and_flattened_functions_stay_incomparable() {
        let mut c = SortInterner::new();
        let inner = c.function(vec![c.nat_sort()], c.bool_sort());
        let curried = c.function(vec![c.nat_sort()], inner);
        let flattened = c.function(vec![c.nat_sort(), c.nat_sort()], c.bool_sort());

        assert_eq!(c.partial_cmp(curried, flattened), None);
        assert_eq!(c.partial_cmp(flattened, curried), None);
    }

    #[test]
    fn test_deeply_nested_relation_terminates() {
        let mut c = SortInterner::new();
        let build = |c: &mut SortInterner, leaf: ResolvedSortId| {
            let list = c.generic(ComplexSort::List, leaf);
            let fset = c.generic(ComplexSort::FSet, list);
            c.generic(ComplexSort::Bag, fset)
        };
        let pos = c.pos_sort();
        let real = c.real_sort();
        let pos_nested = build(&mut c, pos);
        let real_nested = build(&mut c, real);

        assert_eq!(c.partial_cmp(pos_nested, real_nested), Some(Less));
    }

    #[test]
    fn test_nominal_and_bool_stay_unrelated() {
        let mut c = SortInterner::new();
        let def = c.def(SortId::new(0));

        assert_eq!(c.partial_cmp(c.bool_sort(), c.pos_sort()), None);
        assert_eq!(c.partial_cmp(def, c.bool_sort()), None);
        assert_eq!(c.partial_cmp(c.unit_sort(), def), None);
    }

    #[test]
    fn test_join() {
        let mut c = SortInterner::new();
        let e = ExampleSorts::new(&mut c);

        assert_eq!(c.join(c.bool_sort(), c.pos_sort()), None);
        assert_eq!(c.join(c.pos_sort(), c.pos_sort()), Some(c.pos_sort()));
        assert_eq!(c.join(c.pos_sort(), c.nat_sort()), Some(c.nat_sort()));
        assert_eq!(c.join(c.real_sort(), c.nat_sort()), Some(c.real_sort()));

        // `function_sort2 <= function_sort1` (see `test_partial_ord`), so
        // their join is the wider of the two, `function_sort1` itself.
        assert_eq!(c.join(e.function_sort1, e.function_sort2), Some(e.function_sort1));
        assert_eq!(c.join(e.function_sort1, e.function_sort4), None);
        assert_eq!(c.join(e.function_sort4, e.function_sort4), Some(e.function_sort4));

        assert_eq!(c.join(e.set_sort1, e.fset_sort1), Some(e.set_sort1));
        assert_eq!(c.join(e.bag_sort1, e.fbag_sort2), Some(e.bag_sort1));
        assert_eq!(c.join(e.def_sort1, e.def_sort1), Some(e.def_sort1));
        assert_eq!(c.join(e.def_sort1, e.def_sort2), None);

        // Container-element covariance and the function-sort case combined:
        // FBag <= Bag at the head, function_sort2 <= function_sort1 at the
        // element, so the join widens both positions at once.
        assert_eq!(c.join(e.fbag_sort1, e.bag_sort1), Some(e.bag_sort1));
    }

    #[test]
    fn test_join_meet_are_lattice_operations() {
        let mut c = SortInterner::new();
        // `pos_to_int` and `nat_to_real` are themselves incomparable (domain
        // says `Greater`, range says `Less`), but a join/meet still exists:
        // the domain is combined contravariantly (meet for join, join for
        // meet) while the range is combined directly.
        let pos_to_int = c.function(vec![c.pos_sort()], c.int_sort());
        let nat_to_real = c.function(vec![c.nat_sort()], c.real_sort());
        assert_eq!(c.partial_cmp(pos_to_int, nat_to_real), None);

        let pos_to_real = c.function(vec![c.pos_sort()], c.real_sort());
        assert_eq!(c.join(pos_to_int, nat_to_real), Some(pos_to_real));
        let nat_to_int = c.function(vec![c.nat_sort()], c.int_sort());
        assert_eq!(c.meet(pos_to_int, nat_to_real), Some(nat_to_int));

        // The join is at least as wide as both operands and the meet at most
        // as wide, for every pair tested above.
        for (lhs, rhs) in [(pos_to_int, nat_to_real), (c.pos_sort(), c.nat_sort())] {
            let joined = c.join(lhs, rhs).expect("comparable in this fixture");
            let met = c.meet(lhs, rhs).expect("comparable in this fixture");
            assert_ne!(c.partial_cmp(lhs, joined), Some(Greater));
            assert_ne!(c.partial_cmp(rhs, joined), Some(Greater));
            assert_ne!(c.partial_cmp(met, lhs), Some(Greater));
            assert_ne!(c.partial_cmp(met, rhs), Some(Greater));
        }
    }

    #[test]
    fn test_meet() {
        let mut c = SortInterner::new();
        let e = ExampleSorts::new(&mut c);

        assert_eq!(c.meet(c.bool_sort(), c.pos_sort()), None);
        assert_eq!(c.meet(c.pos_sort(), c.pos_sort()), Some(c.pos_sort()));
        assert_eq!(c.meet(c.pos_sort(), c.nat_sort()), Some(c.pos_sort()));
        assert_eq!(c.meet(c.real_sort(), c.nat_sort()), Some(c.nat_sort()));

        // Dual of the `test_join` case: `function_sort2 <= function_sort1`, so
        // their meet is the narrower of the two, `function_sort2` itself.
        assert_eq!(c.meet(e.function_sort1, e.function_sort2), Some(e.function_sort2));
        assert_eq!(c.meet(e.function_sort1, e.function_sort4), None);
        assert_eq!(c.meet(e.function_sort4, e.function_sort4), Some(e.function_sort4));

        assert_eq!(c.meet(e.set_sort1, e.fset_sort1), Some(e.fset_sort1));
        assert_eq!(c.meet(e.bag_sort1, e.fbag_sort2), Some(e.fbag_sort2));
        assert_eq!(c.meet(e.def_sort1, e.def_sort1), Some(e.def_sort1));
        assert_eq!(c.meet(e.def_sort1, e.def_sort2), None);
    }

    #[test]
    fn test_widening_distance_matches_legacy_and_extends_compositionally() {
        let mut c = SortInterner::new();

        assert_eq!(c.widening_distance(c.pos_sort(), c.pos_sort()), Some((0, 0)));
        assert_eq!(c.widening_distance(c.pos_sort(), c.nat_sort()), Some((1, 0)));
        assert_eq!(c.widening_distance(c.pos_sort(), c.real_sort()), Some((3, 0)));
        assert_eq!(c.widening_distance(c.nat_sort(), c.pos_sort()), None);

        let fset_pos = c.generic(ComplexSort::FSet, c.pos_sort());
        let set_pos = c.generic(ComplexSort::Set, c.pos_sort());
        assert_eq!(c.widening_distance(fset_pos, set_pos), Some((1, 0)));

        let pos_list = c.generic(ComplexSort::List, c.pos_sort());
        let nat_list = c.generic(ComplexSort::List, c.nat_sort());
        // A container-element step carries no head step of its own; the
        // element's own head distance (1, for Pos -> Nat) becomes interior.
        assert_eq!(c.widening_distance(pos_list, nat_list), Some((0, 1)));

        // Head (FSet -> Set, 1 step) and interior (Pos -> Real, 3 steps)
        // accumulate independently.
        let set_real = c.generic(ComplexSort::Set, c.real_sort());
        assert_eq!(c.widening_distance(fset_pos, set_real), Some((1, 3)));
    }

    #[test]
    fn test_is_materializable_matches_todays_coercions_only() {
        let mut c = SortInterner::new();

        assert!(c.is_materializable(c.pos_sort(), c.pos_sort()));
        assert!(c.is_materializable(c.pos_sort(), c.real_sort()));
        assert!(!c.is_materializable(c.real_sort(), c.pos_sort()));

        let fset_pos = c.generic(ComplexSort::FSet, c.pos_sort());
        let set_pos = c.generic(ComplexSort::Set, c.pos_sort());
        assert!(c.is_materializable(fset_pos, set_pos));

        // The relation now says `List(Pos) <= List(Nat)`, but no element-wise
        // traversal exists to build the `List(Nat)` term, so this must stay
        // excluded from the materializable set.
        let pos_list = c.generic(ComplexSort::List, c.pos_sort());
        let nat_list = c.generic(ComplexSort::List, c.nat_sort());
        assert!(c.partial_cmp(pos_list, nat_list).is_some());
        assert!(!c.is_materializable(pos_list, nat_list));

        // Likewise a comparable FSet/Set pair whose elements differ.
        let set_nat = c.generic(ComplexSort::Set, c.nat_sort());
        assert!(c.partial_cmp(fset_pos, set_nat).is_some());
        assert!(!c.is_materializable(fset_pos, set_nat));

        // And function-sort variance, even though it is now comparable.
        let nat_to_int = c.function(vec![c.nat_sort()], c.int_sort());
        let pos_to_real = c.function(vec![c.pos_sort()], c.real_sort());
        assert!(c.partial_cmp(nat_to_int, pos_to_real).is_some());
        assert!(!c.is_materializable(nat_to_int, pos_to_real));
    }
}
