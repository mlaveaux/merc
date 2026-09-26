//! Regression tests for parser bugs fixed during the crate review, plus
//! randomized print/parse round-trip (string-fixpoint) property tests.

use std::ops::ControlFlow;

use rand::RngExt;

use merc_syntax::ActFrmKind;
use merc_syntax::Bound;
use merc_syntax::PbesExprKind;
use merc_syntax::ProcExprBinaryOp;
use merc_syntax::ProcessExprKind;
use merc_syntax::RegFrmKind;
use merc_syntax::StateFrmKind;
use merc_syntax::Traverse;
use merc_syntax::UntypedDataSpecification;
use merc_syntax::UntypedPbes;
use merc_syntax::UntypedPres;
use merc_syntax::UntypedProcessSpecification;
use merc_syntax::UntypedStateFrmSpec;
use merc_syntax::make_process_specification;
use merc_syntax::random_data_specification;
use merc_syntax::random_lps;
use merc_syntax::random_pbes;
use merc_syntax::random_pbes_with_data_specification;
use merc_syntax::random_pres;
use merc_utilities::random_test;

/// PBES quantifiers used to panic because `forall`/`exists` were registered as
/// prefix operators but only handled in the postfix closure.
#[test]
fn pbes_quantifiers_parse() {
    let pbes = UntypedPbes::parse("pbes mu X = forall n: Nat . val(n < 3) => exists m: Nat . val(m < n); init X;")
        .expect("PBES with quantifiers should parse");

    let formula = &pbes.equations[0].formula;
    assert!(
        matches!(
            formula.node,
            PbesExprKind::Quantifier { .. } | PbesExprKind::Binary { .. }
        ),
        "unexpected formula: {formula:?}"
    );
}

/// `sum`/`inf`/`sup` state-formula operators used to all collapse to `Bound::Sup`.
#[test]
fn state_formula_bounds_are_distinct() {
    for (input, expected) in [
        ("sum n: Nat . val(n < 3)", Bound::Sum),
        ("inf n: Nat . val(n < 3)", Bound::Inf),
        ("sup n: Nat . val(n < 3)", Bound::Sup),
    ] {
        let spec = UntypedStateFrmSpec::parse(input).expect("state formula should parse");
        match spec.formula.node {
            StateFrmKind::Bound { bound, .. } => assert_eq!(bound, expected, "for input {input:?}"),
            other => panic!("expected a Bound for {input:?}, got {other:?}"),
        }
    }
}

/// The process `<<` (until) operator used to hit `unimplemented!`.
#[test]
fn process_until_operator_parses() {
    let spec = UntypedProcessSpecification::parse("init a << b;").expect("`<<` should parse");
    match spec.init.expect("init present").node {
        ProcessExprKind::Binary { op, .. } => assert_eq!(op, ProcExprBinaryOp::Until),
        other => panic!("expected a binary Until, got {other:?}"),
    }
}

/// A bare `delay` / `yaled` (without `@time`) used to fail to parse.
#[test]
fn bare_delay_and_yaled_parse() {
    assert!(matches!(
        UntypedStateFrmSpec::parse("delay").unwrap().formula.node,
        StateFrmKind::Delay(None)
    ));
    assert!(matches!(
        UntypedStateFrmSpec::parse("yaled").unwrap().formula.node,
        StateFrmKind::Yaled(None)
    ));
    assert!(matches!(
        UntypedStateFrmSpec::parse("delay@(3)").unwrap().formula.node,
        StateFrmKind::Delay(Some(_))
    ));
}

/// The action-formula `@` (timed action) postfix operator used to panic in the Pratt parser: it
/// was registered as a postfix operator (`Rule::ActFrmAt`) but had no `ActFrmKind` variant and no
/// `.map_postfix` handler, so pest's `unwrap()` on the missing mapping crashed.
#[test]
fn action_formula_at_parses() {
    let spec = UntypedStateFrmSpec::parse("<a@3>true").expect("`@` on an action formula should parse");
    let StateFrmKind::Modality { formula, .. } = &spec.formula.node else {
        panic!("expected a modality, got {:?}", spec.formula.node);
    };
    let RegFrmKind::Action(action) = &formula.node else {
        panic!("expected an action formula, got {:?}", formula.node);
    };
    assert!(
        matches!(action.node, ActFrmKind::At { .. }),
        "expected ActFrmKind::At, got {:?}",
        action.node
    );
}

/// The left-merge operator must print as `||_` so that the output reparses.
#[test]
fn left_merge_round_trips() {
    let printed = format!("{}", UntypedProcessSpecification::parse("init a ||_ b;").unwrap());
    assert!(printed.contains("||_"), "left merge should print as ||_: {printed}");
    UntypedProcessSpecification::parse(&printed).expect("printed left merge should reparse");
}

#[test]
fn action_formula_at_round_trips() {
    let printed = format!("{}", UntypedStateFrmSpec::parse("<a@3>true").unwrap());
    UntypedStateFrmSpec::parse(&printed).expect("printed `@` action formula should reparse");
}

/// A PRES used the `pbes` keyword, dropped infix operators, and mismatched the
/// constant-multiply rules. Exercise a spec that touches all of those.
#[test]
fn pres_specification_parses() {
    let pres = UntypedPres::parse(
        "pres \
           mu X(n: Nat) = (val(n < 3) => X(n)) && eqinf(X(n)) + val(2) * X(n); \
           nu Y = sup m: Nat . (Y + condsm(Y, Y, Y)); \
         init X(0);",
    )
    .expect("PRES should parse");
    assert_eq!(pres.equations.len(), 2);
}

/// Test that PRES examples print and parse back to the same form.
#[test]
fn pres_examples_print_parse_fixpoint() {
    let examples = [
        "pres mu X = true; init X;",
        "pres mu X = false; init X;",
        "pres mu X = val(1); init X;",
        "pres mu X = Y; nu Y = X; init Y;",
        "pres mu X = eqinf(val(1)); init X;",
        "pres mu X = eqninf(val(1)); init X;",
        "pres mu X = condsm(val(1), val(2), val(3)); init X;",
        "pres mu X = condeq(val(1), val(2), val(3)); init X;",
        "pres mu X = -val(1); init X;",
        "pres mu X = val(2) * val(1); init X;",
        "pres mu X = val(1) * val(2); init X;",
        "pres mu X = val(1) + val(2); init X;",
        "pres mu X = val(1) => val(2); init X;",
        "pres mu X = val(1) || val(2); init X;",
        "pres mu X = val(1) && val(2); init X;",
        "pres mu X(n: Nat) = inf n: Nat . val(n); init X(0);",
        "pres mu X(n: Nat) = sup n: Nat . val(n); init X(0);",
        "pres mu X(n: Nat) = sum n: Nat . val(n); init X(0);",
        "pres \
           mu X(n: Nat) = (val(n < 3) => X(n)) && eqinf(X(n)) + val(2) * X(n); \
           nu Y = sup m: Nat . (Y + condsm(Y, Y, Y)); \
         init X(0);",
    ];

    for input in examples {
        let spec = UntypedPres::parse(input).unwrap_or_else(|e| panic!("failed to parse {input:?}: {e}"));
        let printed = format!("{spec}");
        let reparsed = UntypedPres::parse(&printed)
            .unwrap_or_else(|e| panic!("printed PRES failed to reparse:\n{printed}\nerror: {e}"));
        assert_eq!(
            printed,
            format!("{reparsed}"),
            "PRES print/parse not a fixpoint for input {input:?}"
        );
    }
}

/// `visit_*` must return a `Break` value produced by a nested (non-root) node.
#[test]
fn visitor_breaks_from_nested_node() {
    // The `Y` identifier only appears below the top-level conjunction.
    let spec = UntypedStateFrmSpec::parse("true && (mu X. (X && Y))").unwrap();

    let found = spec.formula.visit(|frm| {
        if let StateFrmKind::Id(name, _) = &frm.node
            && name == "Y"
        {
            return ControlFlow::Break(name.clone());
        }
        ControlFlow::Continue(())
    });

    assert_eq!(found.as_deref(), Some("Y"), "Break value from a nested node was lost");
}

/// An `EqnSpec` without a `var` declaration section must not emit an empty
/// `var` section when printed. The grammar requires at least one declaration
/// after `var`, so printing an empty `var\n` block produces output that cannot
/// be parsed back.
#[test]
fn eqn_spec_without_variables_no_empty_var_section() {
    let spec = UntypedDataSpecification::parse("eqn true = false;").expect("eqn without var should parse");
    let printed = format!("{spec}");
    assert!(
        !printed.contains("var\n"),
        "empty var section must not be emitted:\n{printed}"
    );
    UntypedDataSpecification::parse(&printed).expect("re-printed form must parse");
}

/// Action declarations with sort arguments used to be printed as `id(s1, s2)`
/// instead of the correct `id: s1 # s2` form. The incorrect form would fail to
/// reparse because the grammar expects a colon-separated sort product.
#[test]
fn act_decl_with_args_prints_colon_hash() {
    let spec = UntypedProcessSpecification::parse("act a: Bool # Nat;")
        .expect("action declaration with sort args should parse");
    let printed = format!("{spec}");
    assert!(
        printed.contains("a: Bool # Nat"),
        "ActDecl with args must use ':' and '#':\n{printed}"
    );
    UntypedProcessSpecification::parse(&printed).expect("printed form must reparse");
}

/// A nullary struct constructor's `?is_foo` recognizer was dropped when printing:
/// `ConstructorDecl::fmt` returned early for the no-`args` case before checking
/// `self.recogniser`, so e.g. `struct b1?is_b1 | b2?is_b2` reprinted without either
/// recognizer, silently losing the generated `is_b1`/`is_b2` projection functions.
#[test]
fn nullary_struct_constructor_recognizer_is_printed() {
    let spec = UntypedProcessSpecification::parse("sort D = struct b1?is_b1 | b2?is_b2;\ninit delta;")
        .expect("struct with nullary-constructor recognizers should parse");
    let printed = format!("{spec}");
    assert!(
        printed.contains("b1?is_b1"),
        "recognizer on b1 must be printed:\n{printed}"
    );
    assert!(
        printed.contains("b2?is_b2"),
        "recognizer on b2 must be printed:\n{printed}"
    );
    UntypedProcessSpecification::parse(&printed).expect("printed form must reparse");
}

/// Property: for every generated AST, the printed form parses, and printing the
/// reparsed AST yields exactly the same string (a fixpoint of `parse ∘ display`).
/// This catches Display/grammar mismatches without depending on `PartialEq`
/// surviving parenthesization.

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn random_lps_print_parse_fixpoint() {
    random_test(100, |rng| {
        let spec = random_lps(rng, 4, 3, 0.6);
        let printed = format!("{spec}");
        let reparsed = UntypedProcessSpecification::parse(&printed)
            .unwrap_or_else(|e| panic!("failed to reparse generated LPS:\n{printed}\nerror: {e}"));
        assert_eq!(printed, format!("{reparsed}"), "LPS print/parse not a fixpoint");
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn random_process_spec_print_parse_fixpoint() {
    random_test(100, |rng| {
        let use_integers = rng.random_bool(0.5);
        let spec = make_process_specification(rng, 3, 4, use_integers);
        let printed = format!("{spec}");
        let reparsed = UntypedProcessSpecification::parse(&printed)
            .unwrap_or_else(|e| panic!("failed to reparse generated process spec:\n{printed}\nerror: {e}"));
        assert_eq!(
            printed,
            format!("{reparsed}"),
            "process spec print/parse not a fixpoint"
        );
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn random_pbes_print_parse_fixpoint() {
    random_test(100, |rng| {
        let use_quantifiers = rng.random_bool(0.5);
        let use_integers = rng.random_bool(0.5);
        let pbes = random_pbes(rng, 3, 4, 4, use_quantifiers, use_integers);
        let printed = format!("{pbes}");
        let reparsed = UntypedPbes::parse(&printed)
            .unwrap_or_else(|e| panic!("failed to reparse generated PBES:\n{printed}\nerror: {e}"));
        assert_eq!(printed, format!("{reparsed}"), "PBES print/parse not a fixpoint");
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn random_pbes_with_data_specification_print_parse_fixpoint() {
    random_test(100, |rng| {
        let use_quantifiers = rng.random_bool(0.5);
        let use_integers = rng.random_bool(0.5);
        let pbes = random_pbes_with_data_specification(rng, 4, 2, 3, 4, 4, use_quantifiers, use_integers);
        let printed = format!("{pbes}");
        let reparsed = UntypedPbes::parse(&printed)
            .unwrap_or_else(|e| panic!("failed to reparse generated PBES:\n{printed}\nerror: {e}"));
        assert_eq!(printed, format!("{reparsed}"), "PBES print/parse not a fixpoint");
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn random_pres_print_parse_fixpoint() {
    random_test(100, |rng| {
        let use_bounds = rng.random_bool(0.5);
        let pres = random_pres(rng, 3, 4, 4, use_bounds);
        let printed = format!("{pres}");
        let reparsed = UntypedPres::parse(&printed)
            .unwrap_or_else(|e| panic!("failed to reparse generated PRES:\n{printed}\nerror: {e}"));
        assert_eq!(printed, format!("{reparsed}"), "PRES print/parse not a fixpoint");
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn random_data_specification_print_parse_fixpoint() {
    random_test(100, |rng| {
        let spec = random_data_specification(rng, 6, 3);
        let printed = format!("{spec}");
        let reparsed = UntypedDataSpecification::parse(&printed)
            .unwrap_or_else(|e| panic!("failed to reparse generated data specification:\n{printed}\nerror: {e}"));
        assert_eq!(
            printed,
            format!("{reparsed}"),
            "data specification print/parse not a fixpoint"
        );
    });
}
