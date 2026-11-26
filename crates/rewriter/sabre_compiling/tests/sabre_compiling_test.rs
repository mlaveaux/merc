use test_log::test;

use merc_data::to_untyped_data_expression;
use merc_rec_tests::load_rec_from_strings;
use merc_sabre::InnermostRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre_compiling::SabreCompilingRewriter;

/// The compiled rewriter computes the same normal form as the interpreted innermost rewriter.
#[test]
#[cfg_attr(miri, ignore)] // Miri does not support FFI.
fn test_sabre_compiling_factorial() {
    let (spec, terms) = load_rec_from_strings(&[
        include_str!("../../../../examples/REC/rec/factorial5.rec"),
        include_str!("../../../../examples/REC/rec/factorial.rec"),
    ])
    .unwrap();
    assert_eq!(terms.len(), 1, "factorial5 evaluates one term");

    let spec = spec.to_rewrite_spec();
    let mut rewriter = SabreCompilingRewriter::new(&spec, true, true).unwrap();
    let mut oracle = InnermostRewriter::new(&spec);

    for t in terms {
        let data_term = to_untyped_data_expression(t, None);
        let rewritten_term = rewriter.rewrite(&data_term);

        assert_eq!(rewritten_term, oracle.rewrite(&data_term));
        assert_eq!(
            rewritten_term.to_string().chars().filter(|c| *c == 's').count(),
            120, // 5! = 120.
            "The rewritten result does not match the expected result"
        );
    }
}
