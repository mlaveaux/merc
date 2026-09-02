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

/// Specs whose repeated condition-side subterm shows up at points the
/// matching rules themselves don't structurally guarantee are related —
/// oddeven's mutually-recursive `odd`/`even` conditions, evalexpr's shared
/// `eval` subexpressions, tak18's shared recursive calls — so a full-size
/// [ConditionCache] takes markedly fewer steps than a 1-entry one that is
/// evicted before a subterm's second use arrives.
const CONDITION_CACHE_CASES: &[(&str, &[&str])] = &[
    ("oddeven", &[include_str!("../../../../examples/REC/rec/oddeven.rec")]),
    ("evalexpr", &[include_str!("../../../../examples/REC/rec/evalexpr.rec")]),
    (
        "tak18",
        &[
            include_str!("../../../../examples/REC/rec/tak18.rec"),
            include_str!("../../../../examples/REC/rec/tak.rec"),
        ],
    ),
];

pub fn criterion_benchmark_condition_cache(c: &mut Criterion) {
    for (name, rec_files) in CONDITION_CACHE_CASES {
        let (syntax_spec, syntax_terms) = load_rec_from_strings(rec_files).unwrap();
        let spec = syntax_spec.to_rewrite_spec();
        let terms: Vec<_> = syntax_terms
            .iter()
            .map(|term| to_untyped_data_expression(term.clone(), None))
            .collect();

        c.bench_function(&format!("condition cache innermost 1-entry {name}"), |bencher| {
            bencher.iter(|| {
                let mut inner = InnermostRewriter::with_condition_cache_capacity(&spec, 1);
                for term in &terms {
                    let _ = black_box(inner.rewrite(term));
                }
            });
        });

        c.bench_function(&format!("condition cache innermost default {name}"), |bencher| {
            bencher.iter(|| {
                let mut inner = InnermostRewriter::new(&spec);
                for term in &terms {
                    let _ = black_box(inner.rewrite(term));
                }
            });
        });

        c.bench_function(&format!("condition cache sabre 1-entry {name}"), |bencher| {
            bencher.iter(|| {
                let mut sabre = SabreRewriter::with_condition_cache_capacity(&spec, 1);
                for term in &terms {
                    let _ = black_box(sabre.rewrite(term));
                }
            });
        });

        c.bench_function(&format!("condition cache sabre default {name}"), |bencher| {
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
