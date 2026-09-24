use merc_syntax::UntypedStateFrmSpec;
use merc_typecheck::FormulaType;
use merc_typecheck::ModalSpecification;
use merc_typecheck::ResolvedName;
use merc_typecheck::TypingInfo;

/// Type checks `text` as a state formula, returning its full [`TypingInfo`] (every checked
/// expression, merged).
#[track_caller]
fn typing_for(text: &str) -> TypingInfo {
    let spec = UntypedStateFrmSpec::parse(text).expect("the specification should parse");
    let mut spec =
        ModalSpecification::from_untyped(spec, FormulaType::Real).expect("the specification should type check");
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

/// A quantifier's bound variable is checked (and its typing recorded) inside its own body.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_quantifier_bound_variable_hover_reports_declared_sort() {
    assert_eq!(hover("exists d: Nat . val(d)", "d)"), "Nat");
}

/// A quantifier binder's own declaration occurrence (`d` in `exists d: Nat . ...`, not a later use
/// of it in the body) is itself hoverable, reporting its declared sort and a `Variable`
/// resolution whose declaration span points back at itself — the same way a use inside the body
/// resolves.
///
/// Regression test: `collect_binder_sorts` used to only use this span to extend the checking
/// scope (for resolving *later* uses of `d`), never recording a `TypedNode` for the declaration
/// occurrence itself, so hovering `d` between `exists` and `:` found nothing.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_quantifier_binder_declaration_itself_is_hoverable() {
    let text = "exists d: Nat . val(d) && val(d)";
    assert_eq!(hover(text, "d: Nat"), "Nat");
    let name = resolved_name_at(text, "d: Nat");
    let ResolvedName::Variable { name, declaration } = &name else {
        panic!("expected a Variable resolution, got {name:?}");
    };
    assert_eq!(name, "d");
    let declaration = declaration
        .clone()
        .expect("a quantifier-bound variable has a real declaration span");
    assert_eq!(&text[declaration.start..declaration.end], "d");
}

/// A fixpoint (`mu`/`nu`) parameter's own declaration (`n` in `nu X(n: Nat = 0)`, not a use of it
/// in the body) is itself hoverable, mirroring
/// [`test_quantifier_binder_declaration_itself_is_hoverable`].
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_fixed_point_parameter_declaration_itself_is_hoverable() {
    let text = "act a: Nat;\nform nu X(n: Nat = 0) . [a(n)]X(n);";
    assert_eq!(hover(text, "n: Nat"), "Nat");
    let name = resolved_name_at(text, "n: Nat");
    let ResolvedName::Variable { name, .. } = &name else {
        panic!("expected a Variable resolution, got {name:?}");
    };
    assert_eq!(name, "n");
}

/// An action-formula quantifier (`exists`/`forall` inside a `<...>`/`[...]` modality)'s own
/// declaration occurrence is hoverable too, mirroring the state-formula case above.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_action_formula_quantifier_binder_declaration_itself_is_hoverable() {
    let text = "act a: Nat;\nform [exists n: Nat . a(n)]false;";
    assert_eq!(hover(text, "n: Nat"), "Nat");
}
