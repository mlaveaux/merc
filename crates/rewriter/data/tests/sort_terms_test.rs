use merc_data::BasicSort;
use merc_data::ContainerSortKind;
use merc_data::SortAlias;
use merc_data::SortArrow;
use merc_data::SortCons;
use merc_data::SortExpression;
use merc_data::is_container_sort;
use merc_data::is_function_sort;

#[test]
fn test_unknown_sort() {
    let sort = SortExpression::unknown_sort();
    assert_eq!(sort.name(), "@no_value@");
    assert_eq!(format!("{sort}"), "@no_value@");
    assert!(!is_function_sort(&sort));
    assert!(!is_container_sort(&sort));
}

#[test]
fn test_sort_arrow() {
    let nat: SortExpression = BasicSort::new("Nat").into();
    let bool_sort: SortExpression = BasicSort::new("Bool").into();
    let domain = [nat.clone(), bool_sort.clone()];
    let arrow = SortArrow::new(&domain, &bool_sort);

    assert!(is_function_sort(&arrow));
    assert!(!is_container_sort(&arrow));
    assert!(!is_function_sort(&nat));
    assert_eq!(arrow.domain().to_vec(), domain);
    assert_eq!(arrow.codomain().protect(), bool_sort);
}

#[test]
fn test_sort_cons_kind_round_trips_for_every_variant() {
    let nat: SortExpression = BasicSort::new("Nat").into();
    for (kind, keyword) in [
        (ContainerSortKind::List, "List"),
        (ContainerSortKind::Set, "Set"),
        (ContainerSortKind::Bag, "Bag"),
        (ContainerSortKind::FSet, "FSet"),
        (ContainerSortKind::FBag, "FBag"),
    ] {
        let cons = SortCons::new(kind, nat.clone());
        assert!(is_container_sort(&cons));
        assert!(!is_function_sort(&cons));
        assert_eq!(cons.kind(), kind);
        assert_eq!(cons.element_sort().protect(), nat);
        assert_eq!(format!("{cons}"), format!("{keyword}(Nat)"));
    }
}

#[test]
fn test_sort_alias() {
    let nat: SortExpression = BasicSort::new("Nat").into();
    let alias = SortAlias::new(BasicSort::new("D"), nat.clone());

    assert_eq!(alias.name().protect(), BasicSort::new("D"));
    assert_eq!(alias.reference().protect(), nat);
    assert_eq!(format!("{alias}"), "D = Nat");
}
