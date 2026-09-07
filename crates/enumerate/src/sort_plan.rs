use ahash::HashMap;
use ahash::HashMapExt;
use merc_aterm::Term;
use merc_data::ContainerSortKind;
use merc_data::DataFunctionSymbol;
use merc_data::Mcrl2DataSpecification;
use merc_data::SortArrowRef;
use merc_data::SortConsRef;
use merc_data::SortExpression;
use merc_data::SortExpressionRef;
use merc_data::is_container_sort;
use merc_data::is_function_sort;
use merc_utilities::TagIndex;

/// Distinguishes a [`SortPlanId`] from every other `TagIndex<usize, _>` in the
/// workspace at compile time.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SortPlanTag;

/// Indexes a [`SortPlan`] inside a [`SortPlans`].
pub type SortPlanId = TagIndex<usize, SortPlanTag>;

/// Why a sort cannot be enumerated at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotEnumerableReason {
    /// The sort has no constructors at all (an abstract sort, or one used only
    /// as a function/mapping argument).
    NoConstructors,
    /// The sort has constructors, but every one of them (transitively) needs an
    /// argument sort that is itself not enumerable, so no closed term of this
    /// sort can ever be built. Equivalent to the sort being empty from the
    /// enumerator's point of view.
    EmptySort,
    /// The sort is a function (`SortArrow`) sort. See
    /// `docs/enumeration-crate-plan.md` §4.7 for why this is not ported yet.
    FunctionSort,
    /// The sort is `Bag` or `FBag`. Multisets are refused outright, matching
    /// mCRL2 (§4.7); `List`/`Set`/`FSet` go through the generic constructor path.
    Bag,
    /// The sort was never seen while building the [`SortPlans`] (not among the
    /// data specification's declared sorts or any constructor's domain/target),
    /// so nothing is known about it.
    UnknownSort,
}

/// The three-way classification of a sort's enumerability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortEnumerability {
    /// Finitely many closed terms.
    Finite,
    /// Infinitely many closed terms, but every one of them is reachable by
    /// finitely many constructor applications (`Nat`, `List(D)`, mutually
    /// recursive datatypes).
    InfiniteEnumerable,
    /// Cannot be enumerated.
    NotEnumerable(NotEnumerableReason),
}

/// One constructor of a [`SortPlan`], with its argument sorts pre-resolved to
/// [`SortPlanId`]s so expanding it needs no sort lookup.
#[derive(Debug)]
pub struct ConstructorPlan {
    symbol: DataFunctionSymbol,
    arguments: Vec<SortPlanId>,
}

impl ConstructorPlan {
    /// Returns the constructor's function symbol.
    pub fn symbol(&self) -> &DataFunctionSymbol {
        &self.symbol
    }

    /// Returns the sorts of the constructor's arguments, in declaration order.
    pub fn arguments(&self) -> &[SortPlanId] {
        &self.arguments
    }

    /// Returns the constructor's arity.
    pub fn arity(&self) -> usize {
        self.arguments.len()
    }
}

/// The precomputed enumeration plan for a single sort: its constructors,
/// ordered so that a bounded search visits small terms first, and its
/// enumerability classification.
///
/// See `docs/enumeration-crate-plan.md` §6.1.
#[derive(Debug)]
pub struct SortPlan {
    sort: SortExpression,
    /// Constructors targeting this sort, ordered by ascending minimal closed
    /// term size, so the base cases (if any) come first.
    constructors: Vec<ConstructorPlan>,
    enumerability: SortEnumerability,
    /// Size (constructor application count) of the smallest closed term of
    /// this sort, or `u32::MAX` if the sort is not enumerable at all.
    min_size: u32,
}

impl SortPlan {
    /// Returns the sort this plan describes.
    pub fn sort(&self) -> &SortExpression {
        &self.sort
    }

    /// Returns the constructors targeting this sort, ordered by ascending
    /// minimal term size (§4.5).
    pub fn constructors(&self) -> &[ConstructorPlan] {
        &self.constructors
    }

    /// Returns this sort's enumerability classification.
    pub fn enumerability(&self) -> SortEnumerability {
        self.enumerability
    }

    /// Returns the size of the smallest closed term of this sort, or `None` if
    /// the sort is not enumerable.
    pub fn min_size(&self) -> Option<u32> {
        match self.min_size {
            u32::MAX => None,
            size => Some(size),
        }
    }
}

/// The precomputed enumeration plans for every sort reachable from a data
/// specification (its declared sorts, plus every constructor's domain and
/// target sorts), built once and shared by `&` across an enumeration run.
///
/// See `docs/enumeration-crate-plan.md` §6.1.
#[derive(Debug)]
pub struct SortPlans {
    plans: Vec<SortPlan>,
    index: HashMap<SortExpression, SortPlanId>,
}

impl SortPlans {
    /// Builds the enumeration plans for every sort reachable from `spec`.
    pub fn build(spec: &Mcrl2DataSpecification) -> SortPlans {
        // Phase 1: collect the sort universe in first-seen order, and group the
        // specification's constructors by target sort.
        let mut sorts: Vec<SortExpression> = Vec::new();
        let mut index: HashMap<SortExpression, SortPlanId> = HashMap::new();

        let intern = |sort: SortExpression, sorts: &mut Vec<SortExpression>, index: &mut HashMap<_, _>| -> SortPlanId {
            if let Some(id) = index.get(&sort) {
                return *id;
            }
            let id = SortPlanId::new(sorts.len());
            sorts.push(sort.clone());
            index.insert(sort, id);
            id
        };

        for basic in spec.sorts() {
            intern(SortExpression::from(basic.clone()), &mut sorts, &mut index);
        }
        for symbol in spec.constructors() {
            let (domain, target) = constructor_signature(symbol);
            intern(target, &mut sorts, &mut index);
            for argument in domain {
                intern(argument, &mut sorts, &mut index);
            }
        }

        let mut grouped: Vec<Vec<ConstructorPlan>> = sorts.iter().map(|_| Vec::new()).collect();
        for symbol in spec.constructors() {
            let (domain, target) = constructor_signature(symbol);
            let arguments = domain.into_iter().map(|sort| index[&sort]).collect();
            let target_id = index[&target];
            grouped[target_id.value()].push(ConstructorPlan {
                symbol: symbol.clone(),
                arguments,
            });
        }

        // Phase 2: classify sorts that are trivially not enumerable, before the
        // fixpoints below ever look at their constructors.
        let mut base_reason: Vec<Option<NotEnumerableReason>> = sorts
            .iter()
            .map(|sort| {
                if is_function_sort(sort) {
                    Some(NotEnumerableReason::FunctionSort)
                } else if is_container_sort(sort) {
                    let cons = SortConsRef::from(Term::copy(sort));
                    match cons.kind() {
                        ContainerSortKind::Bag | ContainerSortKind::FBag => Some(NotEnumerableReason::Bag),
                        ContainerSortKind::List | ContainerSortKind::Set | ContainerSortKind::FSet => None,
                    }
                } else {
                    None
                }
            })
            .collect();
        for (id, reason) in base_reason.iter_mut().enumerate() {
            if reason.is_none() && grouped[id].is_empty() {
                *reason = Some(NotEnumerableReason::NoConstructors);
            }
        }

        // Phase 3: least fixpoint of the minimal closed term size, the same
        // shape as `merc_typecheck::resolution::non_empty::nonempty_sorts` but
        // relaxing a `u32` instead of growing a `bool` set. `constructor_size`
        // is `None` while any argument sort's `min_size` is still unknown.
        let mut min_size: Vec<u32> = sorts.iter().map(|_| u32::MAX).collect();
        let mut constructor_size: Vec<Vec<u32>> =
            grouped.iter().map(|cs| cs.iter().map(|_| u32::MAX).collect()).collect();

        let not_enumerable_from_start: Vec<bool> = base_reason.iter().map(Option::is_some).collect();
        loop {
            let mut changed = false;
            for (sort_id, constructors) in grouped.iter().enumerate() {
                if not_enumerable_from_start[sort_id] {
                    continue;
                }

                for (constructor_id, constructor) in constructors.iter().enumerate() {
                    let mut size: Option<u32> = Some(1);
                    for &argument in &constructor.arguments {
                        match min_size[argument.value()] {
                            u32::MAX => {
                                size = None;
                                break;
                            }
                            argument_size => size = size.map(|s| s.saturating_add(argument_size)),
                        }
                    }

                    if let Some(size) = size
                        && size < constructor_size[sort_id][constructor_id]
                    {
                        constructor_size[sort_id][constructor_id] = size;
                        changed = true;
                    }
                }

                if let Some(&best) = constructor_size[sort_id].iter().min()
                    && best < min_size[sort_id]
                {
                    min_size[sort_id] = best;
                    changed = true;
                }
            }

            if !changed {
                break;
            }
        }

        // Phase 4: a sort is recursive if it is reachable from itself through
        // constructor argument sorts — the graph-cycle generalisation of "this
        // constructor's own sort appears among its arguments".
        let recursive = detect_recursive_sorts(&grouped);

        // Phase 5: least fixpoint of finiteness. A sort starts finite unless it
        // is not enumerable or recursive, and is knocked down to infinite as
        // soon as any of its constructors uses a non-finite argument sort. This
        // only ever flips `true` to `false`, so it converges monotonically.
        let mut is_finite: Vec<bool> = sorts
            .iter()
            .enumerate()
            .map(|(id, _)| !not_enumerable_from_start[id] && !recursive[id])
            .collect();
        loop {
            let mut changed = false;
            for (sort_id, constructors) in grouped.iter().enumerate() {
                if !is_finite[sort_id] {
                    continue;
                }
                if constructors
                    .iter()
                    .any(|c| c.arguments.iter().any(|&argument| !is_finite[argument.value()]))
                {
                    is_finite[sort_id] = false;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        // Phase 6: assemble the plans, ordering each sort's constructors by
        // ascending minimal term size (§4.5) so a bounded search visits small
        // terms first.
        let plans = sorts
            .into_iter()
            .enumerate()
            .map(|(sort_id, sort)| {
                let mut constructors: Vec<(u32, ConstructorPlan)> = grouped[sort_id]
                    .drain(..)
                    .enumerate()
                    .map(|(constructor_id, constructor)| (constructor_size[sort_id][constructor_id], constructor))
                    .collect();
                constructors.sort_by_key(|(size, _)| *size);

                let enumerability = if let Some(reason) = base_reason[sort_id] {
                    SortEnumerability::NotEnumerable(reason)
                } else if min_size[sort_id] == u32::MAX {
                    SortEnumerability::NotEnumerable(NotEnumerableReason::EmptySort)
                } else if is_finite[sort_id] {
                    SortEnumerability::Finite
                } else {
                    SortEnumerability::InfiniteEnumerable
                };

                SortPlan {
                    sort,
                    constructors: constructors.into_iter().map(|(_, c)| c).collect(),
                    enumerability,
                    min_size: min_size[sort_id],
                }
            })
            .collect();

        SortPlans { plans, index }
    }

    /// Looks up the plan id for `sort`, or `None` if `sort` was not reachable
    /// from the data specification this [`SortPlans`] was built from (see
    /// [`NotEnumerableReason::UnknownSort`]).
    pub fn get(&self, sort: &SortExpressionRef<'_>) -> Option<SortPlanId> {
        self.index.get(&sort.protect()).copied()
    }

    /// Returns the plan for `id`.
    pub fn plan(&self, id: SortPlanId) -> &SortPlan {
        &self.plans[id]
    }

    /// Returns the number of distinct sorts this [`SortPlans`] knows about.
    pub fn len(&self) -> usize {
        self.plans.len()
    }

    /// Returns `true` iff this [`SortPlans`] knows about no sorts at all.
    pub fn is_empty(&self) -> bool {
        self.plans.is_empty()
    }
}

/// Returns `symbol`'s argument sorts (empty for a constant constructor) and
/// its target sort.
fn constructor_signature(symbol: &DataFunctionSymbol) -> (Vec<SortExpression>, SortExpression) {
    let sort = symbol.sort();
    if is_function_sort(&sort) {
        let arrow = SortArrowRef::from(Term::copy(&sort));
        (arrow.domain().to_vec(), arrow.codomain().protect())
    } else {
        (Vec::new(), sort.protect())
    }
}

/// Returns, for each sort id, whether it is reachable from itself through
/// constructor argument sorts.
fn detect_recursive_sorts(grouped: &[Vec<ConstructorPlan>]) -> Vec<bool> {
    let n = grouped.len();
    let edges: Vec<Vec<usize>> = grouped
        .iter()
        .map(|constructors| {
            constructors
                .iter()
                .flat_map(|c| c.arguments.iter().map(|a| a.value()))
                .collect()
        })
        .collect();

    let mut recursive = vec![false; n];
    for start in 0..n {
        let mut visited = vec![false; n];
        let mut stack = edges[start].clone();
        while let Some(node) = stack.pop() {
            if node == start {
                recursive[start] = true;
                break;
            }
            if visited[node] {
                continue;
            }
            visited[node] = true;
            stack.extend(edges[node].iter().copied());
        }
    }
    recursive
}

#[cfg(test)]
mod tests {
    use merc_aterm::Term;
    use merc_syntax::UntypedDataSpecification;
    use merc_typecheck::DataSpecification;

    use super::NotEnumerableReason;
    use super::SortEnumerability;
    use super::SortPlans;

    /// Parses and type-checks the given mCRL2 data specification text.
    fn lower(source: &str) -> merc_data::Mcrl2DataSpecification {
        let untyped = UntypedDataSpecification::parse(source).unwrap();
        let data_spec = DataSpecification::from_untyped(untyped).unwrap();
        data_spec.lower_data_specification()
    }

    fn plan_for<'a>(plans: &'a SortPlans, spec: &merc_data::Mcrl2DataSpecification, name: &str) -> &'a super::SortPlan {
        let sort = spec
            .sorts()
            .iter()
            .find(|s| s.name() == name)
            .map(|s| merc_data::SortExpression::from(s.clone()))
            .or_else(|| {
                // Built-in sorts (Bool, Nat, ...) are not in `spec.sorts()`, but
                // are reachable through some constructor's target sort.
                spec.constructors().iter().find_map(|c| {
                    let sort = c.sort();
                    let sort = if merc_data::is_function_sort(&sort) {
                        merc_data::SortArrowRef::from(Term::copy(&sort)).codomain().protect()
                    } else {
                        sort.protect()
                    };
                    (sort.name() == name).then_some(sort)
                })
            })
            .unwrap_or_else(|| panic!("sort {name} not found in specification"));

        let id = plans
            .get(&sort.copy())
            .unwrap_or_else(|| panic!("sort {name} not in plans"));
        plans.plan(id)
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_bool_is_finite() {
        let spec = lower("sort D;\ncons c: D;");
        let plans = SortPlans::build(&spec);

        let bool_plan = plan_for(&plans, &spec, "Bool");
        assert_eq!(bool_plan.enumerability(), SortEnumerability::Finite);
        assert_eq!(bool_plan.constructors().len(), 2);
        assert_eq!(bool_plan.min_size(), Some(1));
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_nat_is_infinite_enumerable() {
        let spec = lower("sort D;\ncons c: D;");
        let plans = SortPlans::build(&spec);

        let nat_plan = plan_for(&plans, &spec, "Nat");
        assert_eq!(nat_plan.enumerability(), SortEnumerability::InfiniteEnumerable);
        // @c0 (size 1) before @succ_nat(...) (size 2): base case first.
        assert_eq!(nat_plan.constructors()[0].symbol().name().value(), "@c0");
        assert_eq!(nat_plan.min_size(), Some(1));
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_empty_sort_is_not_enumerable() {
        // `D`'s only constructor recurses into itself, so it has no closed
        // terms at all — mirrors non_empty.rs's own empty-sort test, but that
        // check happens in `merc_typecheck` and rejects such a spec before it
        // ever becomes an `Mcrl2DataSpecification` (confirmed by the sibling
        // `test_typecheck_rejects_empty_sort` below), so building the
        // specification directly, bypassing typecheck, is the only way to
        // exercise `SortPlans`'s *own* classification of this case — which
        // matters because nothing stops a caller from assembling one by hand.
        let d: merc_data::SortExpression = merc_data::BasicSort::new("D").into();
        let arrow: merc_data::SortExpression = merc_data::SortArrow::new(std::slice::from_ref(&d), d.clone()).into();
        let f = merc_data::DataFunctionSymbol::with_sort("f", arrow.copy());
        let spec = merc_data::Mcrl2DataSpecification::new(
            vec![merc_data::BasicSort::new("D")],
            Vec::new(),
            vec![f],
            Vec::new(),
            Vec::new(),
        );

        let plans = SortPlans::build(&spec);
        let id = plans.get(&d.copy()).unwrap();
        assert_eq!(
            plans.plan(id).enumerability(),
            SortEnumerability::NotEnumerable(NotEnumerableReason::EmptySort)
        );
        assert_eq!(plans.plan(id).min_size(), None);
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_typecheck_rejects_empty_sort() {
        // Documents the invariant the previous test's comment relies on.
        let untyped = UntypedDataSpecification::parse("sort D;\ncons f: D -> D;").unwrap();
        assert!(DataSpecification::from_untyped(untyped).is_err());
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_abstract_sort_has_no_constructors() {
        let spec = lower("sort D;\n     E;\ncons c: D -> E;");
        let plans = SortPlans::build(&spec);

        let plan = plan_for(&plans, &spec, "D");
        assert_eq!(
            plan.enumerability(),
            SortEnumerability::NotEnumerable(NotEnumerableReason::NoConstructors)
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_function_sort_is_not_enumerable() {
        // `Bool -> Bool` would be reachable as `f`'s own sort if `f` were a
        // mapping, but walking `spec.mappings()` isn't part of `SortPlans`
        // (§Division of responsibility: constructors are the only source of
        // sorts); build a spec whose *constructor* has a function-sorted
        // argument instead.
        let spec = lower(
            "sort D;
             cons c: (Bool -> Bool) -> D;",
        );
        let plans = SortPlans::build(&spec);
        let sort = spec.constructors()[0].sort();
        let arrow = merc_data::SortArrowRef::from(Term::copy(&sort));
        let function_sort = arrow.domain().to_vec().into_iter().next().unwrap();

        let id = plans.get(&function_sort.copy()).unwrap();
        assert_eq!(
            plans.plan(id).enumerability(),
            SortEnumerability::NotEnumerable(NotEnumerableReason::FunctionSort)
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_mutually_recursive_sorts_are_infinite_enumerable() {
        let spec = lower(
            "sort A;
                  B;
             cons a0: A;
                  a1: B -> A;
                  b1: A -> B;",
        );
        let plans = SortPlans::build(&spec);

        let a_plan = plan_for(&plans, &spec, "A");
        let b_plan = plan_for(&plans, &spec, "B");
        assert_eq!(a_plan.enumerability(), SortEnumerability::InfiniteEnumerable);
        assert_eq!(b_plan.enumerability(), SortEnumerability::InfiniteEnumerable);
        assert_eq!(a_plan.min_size(), Some(1)); // a0
        assert_eq!(b_plan.min_size(), Some(2)); // b1(a0)
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_bag_is_not_enumerable() {
        let spec = lower("sort D = Bag(Bool);");
        let plans = SortPlans::build(&spec);

        let sort = spec.aliases()[0].reference().protect();
        let id = plans.get(&sort.copy()).unwrap();
        assert_eq!(
            plans.plan(id).enumerability(),
            SortEnumerability::NotEnumerable(NotEnumerableReason::Bag)
        );
    }
}
