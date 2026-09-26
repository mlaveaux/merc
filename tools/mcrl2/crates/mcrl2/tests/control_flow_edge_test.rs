//! Regression tests for [`mcrl2::ControlFlowGraph`]'s edge construction,
//! exercising `crates/mcrl2/src/control_flow.rs` directly (see
//! `review/phase-6-mcrl2-misc.md`).
//!
//! These tests build a tiny LPS with `read_lps_text`, then feed
//! `ControlFlowGraph::new` a hand-picked `live` predicate so a summand that
//! writes a control flow parameter to a non-constant expression can be
//! included in the graph while still being excluded from disqualifying that
//! parameter (exactly the situation `merc_pbes::CfgPbesSrfLps` relies on for
//! summands of an unreachable SRF equation, see `is_control_flow_parameter`'s
//! doc comment in `control_flow.rs`).

use std::io::Write;

use mcrl2::ATerm;
use mcrl2::ATermList;
use mcrl2::CfgEdge;
use mcrl2::CfgSummand;
use mcrl2::ControlFlowGraph;
use mcrl2::DataExpression;
use mcrl2::DataExpressionRef;
use mcrl2::DataVariable;
use mcrl2::LearnSuccessorsContext;
use mcrl2::read_lps_text;

/// A minimal [`CfgSummand`] built directly from an LPS summand's condition and
/// assignments, with an independently chosen liveness flag so tests can probe
/// `live`/dead behaviour without needing a real reachability analysis.
struct TestSummand {
    condition: DataExpression,
    write_assignments: ATermList<ATerm>,
    live: bool,
}

impl CfgSummand for TestSummand {
    fn condition(&self) -> &DataExpression {
        &self.condition
    }

    fn write_assignments(&self) -> &ATermList<ATerm> {
        &self.write_assignments
    }
}

/// Writes `text` to a fresh temporary `.mcrl2` file and parses/linearises it
/// with `read_lps_text`.
fn parse_lps(text: &str) -> mcrl2::LinearProcessSpecification {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "merc_control_flow_edge_test_{}_{}.mcrl2",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    {
        let mut file = std::fs::File::create(&path).expect("Failed to create temp LPS file");
        file.write_all(text.as_bytes()).expect("Failed to write temp LPS file");
    }

    let result = read_lps_text(path.to_str().unwrap()).expect("Failed to parse/linearise LPS");
    let _ = std::fs::remove_file(&path);
    result
}

/// A summand that writes `d` to another (free) process parameter `e` leaves
/// `d`'s value genuinely unknown after firing: it is emphatically *not* a
/// self-loop, since `e` need not equal `d`'s source location. A summand that
/// leaves `d` untouched altogether *is* a self-loop.
///
/// [`CfgEdge::target`]'s doc comment says `None` means the edge "arrives back
/// at `source` (a self-loop)" — i.e. `target: None` is documented to mean
/// "unchanged", not "changed to an unknown value". This test builds one
/// summand of each kind (both guarded by the same `d == 1` source location,
/// both excluded from `live` so their non-constant write cannot disqualify
/// `d` as a control flow parameter — see `is_control_flow_parameter`) and
/// checks that `ControlFlowGraph` reports two different edges for them.
///
/// It currently does not: both summands normalise to
/// `CfgEdge { source: Some(_), target: None }`, because
/// `ControlFlowGraph::new` collapses "no assignment" and "assigned a
/// non-constant expression" (`SummandAnalysis::target`'s `None`-shaped key
/// missing vs. present-with-`None`-value) into the exact same `Option<V>` when
/// building `CfgEdge`. A caller reading only `edges()` cannot tell these two
/// summands apart, contradicting `CfgEdge::target`'s own doc comment.
#[test]
fn unchanged_edge_is_distinguishable_from_non_constant_write_edge() {
    let text = "act a, b, c;
proc P(d: Nat, e: Nat) =
  (d == 1) -> a . P(e, e) +
  (d == 1) -> b . P(d, e) +
  (d == 2) -> c . P(3, e);
init P(0, 0);
";
    let lps = parse_lps(text);
    assert_eq!(lps.num_summands(), 3, "expected the three declared alternatives to linearise 1:1");

    let parameters: Vec<DataVariable> = lps.parameters().iter().collect();
    assert_eq!(parameters.len(), 2, "expected exactly `d` and `e` as process parameters");

    let context = LearnSuccessorsContext::new(&lps);

    // Only summand 2 (`d == 2 -> c . P(3, e)`, a constant write to `d`) is
    // live. This is enough for `d` to be classified as a control flow
    // parameter (it is both read via `d == 2` and only ever written a
    // constant by every *live* summand), while summands 0 and 1 remain dead
    // and hence exempt from the "writes must be constant" requirement.
    let summands: Vec<TestSummand> = (0..lps.num_summands())
        .map(|i| {
            let summand = lps.action_summand(i).expect("summand must exist");
            TestSummand {
                condition: summand.condition(),
                write_assignments: summand.assignments(),
                live: i == 2,
            }
        })
        .collect();

    let mut cache: Vec<String> = Vec::new();
    let mut intern = |value: &DataExpressionRef<'_>| -> usize {
        let text = value.protect().to_string();
        if let Some(pos) = cache.iter().position(|c| c == &text) {
            pos
        } else {
            cache.push(text);
            cache.len() - 1
        }
    };

    let graph = ControlFlowGraph::new(&parameters, &summands, |s: &TestSummand| s.live, &context, &mut intern);

    assert_eq!(
        graph.control_flow_parameters().len(),
        1,
        "`d` must be classified as a control flow parameter: it is read via `d == 2` in the live \
         summand, and every live summand's write to it is constant"
    );

    let unknown_write_edges: Vec<CfgEdge<usize>> = graph.edges(0).to_vec();
    let unchanged_edges: Vec<CfgEdge<usize>> = graph.edges(1).to_vec();

    assert_eq!(unknown_write_edges.len(), 1, "summand 0 guards `d`, so it must contribute one edge");
    assert_eq!(unchanged_edges.len(), 1, "summand 1 guards `d`, so it must contribute one edge");

    // Both summands guard `d == 1`, so both edges have the same source.
    assert_eq!(unknown_write_edges[0].source, unchanged_edges[0].source);

    // But summand 0 actually overwrites `d` with the unrelated parameter `e`
    // (its value after firing is *not* pinned to the source location, or to
    // any fixed location at all), while summand 1 truly leaves `d` unchanged
    // (a genuine self-loop). These must not be reported as the same edge.
    assert_ne!(
        unknown_write_edges[0], unchanged_edges[0],
        "a summand that overwrites `d` with a non-constant expression (`d := e`) must not be \
         reported identically to a summand that truly leaves `d` unchanged: both currently \
         normalise to `target: None`, silently reporting the non-constant write as a self-loop"
    );
}
