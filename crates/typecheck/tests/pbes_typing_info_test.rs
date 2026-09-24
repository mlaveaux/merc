use merc_syntax::UntypedPbes;
use merc_typecheck::PbesSpecification;
use merc_typecheck::ResolvedName;
use merc_typecheck::TypingInfo;

/// Type checks `text` as a PBES, returning its full [`TypingInfo`] (every checked expression,
/// merged).
#[track_caller]
fn typing_for(text: &str) -> TypingInfo {
    let spec = UntypedPbes::parse(text).expect("the specification should parse");
    let mut spec = PbesSpecification::from_untyped(spec).expect("the specification should type check");
    spec.typing_info()
}

/// Finds `needle`'s byte offset in `text` and returns the sort (as displayed text) of the most
/// specific typed node there.
#[track_caller]
fn hover(text: &str, needle: &str) -> String {
    let offset = text
        .find(needle)
        .unwrap_or_else(|| panic!("'{needle}' not found in '{text}'"));
    typing_for(text)
        .at_offset(offset)
        .unwrap_or_else(|| panic!("no typed node at offset {offset} in '{text}'"))
        .sort
        .as_ref()
        .unwrap_or_else(|| panic!("node at offset {offset} in '{text}' has no sort"))
        .to_string()
}

/// As [`hover`], but returns the resolved name instead of the sort.
#[track_caller]
fn resolved_name_at(text: &str, needle: &str) -> ResolvedName {
    let offset = text
        .find(needle)
        .unwrap_or_else(|| panic!("'{needle}' not found in '{text}'"));
    typing_for(text)
        .at_offset(offset)
        .unwrap_or_else(|| panic!("no typed node at offset {offset} in '{text}'"))
        .name
        .clone()
        .unwrap_or_else(|| panic!("node at offset {offset} in '{text}' has no resolved name"))
}

/// A `PropVarInst` argument yields a non-empty, resolvable `TypingInfo`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_prop_var_inst_argument_hover_reports_declared_sort() {
    assert_eq!(hover("pbes mu X(n: Nat) = val(n == n); init X(1);", "1);"), "Pos");
}

/// The declaration span carried by a `Variable` resolution is the actual goto-definition
/// target: stable and shared between the equation's own parameter and its (self-recursive)
/// occurrence.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_prop_var_inst_self_recursive_argument_goto_def_declaration_matches_parameter() {
    let text = "pbes nu X(n: Nat) = val(n == 0) || X(n); init X(0);";
    let ResolvedName::Variable {
        name: first_name,
        declaration: first,
    } = resolved_name_at(text, "n ==")
    else {
        panic!("expected a Variable resolution");
    };
    let ResolvedName::Variable {
        name: second_name,
        declaration: second,
    } = resolved_name_at(text, "n);")
    else {
        panic!("expected a Variable resolution");
    };
    assert_eq!(first_name, "n");
    assert_eq!(second_name, "n");
    let first = first.expect("an equation parameter has a real declaration");
    let second = second.expect("an equation parameter has a real declaration");
    assert_eq!(first, second, "both occurrences refer to the same equation parameter");
}

/// A quantifier-bound variable's declaration is likewise stable and shared across its own
/// occurrences, distinct from the equation's own parameter list.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_quantifier_bound_variable_goto_def_declaration_is_shared_across_occurrences() {
    let text = "pbes mu X = forall n: Nat . val(n == n); init X;";
    let ResolvedName::Variable { declaration: first, .. } = resolved_name_at(text, "n ==") else {
        panic!("expected a Variable resolution");
    };
    let ResolvedName::Variable {
        declaration: second, ..
    } = resolved_name_at(text, "n)")
    else {
        panic!("expected a Variable resolution");
    };
    let first = first.expect("a quantifier-bound variable has a real declaration");
    let second = second.expect("a quantifier-bound variable has a real declaration");
    assert_eq!(first, second, "both occurrences refer to the same quantifier binder");
}

/// A quantifier binder's own declaration occurrence (`n` in `forall n: Nat . ...`, not a later use
/// of it in the body) is itself hoverable.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_quantifier_binder_declaration_itself_is_hoverable() {
    let text = "pbes mu X = forall n: Nat . val(n == n); init X;";
    assert_eq!(hover(text, "n: Nat"), "Nat");
    let name = resolved_name_at(text, "n: Nat");
    let ResolvedName::Variable { name, declaration } = &name else {
        panic!("expected a Variable resolution, got {name:?}");
    };
    assert_eq!(name, "n");
    let declaration = declaration
        .clone()
        .expect("a quantifier-bound variable has a real declaration span");
    assert_eq!(&text[declaration.start..declaration.end], "n");
}

/// An equation's own parameter declaration (`n` in `X(n: Nat)`, not a use of it in the formula)
/// is itself hoverable.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_equation_parameter_declaration_itself_is_hoverable() {
    let text = "pbes mu X(n: Nat) = val(n == n); init X(1);";
    assert_eq!(hover(text, "n: Nat"), "Nat");
    let name = resolved_name_at(text, "n: Nat");
    let ResolvedName::Variable { name, .. } = &name else {
        panic!("expected a Variable resolution, got {name:?}");
    };
    assert_eq!(name, "n");
}

/// A `glob` variable's own declaration (not a use of it) is itself hoverable, mirroring
/// [`test_quantifier_binder_declaration_itself_is_hoverable`].
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_global_variable_declaration_itself_is_hoverable() {
    let text = "glob x: Nat; pbes mu X = val(x == x); init X;";
    assert_eq!(hover(text, "x:"), "Nat");
}

/// A quantifier's bound variable is checked (and its typing recorded) inside its own body.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_quantifier_bound_variable_hover_reports_declared_sort() {
    assert_eq!(
        hover("pbes mu X = forall n: Nat . val(n == n); init X;", "n == n"),
        "Nat"
    );
}

/// Every branch of a conjunction/disjunction chain contributes its own `val(...)` typing, not
/// just the last one.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_every_branch_of_a_conjunction_chain_contributes_typing() {
    let text = "pbes mu X(a: Nat, b: Nat, c: Nat) = val(a == a) && val(b == b) && val(c == c); init X(0, 0, 0);";
    assert_eq!(hover(text, "a == a"), "Nat");
    assert_eq!(hover(text, "b == b"), "Nat");
    assert_eq!(hover(text, "c == c"), "Nat");
}

/// `typing_info` merges in the data specification's own `eqn` typing, not just the PBES
/// expressions': a data-`eqn` right-hand side has no PBES formula wrapping it, so only the merge
/// itself can surface it here.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_typing_info_merges_the_data_specification_eqn_typing() {
    let text = "map f: Nat; eqn f = 1; pbes mu X = val(f == f); init X;";
    assert_eq!(hover(text, "1;"), "Pos");
}

/// `init X(1);`'s own name resolves to a `PropositionalVariable`, pointing at its `pbes`
/// declaration.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_prop_var_inst_name_goto_def_resolves_to_its_declaration() {
    let text = "pbes mu X(n: Nat) = val(n == n); init X(1);";
    let name = resolved_name_at(text, "X(1)");
    let ResolvedName::PropositionalVariable { name, declaration } = &name else {
        panic!("expected a PropositionalVariable resolution, got {name:?}");
    };
    assert_eq!(name, "X");
    let declaration = declaration.clone().expect("a plain `pbes` equation has a real span");
    assert_eq!(&text[declaration.start..declaration.end], "X");
}

/// An equation's own parameter sort resolves to the `sort` block declaring it.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_equation_parameter_sort_goto_def_resolves_to_its_declaration() {
    let text = "sort D; cons c: D; pbes mu X(n: D) = val(true); init X(c);";
    // "D)" is unique to the equation parameter's own sort.
    let name = resolved_name_at(text, "D)");
    let ResolvedName::Sort { name, declaration } = &name else {
        panic!("expected a Sort resolution, got {name:?}");
    };
    assert_eq!(name, "D");
    let declaration = declaration.clone().expect("a plain `sort` declaration has a real span");
    assert_eq!(&text[declaration.start..declaration.end], "D");
}

/// A `glob` variable's own sort resolves the same way.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_global_variable_sort_goto_def_resolves_to_its_declaration() {
    let text = "sort D; glob x: D; pbes mu X = val(true); init X;";
    // "D; pbes" is unique to the `glob`'s own sort — `sort D;` also contains "D;", but is
    // followed by " glob", not " pbes".
    let name = resolved_name_at(text, "D; pbes");
    let ResolvedName::Sort { name, .. } = &name else {
        panic!("expected a Sort resolution, got {name:?}");
    };
    assert_eq!(name, "D");
}

/// A `Quantifier` binder's own declared sort resolves too — distinct from the bound variable
/// itself, already covered by [`test_quantifier_bound_variable_goto_def_declaration_points_at_binder`].
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_quantifier_bound_variable_sort_goto_def_resolves_to_its_declaration() {
    let text = "sort D; pbes mu X = forall n: D . val(true); init X;";
    // Unique to the binder's own sort: no other "D ." occurs in `text`.
    let name = resolved_name_at(text, "D .");
    let ResolvedName::Sort { name, declaration } = &name else {
        panic!("expected a Sort resolution, got {name:?}");
    };
    assert_eq!(name, "D");
    let declaration = declaration.clone().expect("a plain `sort` declaration has a real span");
    assert_eq!(&text[declaration.start..declaration.end], "D");
}

/// A self-recursive `PropVarInst` inside the equation's own formula resolves the same way as a use
/// from `init`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_self_recursive_prop_var_inst_name_goto_def_resolves_to_its_declaration() {
    let text = "pbes nu X(n: Nat) = val(n == 0) || X(n); init X(0);";
    let name = resolved_name_at(text, "X(n)");
    let ResolvedName::PropositionalVariable { name, declaration } = &name else {
        panic!("expected a PropositionalVariable resolution, got {name:?}");
    };
    assert_eq!(name, "X");
    let declaration = declaration.clone().expect("a plain `pbes` equation has a real span");
    assert_eq!(&text[declaration.start..declaration.end], "X");
}

/// A list literal's element sort widens to the joined sort of all its elements (here `Nat`, from
/// the `10`/`m` mix), not just the first element's own sort (`Pos`, `10`'s literal sort).
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_list_literal_element_sort_widens_to_the_joined_sort_of_its_elements() {
    let text = "pbes nu X0(m: Nat) = forall i: Nat . val(!(i < 2)) || X0([10, m] . i); init X0(0);";
    // The `.` (list-at) operator's own sort names its operand sorts directly.
    assert_eq!(hover(text, ". i)"), "(List(Nat) # Nat -> Nat)");
}
