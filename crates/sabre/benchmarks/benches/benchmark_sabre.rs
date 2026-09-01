#![forbid(unsafe_code)]

use std::hint::black_box;

use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;

use merc_data::to_untyped_data_expression;
use merc_rec_tests::load_rec_from_strings;
use merc_sabre::InnermostRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre::SabreRewriter;
use merc_sabre::SetAutomaton;

const CASES: &[(&str, &[&str])] = &[
    ("fibfree", &[include_str!("../../../../examples/REC/rec/fibfree.rec")]),
    (
        "benchexpr20",
        &[
            include_str!("../../../../examples/REC/rec/benchexpr20.rec"),
            include_str!("../../../../examples/REC/rec/asfsdfbenchmark.rec"),
        ],
    ),
    ("evalexpr", &[include_str!("../../../../examples/REC/rec/evalexpr.rec")]),
    (
        "sieve1000",
        &[
            include_str!("../../../../examples/REC/rec/sieve1000.rec"),
            include_str!("../../../../examples/REC/rec/sieve.rec"),
        ],
    ),
    (
        "tak36",
        &[
            include_str!("../../../../examples/REC/rec/tak36.rec"),
            include_str!("../../../../examples/REC/rec/tak.rec"),
        ],
    ),
];

pub fn criterion_benchmark_set_automaton(c: &mut Criterion) {
    for (name, rec_files) in CASES {
        let (syntax_spec, _) = load_rec_from_strings(rec_files).unwrap();
        let result = syntax_spec.to_rewrite_spec();

        c.bench_function(&format!("set automaton {}", name), |bencher| {
            bencher.iter(|| {
                let _ = black_box(SetAutomaton::new(&result, |_| (), false));
            });
        });

        c.bench_function(&format!("apma automaton {}", name), |bencher| {
            bencher.iter(|| {
                let _ = black_box(SetAutomaton::new(&result, |_| (), true));
            });
        });
    }
}

pub fn criterion_benchmark_innermost_rewriter(c: &mut Criterion) {
    for (name, rec_files) in CASES {
        let (syntax_spec, syntax_terms) = load_rec_from_strings(rec_files).unwrap();
        let spec = syntax_spec.to_rewrite_spec();
        let terms: Vec<_> = syntax_terms
            .iter()
            .map(|term| to_untyped_data_expression(term.clone(), None))
            .collect();

        c.bench_function(&format!("innermost rewriter {}", name), |bencher| {
            bencher.iter(|| {
                let mut inner = InnermostRewriter::new(&spec);
                for term in &terms {
                    let _ = black_box(inner.rewrite(term));
                }
            });
        });
    }
}

pub fn criterion_benchmark_sabre_rewriter(c: &mut Criterion) {
    for (name, rec_files) in CASES {
        let (syntax_spec, syntax_terms) = load_rec_from_strings(rec_files).unwrap();
        let spec = syntax_spec.to_rewrite_spec();
        let terms: Vec<_> = syntax_terms
            .iter()
            .map(|term| to_untyped_data_expression(term.clone(), None))
            .collect();

        c.bench_function(&format!("sabre rewriter {}", name), |bencher| {
            bencher.iter(|| {
                let mut sabre = SabreRewriter::new(&spec);
                for term in &terms {
                    let _ = black_box(sabre.rewrite(term));
                }
            });
        });
    }
}

/// Builds a `s(s(...d0...))` literal with `n` applications of `s`, for the
/// [criterion_benchmark_condition_cache] specs below.
fn nat_literal(n: u32) -> String {
    let mut result = String::from("d0");
    for _ in 0..n {
        result = format!("s({result})");
    }
    result
}

/// Two REC specs whose only difference is whether a rule's two conditions
/// mention the exact same subterm (`shared`, where `matching::condition_cache`
/// applies) or two different variables that merely happen to be bound to
/// equal values at the call site below (`unshared`, where it does not):
/// `slow` takes `n` steps to reach a normal form, so the gap between them
/// isolates the effect of normalising it once instead of twice.
const CONDITION_CACHE_N: u32 = 400;

pub fn criterion_benchmark_condition_cache(c: &mut Criterion) {
    let nat = nat_literal(CONDITION_CACHE_N);
    let cases: &[(&str, String)] = &[
        (
            "shared",
            format!(
                "REC-SPEC ConditionCacheShared\n\
                 SORTS\n  Nat Bool\n\
                 CONS\n  true : -> Bool\n  false : -> Bool\n  d0 : -> Nat\n  s : Nat -> Nat\n\
                 OPNS\n  slow : Nat -> Bool\n  check : Nat -> Bool\n\
                 VARS\n  N : Nat\n\
                 RULES\n  \
                 slow(d0) -> true\n  \
                 slow(s(N)) -> slow(N)\n  \
                 check(N) -> true if slow(N) = true and-if slow(N) = true\n\
                 EVAL\n\ncheck({nat})\n# result: true\n\nEND-SPEC\n"
            ),
        ),
        (
            "unshared",
            format!(
                "REC-SPEC ConditionCacheUnshared\n\
                 SORTS\n  Nat Bool\n\
                 CONS\n  true : -> Bool\n  false : -> Bool\n  d0 : -> Nat\n  s : Nat -> Nat\n\
                 OPNS\n  slow : Nat -> Bool\n  check : Nat Nat -> Bool\n\
                 VARS\n  N M : Nat\n\
                 RULES\n  \
                 slow(d0) -> true\n  \
                 slow(s(N)) -> slow(N)\n  \
                 check(N, M) -> true if slow(N) = true and-if slow(M) = true\n\
                 EVAL\n\ncheck({nat}, {nat})\n# result: true\n\nEND-SPEC\n"
            ),
        ),
    ];

    for (name, rec_source) in cases {
        let (syntax_spec, syntax_terms) = load_rec_from_strings(&[rec_source.as_str()]).unwrap();
        let spec = syntax_spec.to_rewrite_spec();
        let terms: Vec<_> = syntax_terms
            .iter()
            .map(|term| to_untyped_data_expression(term.clone(), None))
            .collect();

        c.bench_function(&format!("condition cache innermost {}", name), |bencher| {
            bencher.iter(|| {
                let mut inner = InnermostRewriter::new(&spec);
                for term in &terms {
                    let _ = black_box(inner.rewrite(term));
                }
            });
        });

        c.bench_function(&format!("condition cache sabre {}", name), |bencher| {
            bencher.iter(|| {
                let mut sabre = SabreRewriter::new(&spec);
                for term in &terms {
                    let _ = black_box(sabre.rewrite(term));
                }
            });
        });
    }
}

criterion_group!(
    benches,
    criterion_benchmark_set_automaton,
    criterion_benchmark_innermost_rewriter,
    criterion_benchmark_sabre_rewriter,
    criterion_benchmark_condition_cache,
);
criterion_main!(benches);
