//! Strategy improvement on the exact play-value valuation of Vöge and Jurdziński.
//!
//! > Jens Vöge and Marcin Jurdziński. A Discrete Strategy Improvement
//! > Algorithm for Solving Parity Games. CAV 2000, pages 202-215.
//!
//! Two algorithms are provided, both operating on a strategy *profile*: a
//! single array `p` holding Even's strategy `σ` at Even vertices and Odd's
//! strategy `τ` at Odd vertices.
//!
//! - [`StrategyImprovementAlgorithm::VogeJurdzinski`]: classic switch-all
//!   strategy improvement for Even, where Odd plays a best response.
//! - [`StrategyImprovementAlgorithm::HybridB`]: the cross-informed variant
//!   H-B. Let `σ̄` be Odd's best response to `σ`, with valuation `Ξ_σ`, and
//!   `σ*` Even's best response to `σ̄`. Of the improving edges `I(v)` only
//!   those that are optimal against `σ̄` (w.r.t. the valuation of `σ*`) are
//!   kept, `F(v)`. If some `F(v)` is non-empty exactly those vertices switch
//!   to their `Ξ_σ`-best edge in `F(v)`, otherwise all improvable vertices
//!   switch to their `Ξ_σ`-best improving edge (the global fallback).
//!
//! # Definitions
//!
//! The definitions are those of the merc website
//! (`developer/parity/hybrid-strategy-improvement.md`, Definitions 1 to 11),
//! which are formalised in `merc-verified` (`MercLib/HybridSI`). Write `n` for
//! the number of vertices, `m` for the number of edges and `G` for the
//! [`RelevancyGame`] (Definition 3), in which the relevance order is the order
//! on the indices, so the most relevant vertex of a set is its maximum.
//!
//! - **Profile** (Definition 4): an array `p` with `p[v]` a successor of `v`,
//!   holding Even's strategy `σ` at Even vertices and Odd's strategy `τ` at
//!   Odd vertices. Runs start from `p₀(v) = min succ(v)`, see
//!   [`initial_profile`].
//! - **Play value** (Definition 5): the play from `v` under `p` is a lasso,
//!   and its value is `Ξ_p(v) = (λ, B, k)` with `λ` the maximum of the cycle,
//!   `B` the vertices greater than `λ` visited before `λ` and `k` the number of
//!   steps to `λ`. See [`Valuation`].
//! - **Order** (Definition 6): first the reward of `λ`, then `B` by `d =
//!   max(B_a Δ B_b)` (the set containing `d` is better iff `pr(d)` is even),
//!   then `k` (shorter is better iff `pr(λ)` is even). See
//!   [`Valuation::compare`].
//! - **Best response and valuation** (Definition 7): `σ̄` is Odd's best
//!   response to `σ` (`ps` in the code) and `Ξ_σ = Ξ_(σ, σ̄)` the valuation of
//!   `σ` (`valuation`); `σ*` is Even's best response to `σ̄` (`pstar`) and `K* =
//!   Ξ_(σ*, σ̄)` its valuation (`response`). See
//!   [`StrategyImprovement::best_response`].
//! - **Improving edges** (Definition 8): `I_σ(v) = { w ∈ succ(v) | Ξ_σ(w) >
//!   Ξ_σ(σ(v)) }` for Even vertices `v`; `σ` is optimal iff all are empty. See
//!   [`improving_moves`].
//! - **Filter** (Definition 9): `F_σ(v) = { w ∈ I_σ(v) | K*(w) = max_{x ∈
//!   succ(v)} K*(x) }`, the improving edges that are also optimal against `σ̄`.
//!   See [`optimal_moves`].
//! - **Step** (Definition 10): with `S = F_σ` if some `F_σ(v)` is non-empty and
//!   `S = I_σ` otherwise (the fallback), every `v` with `S(v)` non-empty
//!   switches to the first `Ξ_σ`-maximum of `S(v)`. VJ always takes `S = I_σ`.
//!   See [`StrategyImprovement::run`].
//! - **Iterations and epochs** (Definition 11): an iteration is a step from a
//!   non-optimal `σ`, an epoch a maximal run of iterations with the same `σ̄`.
//!
//! # Cost
//!
//! - **One valuation** `Ξ_p` ([`Valuation::compute`]) costs `O(n log h)`,
//!   where `h ≤ n` is the maximal depth of a tail. Following the plays and
//!   detecting their cycles is `O(n)`, since every vertex is visited once; the
//!   binary lifting tables are `O(n log h)`. Afterwards one comparison costs
//!   `O(log h)`.
//! - **One best-response round** is one valuation plus one comparison per edge
//!   of the responding player, `O((n + m) log h)`. A best response takes
//!   several rounds: it is one-player switch-all strategy improvement, which
//!   is polynomial in practice, but no polynomial bound on its rounds is known.
//! - **One iteration** of VJ is one best response for Odd plus one comparison
//!   per Even edge to compute `I_σ`. One iteration of H-B adds a best response
//!   for Even and three comparisons per edge of an improvable vertex for
//!   `F_σ`.
//!
//! The statistic `best_response_rounds` is therefore the number of
//! valuations, and dominates the running time.
//!
//! # Reference
//!
//! Best responses are warm started from the given profile, keep the current
//! move unless a successor is strictly better, and take the first best
//! successor (in relevance order). This mirrors the Python reference
//! implementation `experiments/hybrid/scripts/algs.py` exactly, so that
//! iteration counts can be compared: `merc-vpg solve -v` logs them as JSON
//! key-values, and `experiments/hb_rust/scripts/check_reference.py` compares
//! them with the reference.

#![allow(clippy::needless_range_loop)] // Vertices index several parallel arrays.

use std::cmp::Ordering;
use std::fmt;
use std::time::Duration;
use std::time::Instant;

use bitvec::bitvec;
use bitvec::order::Lsb0;
use log::debug;
use log::trace;
use merc_utilities::MercError;

use crate::PG;
use crate::Player;
use crate::RelevancyGame;
use crate::Set;
use crate::Strategy;
use crate::VertexIndex;

/// The strategy improvement algorithm to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyImprovementAlgorithm {
    /// The cross-informed hybrid algorithm H-B.
    HybridB,
    /// The classic switch-all algorithm of Vöge and Jurdziński.
    VogeJurdzinski,
}

/// Options for [`solve_strategy_improvement`].
#[derive(Debug, Clone)]
pub struct StrategyImprovementOptions {
    /// The algorithm to run.
    pub algorithm: StrategyImprovementAlgorithm,
    /// For experiments only: start from this profile (one move per vertex,
    /// indexed by original vertex) instead of the initial profile `p_0`, the
    /// first successor of every vertex in relevance order.
    ///
    /// This allows adversarial starts, e.g. the initial strategies of lower-bound
    /// constructions or of counterexamples found by search. The algorithm is
    /// correct and terminates from any profile, but the statements of the
    /// formalisation, in particular the open polynomial bound on the number of
    /// iterations, are about runs from `p_0`.
    pub initial_profile: Option<Vec<VertexIndex>>,
    /// Stop with an error after this many iterations.
    pub max_iterations: Option<usize>,
    /// Compute the decisive-element statistics ([`DecisiveStatistics`]).
    pub decisive: bool,
}

impl StrategyImprovementOptions {
    /// Default options for the given algorithm.
    pub fn new(algorithm: StrategyImprovementAlgorithm) -> Self {
        Self {
            algorithm,
            initial_profile: None,
            max_iterations: None,
            decisive: false,
        }
    }
}

/// Statistics of a strategy improvement run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StrategyImprovementStatistics {
    /// The number of executed iterations, excluding the final optimality check.
    pub iterations: usize,
    /// One plus the number of changes of Odd's best response between consecutive
    /// executed iterations (zero if no iteration was executed).
    pub epochs: usize,
    /// The total number of valuations computed inside best responses.
    pub best_response_rounds: usize,
    /// The number of iterations that used the global fallback (H-B only).
    pub fallbacks: usize,
    /// The wall time of the run (excluding the relabelling).
    pub time: Duration,
    /// The decisive-element statistics, if requested.
    pub decisive: Option<DecisiveStatistics>,
}

/// Statistics of the decisive elements of a run (`experiments/hb_g1`, section 1).
///
/// Let `Ξ_k` be the valuation of Even's strategy in round `k = 0..T-1`, and
/// `Ξ_T` the final one. Round `k` is an *L-round* if the cycle maximum of some
/// vertex differs in `Ξ_k` and `Ξ_{k+1}`. Otherwise its decisive element
/// `δ_k` is the most relevant vertex (relevance index) of the symmetric
/// differences `B_k(v) Δ B_{k+1}(v)` over all `v`, or `-1` if there is none.
/// `N_x` counts the rounds with `δ_k = x`, and `N^I_x` the same within one
/// l-interval (a maximal run of non-L rounds).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecisiveStatistics {
    /// The number of L-rounds `NL`.
    pub l_rounds: usize,
    /// `max_x N_x`.
    pub max_count: usize,
    /// The element `x` (relevance index, `-1` for none) attaining `max_count`.
    pub max_count_element: i64,
    /// `max_{I,x} N^I_x`.
    pub max_interval_count: usize,
    /// The maximum number of consecutive executed rounds with the same `σ̄`.
    pub max_epoch_length: usize,
    /// The number of Even vertices `n_E`.
    pub even_vertices: usize,
}

impl fmt::Display for DecisiveStatistics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "l-rounds {}, max N_x {} (x = {}), max N^I_x {}, max epoch length {}, even vertices {}",
            self.l_rounds,
            self.max_count,
            self.max_count_element,
            self.max_interval_count,
            self.max_epoch_length,
            self.even_vertices
        )
    }
}

/// The part of a valuation needed for the decisive-element statistics.
struct Snapshot {
    profile: Vec<usize>,
    lambda: Vec<usize>,
    depth: Vec<usize>,
}

/// Returns the vertices of `B(x)` under the profile `f` in `out`, in
/// decreasing order.
///
/// Requires that `lambda` and `depth` are those of the [`Valuation`] of `f`.
fn b_set(f: &[usize], lambda: &[usize], depth: &[usize], x: usize, out: &mut Vec<usize>) {
    out.clear();
    let mut y = x;
    while depth[y] > 0 {
        if y > lambda[y] {
            out.push(y);
        }
        y = f[y];
    }
    out.sort_unstable_by(|a, b| b.cmp(a));
}

/// Returns the maximum of the symmetric difference of two sets, or `-1` if
/// they are equal.
///
/// Requires that both sets are given in decreasing order.
fn max_symmetric_difference(left: &[usize], right: &[usize]) -> i64 {
    // The common prefix is in both sets, the first difference is the maximum.
    match left.iter().zip(right).find(|(a, b)| a != b) {
        Some((a, b)) => (*a).max(*b) as i64,
        None => match left.len().cmp(&right.len()) {
            Ordering::Equal => -1,
            Ordering::Greater => left[right.len()] as i64,
            Ordering::Less => right[left.len()] as i64,
        },
    }
}

/// Returns the decisive element between two consecutive valuations: the
/// maximum of `B_prev(v) Δ B_cur(v)` over all vertices `v`, or `-1`.
///
/// Requires that both valuations have the same cycle maxima, i.e. that
/// `prev.lambda == cur.lambda`; `left` and `right` are scratch space.
fn decisive_element(prev: &Snapshot, cur: &Snapshot, left: &mut Vec<usize>, right: &mut Vec<usize>) -> i64 {
    debug_assert_eq!(
        prev.lambda, cur.lambda,
        "The decisive element requires equal cycle maxima"
    );

    let mut delta = -1i64;
    for v in 0..prev.profile.len() {
        let (dp, dc) = (prev.depth[v], cur.depth[v]);
        if dp == 0 && dc == 0 {
            // On the cycle in both: both sets are empty.
            continue;
        }
        if dp > 0 && dc > 0 && prev.profile[v] == cur.profile[v] {
            // Same move: the symmetric difference is that of the successor.
            continue;
        }
        b_set(&prev.profile, &prev.lambda, &prev.depth, v, left);
        b_set(&cur.profile, &cur.lambda, &cur.depth, v, right);
        delta = delta.max(max_symmetric_difference(left, right));
    }
    delta
}

/// Collects the [`DecisiveStatistics`] over the rounds of a run.
struct DecisiveTracker {
    statistics: DecisiveStatistics,
    /// The valuation of the previous round, if any.
    previous: Option<Snapshot>,
    /// `counts[x + 1]` is `N_x` over the whole run.
    counts: Vec<usize>,
    /// `interval_counts[x + 1]` is `N^I_x` within the current l-interval.
    interval_counts: Vec<usize>,
    /// The indices of the non-zero entries of `interval_counts`.
    touched: Vec<usize>,
    /// The number of consecutive executed rounds with the same `σ̄` so far.
    epoch_length: usize,
    /// Scratch space for [`decisive_element`].
    left: Vec<usize>,
    right: Vec<usize>,
}

impl DecisiveTracker {
    fn new(game: &RelevancyGame) -> Self {
        let n = game.num_of_vertices();
        Self {
            statistics: DecisiveStatistics {
                even_vertices: (0..n).filter(|&v| !is_odd(game, v)).count(),
                ..Default::default()
            },
            previous: None,
            counts: vec![0; n + 1],
            interval_counts: vec![0; n + 1],
            touched: Vec::new(),
            epoch_length: 0,
            left: Vec::new(),
            right: Vec::new(),
        }
    }

    /// Records a round, given the profile `ps = (σ, σ̄)` and its valuation `Ξ_σ`.
    ///
    /// Must be called for every round, including the final optimality check.
    fn record_round(&mut self, ps: &[usize], valuation: &Valuation) {
        let current = Snapshot {
            profile: ps.to_vec(),
            lambda: valuation.lambda.clone(),
            depth: valuation.depth.clone(),
        };

        if let Some(prev) = &self.previous {
            if prev.lambda != current.lambda {
                // An L-round ends the current l-interval.
                self.statistics.l_rounds += 1;
                for &i in &self.touched {
                    self.interval_counts[i] = 0;
                }
                self.touched.clear();
            } else {
                let delta = decisive_element(prev, &current, &mut self.left, &mut self.right);
                let index = (delta + 1) as usize;
                self.counts[index] += 1;
                if self.interval_counts[index] == 0 {
                    self.touched.push(index);
                }
                self.interval_counts[index] += 1;
                if self.counts[index] > self.statistics.max_count {
                    self.statistics.max_count = self.counts[index];
                    self.statistics.max_count_element = delta;
                }
                self.statistics.max_interval_count =
                    self.statistics.max_interval_count.max(self.interval_counts[index]);
            }
        }
        self.previous = Some(current);
    }

    /// Records an executed round, where `new_epoch` is true iff `σ̄` changed.
    fn record_epoch(&mut self, new_epoch: bool) {
        if new_epoch {
            self.epoch_length = 0;
        }
        self.epoch_length += 1;
        self.statistics.max_epoch_length = self.statistics.max_epoch_length.max(self.epoch_length);
    }
}

impl fmt::Display for StrategyImprovementStatistics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "iterations {}, epochs {}, best-response rounds {}, fallbacks {}, time {:.6}s",
            self.iterations,
            self.epochs,
            self.best_response_rounds,
            self.fallbacks,
            self.time.as_secs_f64()
        )?;
        if let Some(decisive) = &self.decisive {
            write!(f, ", {decisive}")?;
        }
        Ok(())
    }
}

/// The result of [`solve_strategy_improvement`].
pub struct StrategyImprovementSolution {
    /// The winning sets of both players.
    pub solution: [Set; 2],
    /// The final strategies: Even's optimal strategy and Odd's best response to
    /// it, defined on all vertices of the respective player.
    pub strategy: Option<[Strategy; 2]>,
    /// Statistics of the run.
    pub statistics: StrategyImprovementStatistics,
}

/// Solves the given parity game by strategy improvement, starting from the
/// initial profile `p_0`, the first successor of every vertex in relevance
/// order, unless `options.initial_profile` gives another one.
///
/// Requires that the game is total. Returns the winning regions, which
/// partition the vertices, and if `compute_strategy` is true Even's optimal
/// strategy together with Odd's best response to it, which are winning on the
/// respective regions.
///
/// # Errors
///
/// Fails if `options.initial_profile` is not a profile of the game, or if more
/// than `options.max_iterations` iterations are needed.
pub fn solve_strategy_improvement<G: PG>(
    game: &G,
    options: &StrategyImprovementOptions,
    compute_strategy: bool,
) -> Result<StrategyImprovementSolution, MercError> {
    debug_assert!(game.is_total(), "Strategy improvement requires a total parity game");

    let relabelled = RelevancyGame::new(game);
    let initial = match &options.initial_profile {
        Some(profile) => relabel_profile(&relabelled, profile)?,
        None => initial_profile(&relabelled),
    };

    let mut solver = StrategyImprovement::new(&relabelled);
    let start = Instant::now();
    let profile = solver.run(options, initial)?;
    solver.statistics.time = start.elapsed();
    debug!("Strategy improvement: {}", solver.statistics);

    // The optimal valuation determines the winners: a vertex is won by Even iff
    // the cycle maximum of its play has an even priority.
    let n = relabelled.num_of_vertices();
    let mut w0 = bitvec![usize, Lsb0; 0; n];
    let mut w1 = bitvec![usize, Lsb0; 0; n];
    for v in 0..n {
        let old = *relabelled.to_original(VertexIndex::new(v));
        if is_even(&relabelled, solver.valuation.lambda[v]) {
            w0.set(old, true);
        } else {
            w1.set(old, true);
        }
    }

    let strategy = compute_strategy.then(|| {
        let mut s0 = Strategy::new();
        let mut s1 = Strategy::new();
        for v in 0..n {
            let from = relabelled.to_original(VertexIndex::new(v));
            let to = relabelled.to_original(VertexIndex::new(profile[v]));
            if is_odd(&relabelled, v) {
                s1.set(from, to);
            } else {
                s0.set(from, to);
            }
        }
        [s0, s1]
    });

    Ok(StrategyImprovementSolution {
        solution: [w0, w1],
        strategy,
        statistics: solver.statistics.clone(),
    })
}

/// Returns the initial profile `p_0`: the first successor of every vertex in
/// relevance order, i.e. `p_0(v) = min succ(v)` in the relevancy game.
///
/// This is `initProfile` of the formalisation, from which its correctness,
/// termination and (open) polynomial iteration statements start.
fn initial_profile(game: &RelevancyGame) -> Vec<usize> {
    (0..game.num_of_vertices())
        .map(|v| {
            successors(game, v)
                .next()
                .expect("Strategy improvement requires a total parity game")
        })
        .collect()
}

/// Returns the given profile, indexed by and pointing to original vertices, in
/// the vertices of the relevancy game.
///
/// # Errors
///
/// Fails if `profile` does not have a move for every vertex, or if a move is
/// not an edge.
fn relabel_profile(game: &RelevancyGame, profile: &[VertexIndex]) -> Result<Vec<usize>, MercError> {
    let n = game.num_of_vertices();
    if profile.len() != n {
        return Err(format!("Initial profile has {} moves, expected {n}", profile.len()).into());
    }
    (0..n)
        .map(|v| {
            let old = *game.to_original(VertexIndex::new(v));
            let w = *game.to_relevancy(profile[old]);
            if successors(game, v).any(|x| x == w) {
                Ok(w)
            } else {
                Err(format!("Initial move {old} -> {} is not an edge", *profile[old]).into())
            }
        })
        .collect()
}

/// Solves the given parity game by strategy improvement with default options,
/// returning the solution in the same form as [`crate::solve_zielonka`].
pub fn solve_strategy_improvement_default<G: PG>(
    game: &G,
    algorithm: StrategyImprovementAlgorithm,
    compute_strategy: bool,
) -> ([Set; 2], Option<[Strategy; 2]>) {
    let result = solve_strategy_improvement(game, &StrategyImprovementOptions::new(algorithm), compute_strategy)
        .expect("Strategy improvement without an iteration bound cannot fail");
    (result.solution, result.strategy)
}

/// Returns true iff vertex `v` is owned by Odd.
fn is_odd(game: &RelevancyGame, v: usize) -> bool {
    game.owner(VertexIndex::new(v)) == Player::Odd
}

/// Returns true iff vertex `v` has an even priority.
fn is_even(game: &RelevancyGame, v: usize) -> bool {
    game.priority(VertexIndex::new(v)).value().is_multiple_of(2)
}

/// Returns the successors of vertex `v`, in relevance order.
fn successors(game: &RelevancyGame, v: usize) -> impl Iterator<Item = usize> + '_ {
    game.outgoing_edges(VertexIndex::new(v)).map(|edge| *edge.to())
}

/// The play values `Ξ_f` of a profile `f` (Definition 5), with the order of
/// Definition 6.
///
/// # Computation
///
/// Under `f` every vertex has one successor, so the edges `x → f(x)` form a
/// functional graph: every component is one cycle with trees hanging into it.
/// [`Valuation::compute`] follows the play from every unvisited vertex until it
/// reaches a vertex that is either evaluated or on the current path. In the
/// latter case it has closed a new cycle, whose maximum is `λ`; the vertices of
/// the path are then evaluated backwards from the vertex they run into. This
/// visits every vertex once, `O(n)`.
///
/// Comparing `B` (Definition 6, case 2) needs `max(B(a) Δ B(b))`. Since `B(v)`
/// is the set of vertices greater than `λ` on the tree path from `v` to the
/// root (see the invariants), and `B(a)` and `B(b)` agree above the lowest
/// common ancestor of `a` and `b`, this is the maximum of `B` on the two paths
/// below that ancestor. Binary lifting tables (`up` and `best`) answer this in
/// `O(log h)`, for `h` the maximal depth, after `O(n log h)` preprocessing.
///
/// # Invariants
///
/// After [`Valuation::compute`] on `f`, for every vertex `v`:
///
/// - `lambda[v]` is `λ(v)`, the maximum of the cycle that the play from `v`
///   ends in;
/// - `steps[v]` is `k(v)`, the number of steps from `v` to the first visit of
///   `λ(v)`;
/// - `depth[v]` is the number of steps from `v` to the cycle, zero on the
///   cycle.
///
/// The vertices with cycle maximum `λ` form a tree rooted at `λ`, where all
/// vertices of the cycle are identified with `λ` (see [`Valuation::node`]) and
/// the parent of a tail vertex `x` is `f(x)`. `B(v)` consists of the vertices
/// greater than `λ(v)` on the tree path from `v` to the root, excluding it.
#[derive(Default)]
struct Valuation {
    /// The cycle maximum `λ` of every vertex.
    lambda: Vec<usize>,
    /// The number of steps `k` to the cycle maximum.
    steps: Vec<usize>,
    /// The depth in the tree of `λ`; zero on the cycle.
    depth: Vec<usize>,
    /// `up[j * n + x]`: the ancestor `2^j` steps above `x` (the root is its own parent).
    up: Vec<usize>,
    /// `best[j * n + x]`: one plus the maximum vertex of `B` among the `2^j`
    /// vertices from `x` upwards, or zero if there is none.
    best: Vec<usize>,
    /// The number of levels of `up` and `best`.
    levels: usize,
    /// Scratch space: the visit state of every vertex, see [`Visit`], and the current path.
    state: Vec<u8>,
    path: Vec<usize>,
}

/// The visit states of [`Valuation::state`].
struct Visit;

impl Visit {
    const NEW: u8 = 0;
    const ON_PATH: u8 = 1;
    const DONE: u8 = 2;
}

impl Valuation {
    /// Computes the play values of the profile `f`, in `O(n)` for the plays
    /// and `O(n log h)` for the lifting tables.
    ///
    /// Requires that `f[v]` is a successor of `v` for every vertex `v`;
    /// afterwards the [invariants](Valuation#invariants) hold for `f`.
    fn compute(&mut self, f: &[usize]) {
        let n = f.len();
        self.lambda.resize(n, 0);
        self.steps.resize(n, 0);
        self.depth.resize(n, 0);
        self.state.clear();
        self.state.resize(n, Visit::NEW);
        self.up.resize(n, 0);
        self.best.resize(n, 0);

        let mut max_depth = 0;
        for s in 0..n {
            if self.state[s] == Visit::NEW {
                max_depth = max_depth.max(self.evaluate_play(f, s));
            }
        }
        self.build_lifting_tables(n, max_depth);
    }

    /// Evaluates the play from `s` up to the first evaluated vertex, and returns
    /// the maximum depth of the newly evaluated vertices.
    ///
    /// Requires that `s` is new and that every vertex is either new or
    /// evaluated. Afterwards all vertices of the play are evaluated and the
    /// first level of the lifting tables is set for them.
    fn evaluate_play(&mut self, f: &[usize], s: usize) -> usize {
        // Follow the play from `s` until a vertex on the path or an evaluated vertex is reached.
        self.path.clear();
        let mut u = s;
        while self.state[u] == Visit::NEW {
            self.state[u] = Visit::ON_PATH;
            self.path.push(u);
            u = f[u];
        }

        if self.state[u] == Visit::ON_PATH {
            let i = self
                .path
                .iter()
                .position(|&x| x == u)
                .expect("u is on the current path");
            self.evaluate_cycle(i);
        }

        // The remaining path is a tail into an evaluated vertex, evaluated backwards.
        let mut max_depth = 0;
        while let Some(x) = self.path.pop() {
            let y = f[x];
            let lambda = self.lambda[y];
            self.lambda[x] = lambda;
            self.steps[x] = self.steps[y] + 1;
            self.depth[x] = self.depth[y] + 1;
            max_depth = max_depth.max(self.depth[x]);
            self.up[x] = self.node(y);
            self.best[x] = if x > lambda { x + 1 } else { 0 };
            self.state[x] = Visit::DONE;
        }
        max_depth
    }

    /// Evaluates the cycle `path[i..]` that was just closed, and removes it from
    /// the path.
    ///
    /// Requires that `path[i..]` is a cycle of the profile in play order.
    fn evaluate_cycle(&mut self, i: usize) {
        let cycle = &self.path[i..];
        let (j, &lambda) = cycle
            .iter()
            .enumerate()
            .max_by_key(|(_, x)| **x)
            .expect("cycles are non-empty");
        let m = cycle.len();
        for (t, &x) in cycle.iter().enumerate() {
            self.lambda[x] = lambda;
            // The number of steps from position `t` forward to position `j`.
            self.steps[x] = (j + m - t) % m;
            self.depth[x] = 0;
            self.up[x] = lambda;
            self.best[x] = 0;
            self.state[x] = Visit::DONE;
        }
        self.path.truncate(i);
    }

    /// Builds the binary lifting tables `up` and `best` from their first level.
    ///
    /// Requires that the first level is set for all `n` vertices, and that no
    /// vertex is deeper than `max_depth`.
    fn build_lifting_tables(&mut self, n: usize, max_depth: usize) {
        self.levels = (usize::BITS - max_depth.leading_zeros()).max(1) as usize;
        self.up.resize(self.levels * n, 0);
        self.best.resize(self.levels * n, 0);
        for j in 1..self.levels {
            // Level `j` takes two steps of level `j - 1`.
            let (lower, upper) = self.up.split_at_mut(j * n);
            let (best_lower, best_upper) = self.best.split_at_mut(j * n);
            let prev = &lower[(j - 1) * n..];
            let prev_best = &best_lower[(j - 1) * n..];
            for x in 0..n {
                let mid = prev[x];
                upper[x] = prev[mid];
                best_upper[x] = prev_best[x].max(prev_best[mid]);
            }
        }
    }

    /// Returns the node representing `v` in the tree of its cycle maximum.
    fn node(&self, v: usize) -> usize {
        if self.depth[v] == 0 { self.lambda[v] } else { v }
    }

    /// Lifts node `x` by `d` levels, returning the new node and the maximum of
    /// `best` and the `best` entries passed.
    ///
    /// Requires that `x` has at least `d` ancestors.
    fn lift(&self, mut x: usize, mut d: usize, mut best: usize) -> (usize, usize) {
        let n = self.lambda.len();
        let mut j = 0;
        while d > 0 {
            if d & 1 == 1 {
                best = best.max(self.best[j * n + x]);
                x = self.up[j * n + x];
            }
            d >>= 1;
            j += 1;
        }
        (x, best)
    }

    /// Returns one plus the maximum of `B` on the tree paths from `a` and from
    /// `b` up to their lowest common ancestor (excluding it), or zero if there
    /// is none, for both paths.
    ///
    /// Requires that `a` and `b` are nodes in the same tree, i.e. have the same
    /// cycle maximum. Since `B(a)` and `B(b)` agree above the lowest common
    /// ancestor, the maximum of these two is the maximum of `B(a) Δ B(b)`.
    fn max_below_common_ancestor(&self, mut a: usize, mut b: usize) -> (usize, usize) {
        let n = self.lambda.len();
        let (da, db) = (self.depth[a], self.depth[b]);
        let (mut ma, mut mb) = (0, 0);
        if da > db {
            (a, ma) = self.lift(a, da - db, ma);
        } else if db > da {
            (b, mb) = self.lift(b, db - da, mb);
        }

        if a != b {
            // Lift both as far as possible while staying below the common ancestor.
            for j in (0..self.levels).rev() {
                let (ua, ub) = (self.up[j * n + a], self.up[j * n + b]);
                if ua != ub {
                    ma = ma.max(self.best[j * n + a]);
                    mb = mb.max(self.best[j * n + b]);
                    a = ua;
                    b = ub;
                }
            }
            // Now `a` and `b` are children of the common ancestor.
            ma = ma.max(self.best[a]);
            mb = mb.max(self.best[b]);
        }
        (ma, mb)
    }

    /// Compares the sets `B` of two vertices with the same cycle maximum: the
    /// set containing the maximum of their symmetric difference is better for
    /// Even iff that maximum has an even priority.
    ///
    /// Requires that `lambda[a] == lambda[b]`.
    fn compare_b(&self, game: &RelevancyGame, a: usize, b: usize) -> Ordering {
        let (a, b) = (self.node(a), self.node(b));
        if a == b {
            return Ordering::Equal;
        }

        let (ma, mb) = self.max_below_common_ancestor(a, b);
        match ma.cmp(&mb) {
            Ordering::Equal => Ordering::Equal,
            Ordering::Greater => {
                if is_even(game, ma - 1) {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            Ordering::Less => {
                if is_even(game, mb - 1) {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
        }
    }

    /// Returns the reward of the cycle maximum `λ` of `v`: `λ + 1` if it has an
    /// even priority and `-(λ + 1)` otherwise.
    fn reward(&self, game: &RelevancyGame, v: usize) -> i64 {
        let lambda = self.lambda[v];
        if is_even(game, lambda) {
            lambda as i64 + 1
        } else {
            -(lambda as i64 + 1)
        }
    }

    /// Compares the play values `(λ, B, k)` of `a` and `b` by Definition 6,
    /// where greater is better for Even, in `O(1)` if the cycle maxima differ
    /// and `O(log h)` otherwise.
    ///
    /// This is a total preorder on the vertices, and `Equal` iff the play values
    /// are equal.
    fn compare(&self, game: &RelevancyGame, a: usize, b: usize) -> Ordering {
        if a == b {
            return Ordering::Equal;
        }
        if self.lambda[a] != self.lambda[b] {
            return self.reward(game, a).cmp(&self.reward(game, b));
        }
        self.compare_b(game, a, b).then_with(|| {
            // A shorter path to an even cycle maximum is better.
            if is_even(game, self.lambda[a]) {
                self.steps[b].cmp(&self.steps[a])
            } else {
                self.steps[a].cmp(&self.steps[b])
            }
        })
    }

    /// Returns the best successor of `v` for Even (`maximise`) or Odd, keeping
    /// `current` unless some successor is strictly better.
    ///
    /// Of several best successors the first in relevance order is taken.
    fn best_of(&self, game: &RelevancyGame, v: usize, current: usize, maximise: bool) -> usize {
        let wanted = if maximise { Ordering::Greater } else { Ordering::Less };
        let mut best = current;
        for x in successors(game, v) {
            if self.compare(game, x, best) == wanted {
                best = x;
            }
        }
        best
    }

    /// Returns the first successor of `v` (in relevance order) with the maximal
    /// play value.
    ///
    /// Requires that `v` has a successor.
    fn first_max_successor(&self, game: &RelevancyGame, v: usize) -> usize {
        let mut candidates = successors(game, v);
        let mut max = candidates
            .next()
            .expect("Strategy improvement requires a total parity game");
        for x in candidates {
            if self.compare(game, x, max) == Ordering::Greater {
                max = x;
            }
        }
        max
    }
}

/// Computes the `Ξ_σ`-best improving move of every Even vertex, and returns
/// true iff some vertex is improvable, i.e. iff `σ` is not optimal.
///
/// This is not a best response: `Ξ_σ` already accounts for Odd's best
/// response `σ̄`, and this function only reads off the improving edges
/// `I_σ(v)` of Definition 8, keeping the first `Ξ_σ`-maximum of each, which
/// is VJ's switch-all choice (Definition 10 with `S = I_σ`). It costs one
/// comparison per Even edge.
///
/// Requires that `valuation` is `Ξ_σ`, the valuation of Even's moves in `p`
/// against Odd's best response. Afterwards `improving[v]` is the first
/// successor `w` of Even vertex `v` that maximises `Ξ_σ(w)` if
/// `Ξ_σ(w) > Ξ_σ(σ(v))`, and `None` for Odd and unimprovable vertices.
fn improving_moves(game: &RelevancyGame, valuation: &Valuation, p: &[usize], improving: &mut [Option<usize>]) -> bool {
    let mut improvable = false;
    for v in 0..p.len() {
        improving[v] = None;
        if !is_odd(game, v) {
            let b = valuation.best_of(game, v, p[v], true);
            if b != p[v] {
                improving[v] = Some(b);
                improvable = true;
            }
        }
    }
    improvable
}

/// Restricts the improving moves to those that are optimal against `σ̄`, and
/// returns true iff some vertex keeps a move.
///
/// This computes `F_σ` of Definition 9. An edge `w` of `v` is optimal against
/// `σ̄` if `K*(w)` is maximal among the successors of `v`, where `K*` is the
/// valuation of Even's best response `σ*` to `σ̄`. Of the improving edges
/// (`Ξ_σ(w) > Ξ_σ(σ(v))`) that are optimal in this sense the first
/// `Ξ_σ`-maximum is kept, which is the H-B choice of Definition 10. It costs
/// three comparisons per edge of an improvable vertex.
///
/// Requires that `valuation` is `Ξ_σ`, that `response` is the valuation of
/// `σ*`, Even's best response to `σ̄`, and that `improving` is the result of
/// [`improving_moves`]. Afterwards `filtered[v]` is the `Ξ_σ`-best edge (the
/// first one in relevance order) in
/// `F(v) = { w ∈ I(v) | Ξ_{σ*}(w) = max_x Ξ_{σ*}(x) }`, or `None` if `F(v)`
/// is empty.
fn optimal_moves(
    game: &RelevancyGame,
    valuation: &Valuation,
    response: &Valuation,
    p: &[usize],
    improving: &[Option<usize>],
    filtered: &mut [Option<usize>],
) -> bool {
    let mut any = false;
    for v in 0..p.len() {
        filtered[v] = None;
        if improving[v].is_none() {
            continue;
        }

        let max = response.first_max_successor(game, v);
        let mut best: Option<usize> = None;
        for w in successors(game, v) {
            if valuation.compare(game, w, p[v]) == Ordering::Greater
                && response.compare(game, w, max) == Ordering::Equal
                && best.is_none_or(|best| valuation.compare(game, w, best) == Ordering::Greater)
            {
                best = Some(w);
            }
        }
        filtered[v] = best;
        any |= best.is_some();
    }
    any
}

/// Returns true iff the profiles `a` and `b` differ at some Odd vertex.
fn odd_moves_differ(game: &RelevancyGame, a: &[usize], b: &[usize]) -> bool {
    (0..a.len()).any(|v| is_odd(game, v) && a[v] != b[v])
}

/// The state of a strategy improvement run on a relevancy game.
struct StrategyImprovement<'a> {
    game: &'a RelevancyGame,
    /// The valuation `Ξ_σ` of Even's current strategy.
    valuation: Valuation,
    /// The valuation of Even's best response `σ*` to `σ̄`.
    response: Valuation,
    statistics: StrategyImprovementStatistics,
}

impl<'a> StrategyImprovement<'a> {
    fn new(game: &'a RelevancyGame) -> Self {
        Self {
            game,
            valuation: Valuation::default(),
            response: Valuation::default(),
            statistics: StrategyImprovementStatistics::default(),
        }
    }

    /// Replaces the moves of player `odd` in `p` by a best response against the
    /// other player's moves, by switch-all one-player strategy improvement
    /// warm started from `p`, and counts the valuations in `rounds`.
    ///
    /// With the other player's moves fixed, the game is a one-player game, and
    /// a best response (Definition 7) is a strategy under which no vertex of
    /// the player has a strictly better successor under the play values of the
    /// profile itself (Vöge and Jurdziński). Every round computes the valuation `Ξ_p` and
    /// switches every vertex of the player with a strictly better successor
    /// to its first best one. For `odd` this yields `σ̄` and `Ξ_σ`, otherwise
    /// `σ*` and `K*`. A round costs `O((n + m) log h)`, see [`Valuation`].
    ///
    /// Requires that `p` is a profile. Afterwards the other player's moves are
    /// unchanged, `valuation` holds the play values of `p`, and no vertex of
    /// player `odd` has a strictly better successor under it.
    fn best_response(game: &RelevancyGame, valuation: &mut Valuation, rounds: &mut usize, p: &mut [usize], odd: bool) {
        loop {
            valuation.compute(p);
            *rounds += 1;
            let mut changed = false;
            for v in 0..p.len() {
                if is_odd(game, v) == odd {
                    let b = valuation.best_of(game, v, p[v], !odd);
                    if b != p[v] {
                        p[v] = b;
                        changed = true;
                    }
                }
            }
            if !changed {
                return;
            }
        }
    }

    /// Runs the algorithm from the profile `p`, and returns the final profile:
    /// Even's optimal strategy and Odd's best response to it.
    ///
    /// Requires that `p` is a profile. Afterwards `valuation` holds the optimal
    /// valuation, and `statistics` the counts of the run.
    ///
    /// # Details
    ///
    /// One iteration is, in terms of the definitions:
    ///
    /// 1. compute `σ̄` and `Ξ_σ` by [`Self::best_response`] for Odd
    ///    (Definition 7);
    /// 2. compute `I_σ` by [`improving_moves`] (Definition 8), and stop if `σ`
    ///    is optimal;
    /// 3. for H-B, compute `σ*` and `K*` by [`Self::best_response`] for Even,
    ///    and `F_σ` by [`optimal_moves`] (Definition 9);
    /// 4. switch according to `S = F_σ`, or `S = I_σ` if all of `F_σ` is empty
    ///    or for VJ (Definition 10).
    ///
    /// Every iteration switches Even's moves to improving moves only, so `Ξ_σ`
    /// strictly increases and the run terminates. Odd's moves in `p` stay the
    /// initial ones: its best response is computed from them in every
    /// iteration.
    ///
    /// # Errors
    ///
    /// Fails if more than `options.max_iterations` iterations are needed.
    fn run(&mut self, options: &StrategyImprovementOptions, mut p: Vec<usize>) -> Result<Vec<usize>, MercError> {
        let game = self.game;
        let n = p.len();
        let mut previous_response: Option<Vec<usize>> = None;
        let mut improving = vec![None; n];
        let mut filtered = vec![None; n];
        let mut decisive = options.decisive.then(|| DecisiveTracker::new(game));

        loop {
            // σ̄: Odd's best response to σ, with valuation Ξ_σ.
            let mut ps = p.clone();
            Self::best_response(
                game,
                &mut self.valuation,
                &mut self.statistics.best_response_rounds,
                &mut ps,
                true,
            );

            let improvable = improving_moves(game, &self.valuation, &p, &mut improving);
            if let Some(tracker) = decisive.as_mut() {
                tracker.record_round(&ps, &self.valuation);
            }
            if !improvable {
                // σ is optimal, and Ξ_σ is the optimal valuation.
                self.statistics.decisive = decisive.map(|tracker| tracker.statistics);
                return Ok(ps);
            }

            let new_epoch = previous_response
                .as_ref()
                .is_none_or(|prev| odd_moves_differ(game, prev, &ps));
            if new_epoch {
                self.statistics.epochs += 1;
            }
            if let Some(tracker) = decisive.as_mut() {
                tracker.record_epoch(new_epoch);
            }

            let switch = match options.algorithm {
                StrategyImprovementAlgorithm::VogeJurdzinski => &improving,
                StrategyImprovementAlgorithm::HybridB => {
                    // σ*: Even's best response to σ̄.
                    let mut pstar = ps.clone();
                    Self::best_response(
                        game,
                        &mut self.response,
                        &mut self.statistics.best_response_rounds,
                        &mut pstar,
                        false,
                    );

                    if optimal_moves(game, &self.valuation, &self.response, &p, &improving, &mut filtered) {
                        &filtered
                    } else {
                        self.statistics.fallbacks += 1;
                        &improving
                    }
                }
            };
            for (v, w) in switch.iter().enumerate() {
                if let Some(w) = w {
                    p[v] = *w;
                }
            }

            trace!("Strategy improvement iteration {}", self.statistics.iterations);
            previous_response = Some(ps);
            self.statistics.iterations += 1;
            if let Some(max) = options.max_iterations
                && self.statistics.iterations > max
            {
                return Err(format!("Strategy improvement exceeded {max} iterations").into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use merc_io::DumpFiles;
    use merc_utilities::random_test;

    use crate::random_parity_game;
    use crate::read_pg;
    use crate::solve_zielonka;
    use crate::verify_solution;
    use crate::write_pg;

    use super::*;

    #[test]
    #[cfg_attr(miri, ignore)] // Miri is too slow for this test.
    fn test_random_strategy_improvement_solver() {
        random_test(100, |rng| {
            let files = DumpFiles::new("test_random_strategy_improvement_solver");
            let game = random_parity_game(rng, true, 60, 8, 4);
            files.dump("input.pg", |writer| write_pg(writer, &game)).unwrap();

            let (expected, _) = solve_zielonka(&game, false);
            for algorithm in [
                StrategyImprovementAlgorithm::HybridB,
                StrategyImprovementAlgorithm::VogeJurdzinski,
            ] {
                let options = StrategyImprovementOptions::new(algorithm);
                let result = solve_strategy_improvement(&game, &options, true).unwrap();
                assert_eq!(result.solution, expected, "{algorithm:?} disagrees with Zielonka");
                verify_solution(&game, &result.solution, &result.strategy.unwrap());
            }
        });
    }

    /// Compares the B-set order against the exact sum `Σ_{x ∈ B} sgn(x) 2^x` on small games.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_valuation_order() {
        random_test(200, |rng| {
            let game = random_parity_game(rng, true, 40, 40, 3);
            let relabelled = RelevancyGame::new(&game);
            let n = relabelled.num_of_vertices();
            let profile: Vec<usize> = (0..n)
                .map(|v| {
                    let s: Vec<usize> = successors(&relabelled, v).collect();
                    s[v % s.len()]
                })
                .collect();

            let mut valuation = Valuation::default();
            valuation.compute(&profile);

            // Reference: follow the play explicitly.
            let key = |v: usize| -> (i64, i128, i64) {
                let lambda = valuation.lambda[v];
                let mut sum: i128 = 0;
                let mut x = v;
                let mut k = 0;
                while x != lambda {
                    if x > lambda && valuation.depth[x] > 0 {
                        sum += if is_even(&relabelled, x) {
                            1i128 << x
                        } else {
                            -(1i128 << x)
                        };
                    }
                    x = profile[x];
                    k += 1;
                }
                assert_eq!(k, valuation.steps[v] as i64);
                let even = is_even(&relabelled, lambda);
                let reward = if even { lambda as i64 + 1 } else { -(lambda as i64 + 1) };
                (reward, sum, if even { -k } else { k })
            };

            for a in 0..n {
                for b in 0..n {
                    assert_eq!(
                        valuation.compare(&relabelled, a, b),
                        key(a).cmp(&key(b)),
                        "Comparison of {a} and {b} is incorrect"
                    );
                }
            }
        });
    }

    #[test]
    fn test_relevancy_game_valuation() {
        // A two vertex game: Even at 0 (priority 0) with a choice, Odd at 1 (priority 1).
        let game = RelevancyGame::new(&read_pg("parity 2;\n0 0 0 0,1;\n1 1 1 1;\n".as_bytes()).unwrap());
        let mut valuation = Valuation::default();
        valuation.compute(&[1, 1]);
        assert_eq!(valuation.lambda, vec![1, 1]);
        valuation.compute(&[0, 1]);
        assert_eq!(valuation.compare(&game, 0, 1), Ordering::Greater);
    }

    /// The G1 counterexample of `experiments/hb_epochs_count` (n = 14): N_13 = 16 = n + NL, with
    /// (iterations, NL, max N_x, max N^I_x, max epoch length) from `hb_g1/scripts/g1.py`.
    #[test]
    fn test_decisive_statistics_g1_counterexample() {
        let game = read_pg("parity 14;\n0 5 1 7;\n1 16 0 8,4,7,12,5;\n2 30 0 10,6,0;\n3 21 0 4,13,6,0,7,5,9,3;\n4 4 0 1,3,0,10,12,6;\n5 24 0 3,7,5,9;\n6 28 1 2,7,6,9;\n7 15 0 13,6;\n8 4 0 11,6,0,3,5,9,7,10;\n9 11 0 12,6,10,0,7;\n10 22 1 3,10,5,13,4,6,1,2;\n11 16 0 2,11,12,8,6,7,4,9,5;\n12 14 0 5,6,13,7,9,0;\n13 1 0 9,10,6;\n".as_bytes()).unwrap();
        // The counterexample starts from its own (adversarial) profile.
        let mut profile: Vec<VertexIndex> = game.iter_vertices().collect();
        for line in "0 7\n1 7\n2 0\n3 13\n4 10\n5 9\n6 6\n7 13\n8 0\n9 0\n10 10\n11 7\n12 13\n13 10\n".lines() {
            let parts: Vec<usize> = line.split_whitespace().map(|x| x.parse().unwrap()).collect();
            profile[parts[0]] = VertexIndex::new(parts[1]);
        }

        let mut options = StrategyImprovementOptions::new(StrategyImprovementAlgorithm::HybridB);
        options.initial_profile = Some(profile);
        options.decisive = true;
        let result = solve_strategy_improvement(&game, &options, false).unwrap();
        let decisive = result.statistics.decisive.unwrap();
        assert_eq!(
            (
                result.statistics.iterations,
                decisive.l_rounds,
                decisive.max_count,
                decisive.max_count_element,
                decisive.max_interval_count,
                decisive.max_epoch_length
            ),
            (18, 2, 16, 13, 9, 7)
        );
    }
}
