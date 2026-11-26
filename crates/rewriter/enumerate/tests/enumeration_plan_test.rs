use merc_aterm::Term;
use merc_data::BasicSort;
use merc_data::DataFunctionSymbol;
use merc_data::Mcrl2DataSpecification;
use merc_data::SortArrow;
use merc_data::SortArrowRef;
use merc_data::SortExpression;
use merc_data::is_function_sort;
use merc_enumerate::EnumerationPlan;
use merc_enumerate::EnumerationPlans;
use merc_enumerate::NotEnumerableReason;
use merc_enumerate::SortEnumerability;
use merc_lowering::lower_data_specification;
use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;

/// Parses and type-checks the given mCRL2 data specification text.
fn lower(source: &str) -> Mcrl2DataSpecification {
    let untyped = UntypedDataSpecification::parse(source).unwrap();
    let data_spec = DataSpecification::from_untyped(untyped).unwrap();
    lower_data_specification(&data_spec)
}

/// The plan of the sort called `name`, which is a declared sort or the target sort of a
/// constructor (the built-in sorts are only reachable that way).
fn plan_for<'a>(plans: &'a EnumerationPlans, spec: &Mcrl2DataSpecification, name: &str) -> &'a EnumerationPlan {
    let sort = spec
        .sorts()
        .iter()
        .find(|s| s.name() == name)
        .map(|s| SortExpression::from(s.clone()))
        .or_else(|| {
            spec.constructors().iter().find_map(|c| {
                let sort = c.sort();
                let sort = if is_function_sort(&sort) {
                    SortArrowRef::from(Term::copy(&sort)).codomain().protect()
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
    let plan = plans.plan(id);
    assert_eq!(*plan.sort(), sort);
    plan
}

/// The constructor names of `plan`, in plan order.
fn constructor_names(plan: &EnumerationPlan) -> Vec<String> {
    plan.constructors()
        .iter()
        .map(|constructor| constructor.symbol().name().value().to_string())
        .collect()
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_bool_is_finite() {
    let spec = lower("sort D;\ncons c: D;");
    let plans = EnumerationPlans::build(&spec);

    let bool_plan = plan_for(&plans, &spec, "Bool");
    assert_eq!(bool_plan.enumerability(), SortEnumerability::Finite);
    let mut names = constructor_names(bool_plan);
    names.sort();
    assert_eq!(names, ["false", "true"]);
    assert!(bool_plan.constructors().iter().all(|c| c.arity() == 0));
    assert_eq!(bool_plan.min_size(), Some(1));

    let d_plan = plan_for(&plans, &spec, "D");
    assert_eq!(d_plan.enumerability(), SortEnumerability::Finite);
    assert_eq!(constructor_names(d_plan), ["c"]);
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_nat_is_infinite_enumerable() {
    let spec = lower("sort D;\ncons c: D;");
    let plans = EnumerationPlans::build(&spec);

    let nat_plan = plan_for(&plans, &spec, "Nat");
    assert_eq!(nat_plan.enumerability(), SortEnumerability::InfiniteEnumerable);
    // @c0 (size 1) before @cNat(@c1) (size 2): base case first.
    assert_eq!(constructor_names(nat_plan), ["@c0", "@cNat"]);
    assert_eq!(nat_plan.constructors()[0].arity(), 0);
    assert_eq!(nat_plan.constructors()[1].arity(), 1);
    assert_eq!(nat_plan.min_size(), Some(1));
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_constructors_are_ordered_by_minimal_size_with_resolved_arguments() {
    let spec = lower(
        "sort T;
         cons node: T # T -> T;
              wrap: T -> T;
              leaf: T;",
    );
    let plans = EnumerationPlans::build(&spec);

    let plan = plan_for(&plans, &spec, "T");
    assert_eq!(plan.enumerability(), SortEnumerability::InfiniteEnumerable);
    // leaf has size 1, wrap(leaf) size 2 and node(leaf, leaf) size 3.
    assert_eq!(constructor_names(plan), ["leaf", "wrap", "node"]);
    assert_eq!(plan.min_size(), Some(1));

    let t = plans.get(&plan.sort().copy()).unwrap();
    assert_eq!(plan.constructors()[1].arguments(), [t]);
    assert_eq!(plan.constructors()[2].arguments(), [t, t]);
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_empty_sort_is_not_enumerable() {
    // `D`'s only constructor recurses into itself, so it has no closed terms at all. The type
    // checker rejects such a specification, so it is built by hand.
    let untyped = UntypedDataSpecification::parse("sort D;\ncons f: D -> D;").unwrap();
    assert!(DataSpecification::from_untyped(untyped).is_err());

    let d: SortExpression = BasicSort::new("D").into();
    let arrow: SortExpression = SortArrow::new(std::slice::from_ref(&d), &d).into();
    let f = DataFunctionSymbol::with_sort("f", arrow.copy());
    let spec = Mcrl2DataSpecification::new(vec![BasicSort::new("D")], Vec::new(), vec![f], Vec::new(), Vec::new());

    let plans = EnumerationPlans::build(&spec);
    let id = plans.get(&d.copy()).unwrap();
    assert_eq!(
        plans.plan(id).enumerability(),
        SortEnumerability::NotEnumerable(NotEnumerableReason::EmptySort)
    );
    assert_eq!(plans.plan(id).min_size(), None);
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_abstract_sort_has_no_constructors() {
    let spec = lower("sort D;\n     E;\ncons c: D -> E;");
    let plans = EnumerationPlans::build(&spec);

    let plan = plan_for(&plans, &spec, "D");
    assert_eq!(
        plan.enumerability(),
        SortEnumerability::NotEnumerable(NotEnumerableReason::NoConstructors)
    );
    assert!(plan.constructors().is_empty());
    assert_eq!(plan.min_size(), None);

    // `E` only has a constructor that needs a `D`, so it is empty too.
    let plan = plan_for(&plans, &spec, "E");
    assert_eq!(
        plan.enumerability(),
        SortEnumerability::NotEnumerable(NotEnumerableReason::EmptySort)
    );
    assert_eq!(plan.min_size(), None);
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_unknown_sort_has_no_plan() {
    let spec = lower("sort D;\ncons c: D;");
    let plans = EnumerationPlans::build(&spec);

    let unknown: SortExpression = BasicSort::new("NotDeclared").into();
    assert!(plans.get(&unknown.copy()).is_none());
    assert!(!plans.is_empty());
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_function_sort_is_not_enumerable() {
    // Only constructors contribute sorts, so the function sort is the argument of a constructor.
    let spec = lower(
        "sort D;
         cons c: (Bool -> Bool) -> D;",
    );
    let plans = EnumerationPlans::build(&spec);
    let sort = spec.constructors()[0].sort();
    let arrow = SortArrowRef::from(Term::copy(&sort));
    let function_sort = arrow.domain().to_vec().into_iter().next().unwrap();

    let id = plans.get(&function_sort.copy()).unwrap();
    assert_eq!(
        plans.plan(id).enumerability(),
        SortEnumerability::NotEnumerable(NotEnumerableReason::FunctionSort)
    );
    assert_eq!(
        plan_for(&plans, &spec, "D").enumerability(),
        SortEnumerability::NotEnumerable(NotEnumerableReason::EmptySort)
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
    let plans = EnumerationPlans::build(&spec);

    let a_plan = plan_for(&plans, &spec, "A");
    let b_plan = plan_for(&plans, &spec, "B");
    assert_eq!(a_plan.enumerability(), SortEnumerability::InfiniteEnumerable);
    assert_eq!(b_plan.enumerability(), SortEnumerability::InfiniteEnumerable);
    assert_eq!(constructor_names(a_plan), ["a0", "a1"]);
    assert_eq!(a_plan.min_size(), Some(1)); // a0
    assert_eq!(b_plan.min_size(), Some(2)); // b1(a0)
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_bag_is_empty_via_function_argument() {
    // `@bag: (S -> Nat) # FBag(S) -> Bag(S)` needs a function-sorted argument.
    let spec = lower("sort D = Bag(Bool);");
    let plans = EnumerationPlans::build(&spec);

    let sort = spec.aliases()[0].reference().protect();
    let id = plans.get(&sort.copy()).unwrap();
    assert_eq!(
        plans.plan(id).enumerability(),
        SortEnumerability::NotEnumerable(NotEnumerableReason::EmptySort)
    );
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_fbag_is_infinite_enumerable() {
    let spec = lower("sort D = FBag(Bool);");
    let plans = EnumerationPlans::build(&spec);

    let sort = spec.aliases()[0].reference().protect();
    let id = plans.get(&sort.copy()).unwrap();
    assert_eq!(plans.plan(id).enumerability(), SortEnumerability::InfiniteEnumerable);
    assert_eq!(plans.plan(id).min_size(), Some(1));
}
