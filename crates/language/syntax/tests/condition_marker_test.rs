//! Process conditions (`c -> p <> q`): the parse tree and the spans refer to the original text.

use std::ops::ControlFlow;

use merc_syntax::ProcessExprKind;
use merc_syntax::Traverse;
use merc_syntax::UntypedProcessSpecification;

/// A chain that is exponential for the unmarked grammar, with spans that must still refer to the
/// original text.
#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn long_condition_chain_parses_with_correct_spans() {
    let terms: Vec<String> = (0..200).map(|i| format!("(v == {i}) -> a{i}(v) . P(v)")).collect();
    let spec = format!("act a0: Nat;\nproc P(v: Nat) = {};\ninit P(0);", terms.join(" + "));

    let parsed = UntypedProcessSpecification::parse(&spec).unwrap();

    let body = &parsed.process_declarations[0].body;
    assert_eq!(&spec[body.span.start..body.span.end], terms.join(" + "));
    // Every guard is parsed as a condition (the `+` operands are absorbed into the data
    // expressions of the conditions, which `merc_process` disambiguates later).
    let mut conditions = 0;
    body.visit::<(), _>(|node| {
        if let ProcessExprKind::Condition { else_, .. } = &node.node {
            assert!(else_.is_none());
            conditions += 1;
        }
        ControlFlow::Continue(())
    });
    assert_eq!(conditions, 200);
}

/// `x -> y -> a <> b <> e` pairs the first `<>` with `y` and the second with `x`, as mCRL2 does.
#[test]
#[cfg_attr(miri, ignore)] // pest calls into psm, which miri does not support.
fn dangling_else_binds_to_nearest_if() {
    let spec = "act a, b, e;\nproc P(x, y: Bool) = x -> y -> a <> b <> e;\ninit P(true, true);";

    let parsed = UntypedProcessSpecification::parse(spec).unwrap();

    assert_eq!(
        parsed.process_declarations[0].body.to_string(),
        "((x) -> (((y) -> (a) <> (b))) <> (e))"
    );
}
