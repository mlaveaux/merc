//! Tests for `compositional::linearise_compositional` (Milestone 2, per
//! `docs/linearisation-implementation-plan.md`).

use merc_linearisation::CompositionalSpecification;
use merc_linearisation::LinearisationError;
use merc_syntax::UntypedProcessSpecification;
use merc_typecheck::ProcessSpecification;

/// Type checks and compositionally linearises `text`, asserting both succeed.
#[track_caller]
fn linearise_ok(text: &str) -> CompositionalSpecification {
    let untyped = UntypedProcessSpecification::parse(text).expect("the specification should parse");
    let spec = ProcessSpecification::from_untyped(untyped).expect("the specification should type check");
    match merc_linearisation::linearise_compositional(&spec) {
        Ok(result) => result,
        Err(error) => panic!("expected compositional linearisation to succeed:\n{text}\nerror: {error}"),
    }
}

#[track_caller]
fn linearise_err(text: &str) -> LinearisationError {
    let untyped = UntypedProcessSpecification::parse(text).expect("the specification should parse");
    let spec = ProcessSpecification::from_untyped(untyped).expect("the specification should type check");
    match merc_linearisation::linearise_compositional(&spec) {
        Err(error) => error,
        Ok(_) => panic!("expected compositional linearisation to be rejected:\n{text}"),
    }
}

#[test]
fn test_plain_parallel_composition_splits_into_one_component_per_operand() {
    let result = linearise_ok("act a, b; proc P1 = a; proc P2 = b; init P1 || P2;");
    assert_eq!(result.components.len(), 2);
    assert_eq!(result.components[0].name, "P1");
    assert_eq!(result.components[1].name, "P2");
    assert!(result.hide.is_none());
    assert!(result.allow.is_none());
    assert!(result.comm.is_none());
}

#[test]
fn test_three_way_parallel_composition_flattens_regardless_of_associativity() {
    let result = linearise_ok("act a, b, c; proc P1 = a; proc P2 = b; proc P3 = c; init P1 || P2 || P3;");
    assert_eq!(result.components.len(), 3);
}

#[test]
fn test_allow_wrapper_is_captured_as_text_and_not_applied() {
    let result = linearise_ok("act a, b; proc P1 = a; proc P2 = b; init allow({a, b}, P1 || P2);");
    assert_eq!(result.components.len(), 2);
    assert_eq!(result.allow.as_deref(), Some("{a, b}"));
}

#[test]
fn test_comm_wrapper_is_captured_as_text() {
    let result = linearise_ok("act a, b, c; proc P1 = a; proc P2 = b; init comm({a|b -> c}, P1 || P2);");
    assert_eq!(result.comm.as_deref(), Some("{a|b -> c}"));
}

#[test]
fn test_hide_allow_comm_nest_in_the_fixed_order() {
    let result = linearise_ok(
        "act a, b, c; proc P1 = a; proc P2 = b; \
         init hide({c}, allow({a, b, c}, comm({a|b -> c}, P1 || P2)));",
    );
    assert_eq!(result.hide.as_deref(), Some("{c}"));
    assert_eq!(result.allow.as_deref(), Some("{a, b, c}"));
    assert_eq!(result.comm.as_deref(), Some("{a|b -> c}"));
    assert_eq!(result.components.len(), 2);
}

#[test]
fn test_duplicate_component_names_get_disambiguated() {
    let result = linearise_ok("act a; proc P = a; init P || P;");
    assert_eq!(result.components.len(), 2);
    assert_eq!(result.components[0].name, "P_1");
    assert_eq!(result.components[1].name, "P_2");
}

#[test]
fn test_operand_that_is_not_a_process_call_is_rejected() {
    let error = linearise_err("act a; proc P = a; init a || P;");
    assert!(
        matches!(error, LinearisationError::CompositionalOperandNotAProcessCall { .. }),
        "got {error:?}"
    );
}

#[test]
fn test_component_with_internal_parallel_composition_is_rejected() {
    // `P2`'s own body uses `||`, so it isn't parallel-free: the shared
    // per-process translator rejects it exactly as it would outside a
    // compositional context, with no separate check needed here.
    let error = linearise_err("act a; proc Inner = a; proc P1 = a; proc P2 = Inner || Inner; init P1 || P2;");
    assert!(
        matches!(error, LinearisationError::UnsupportedConstruct { .. }),
        "got {error:?}"
    );
}

#[test]
fn test_single_process_with_no_parallel_composition_is_one_component() {
    // Degenerate but legitimate: no `||` at the top means "one component".
    let result = linearise_ok("act a; proc P = a; init P;");
    assert_eq!(result.components.len(), 1);
}

#[test]
fn test_manifest_names_each_component_file_and_omits_absent_operators() {
    let result = linearise_ok("act a, b; proc P1 = a; proc P2 = b; init P1 || P2;");
    let manifest = result.manifest();
    assert_eq!(manifest.components.len(), 2);
    assert_eq!(manifest.components[0].name, "P1");
    assert_eq!(manifest.components[0].lps_file, "P1.mcrl2");

    let json = serde_json::to_string(&manifest).unwrap();
    assert!(json.contains("\"lps_file\":\"P1.mcrl2\""));
    assert!(
        !json.contains("\"hide\""),
        "absent hide should be omitted, not null: {json}"
    );
}

#[test]
fn test_manifest_includes_present_operators_in_json() {
    let result = linearise_ok("act a, b; proc P1 = a; proc P2 = b; init allow({a, b}, P1 || P2);");
    let json = serde_json::to_string(&result.manifest()).unwrap();
    assert!(json.contains("\"allow\":\"{a, b}\""), "{json}");
}
