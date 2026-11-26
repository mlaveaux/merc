//! Tests for loading REC specifications, see [merc_rec_tests::load_rec_from_strings] and
//! [merc_rec_tests::load_rec_from_file].

use std::path::Path;

use merc_aterm::ATerm;
use merc_rec_tests::load_rec_from_file;
use merc_rec_tests::load_rec_from_strings;

/// A specification with the given `RULES` and `EVAL` lines.
fn spec_text(rules: &str, eval: &str) -> String {
    format!(
        "REC-SPEC Test
SORTS
  Nat
CONS
  d0 : -> Nat
  s : Nat -> Nat
OPNS
  f : Nat Nat -> Nat
  g : Nat -> Nat
VARS
  X Y Val Max : Nat
RULES
  {rules}
EVAL
  {eval}
END-SPEC
"
    )
}

fn aterm(text: &str) -> ATerm {
    ATerm::from_string(text).unwrap()
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_load_rec_sections() {
    let text = spec_text(
        "f(X, d0) = g(X) if X = d0 and-if s(Y) <> Y\n  g(X) -> X",
        "f(s(d0), d0)\n  g(d0)",
    );
    let (spec, terms) = load_rec_from_strings(&[&text]).unwrap();

    assert_eq!(spec.variables, ["X", "Y", "Val", "Max"]);
    assert_eq!(spec.constructors, [("d0".to_string(), 0), ("s".to_string(), 1)]);
    assert_eq!(terms, [aterm("f(s(d0), d0)"), aterm("g(d0)")]);

    assert_eq!(spec.rewrite_rules.len(), 2);
    let conditional = &spec.rewrite_rules[0];
    assert_eq!(conditional.lhs, aterm("f(X, d0)"));
    assert_eq!(conditional.rhs, aterm("g(X)"));
    assert_eq!(conditional.conditions.len(), 2);
    assert_eq!(conditional.conditions[0].lhs, aterm("X"));
    assert_eq!(conditional.conditions[0].rhs, aterm("d0"));
    assert!(conditional.conditions[0].equality);
    assert_eq!(conditional.conditions[1].lhs, aterm("s(Y)"));
    assert_eq!(conditional.conditions[1].rhs, aterm("Y"));
    assert!(!conditional.conditions[1].equality);

    let plain = &spec.rewrite_rules[1];
    assert_eq!(plain.lhs, aterm("g(X)"));
    assert_eq!(plain.rhs, aterm("X"));
    assert!(plain.conditions.is_empty());
}

#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_load_rec_rejects_malformed_input() {
    for eval in ["f(d0", "f()", "f(d0,)"] {
        assert!(
            load_rec_from_strings(&[&spec_text("g(X) = X", eval)]).is_err(),
            "the term {eval} should be rejected"
        );
    }

    for rule in ["f(X, Y) = g(X) if X <= Y", "f(X, Y) = "] {
        assert!(
            load_rec_from_strings(&[&spec_text(rule, "g(d0)")]).is_err(),
            "the rule {rule} should be rejected"
        );
    }

    assert!(load_rec_from_strings(&[&spec_text("g(X) = X", "f(d0, s(d0))")]).is_ok());
}

/// The specifications passed together are merged: the rules and variables of the included
/// specification, the terms of both.
#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_load_rec_from_strings_merges_specifications() {
    let (spec, terms) = load_rec_from_strings(&[
        include_str!("../../../../examples/REC/rec/factorial5.rec"),
        include_str!("../../../../examples/REC/rec/factorial.rec"),
    ])
    .unwrap();

    assert_eq!(terms, [aterm("fact(s(s(s(s(s(d0))))))")]);
    assert_eq!(spec.variables, ["N", "M"]);
    assert_eq!(spec.constructors, [("d0".to_string(), 0), ("s".to_string(), 1)]);
    assert_eq!(spec.rewrite_rules.len(), 6);
    assert_eq!(spec.rewrite_rules[5].lhs, aterm("fact(s(N))"));
    assert_eq!(spec.rewrite_rules[5].rhs, aterm("times(s(N), fact(N))"));
}

/// Loading from a file resolves the included specifications next to it, giving the same
/// result as passing them explicitly.
#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn test_load_rec_from_file_resolves_includes() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../examples/REC/rec");

    let (from_file, file_terms) = load_rec_from_file(&directory.join("missionaries3.rec")).unwrap();
    let (from_strings, string_terms) = load_rec_from_strings(&[
        include_str!("../../../../examples/REC/rec/missionaries3.rec"),
        include_str!("../../../../examples/REC/rec/missionaries.rec"),
    ])
    .unwrap();

    assert!(!from_file.rewrite_rules.is_empty());
    assert_eq!(from_file.rewrite_rules, from_strings.rewrite_rules);
    assert_eq!(from_file.variables, from_strings.variables);
    assert_eq!(from_file.constructors, from_strings.constructors);
    assert_eq!(file_terms, string_terms);
}
