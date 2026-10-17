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
//! # Details
//!
//! The vertices are first relabelled by (priority, original index), which
//! gives the *relevance order*. The play from every vertex under a profile is
//! a lasso, and its play value is `(λ, B, k)`: `λ` the most relevant vertex of
//! the cycle, `B` the vertices more relevant than `λ` visited before reaching
//! `λ`, and `k` the number of steps to `λ`. Play values are compared exactly:
//! first by the reward of `λ`, then the sets `B` by the most relevant vertex
//! of their symmetric difference (good for the set containing it iff it has an
//! even priority), and finally by `k` (shorter is better iff `λ` is even).
//!
//! The vertices with the same `λ` form a tree rooted at the cycle of `λ`, in
//! which `B(v)` is the set of vertices more relevant than `λ` on the path from
//! `v` to the root. The most relevant vertex of a symmetric difference is the
//! maximum on the two paths up to their lowest common ancestor, which is
//! computed with binary lifting in `O(log n)` per comparison and `O(n log n)`
//! preprocessing per valuation.
//!
//! Best responses are computed by one-player switch-all strategy improvement,
//! warm started from the given profile, where the current move is kept unless
//! a successor is strictly better, and the first best successor (in relevance
//! order) is taken. This mirrors the Python reference implementation
//! `experiments/hybrid/scripts/algs.py` exactly, so that iteration counts can
//! be compared.

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

/// The profile from which Odd's best response is computed in every iteration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum OddWarmStart {
    /// Odd's part of the initial profile (as `algs.py`).
    #[default]
    Initial,
    /// Odd's best response of the previous iteration (as `hb_bound/scripts/hb.py`).
    Previous,
}

/// Options for [`solve_strategy_improvement`].
#[derive(Debug, Clone)]
pub struct StrategyImprovementOptions {
    /// The algorithm to run.
    pub algorithm: StrategyImprovementAlgorithm,
    /// The initial move of every vertex (indexed by original vertex), by
    /// default the first outgoing edge of every vertex.
    pub initial_profile: Option<Vec<VertexIndex>>,
    /// The warm start of Odd's best responses.
    pub odd_warm_start: OddWarmStart,
    /// Stop with an error after this many iterations.
    pub max_iterations: Option<usize>,
    /// Compute the decisive-element statistics ([`DecisiveStatistics`]).
    pub decisive: bool,
}

impl fmt::Display for OddWarmStart {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Initial => write!(f, "initial"),
            Self::Previous => write!(f, "previous"),
        }
    }
}

impl StrategyImprovementOptions {
    /// Default options for the given algorithm.
    pub fn new(algorithm: StrategyImprovementAlgorithm) -> Self {
        Self {
            algorithm,
            initial_profile: None,
            odd_warm_start: OddWarmStart::Initial,
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
    profile: Vec<u32>,
    lambda: Vec<u32>,
    depth: Vec<u32>,
}

/// The vertices of `B(x)` under the profile `f`, in decreasing order.
fn b_set(f: &[u32], lambda: &[u32], depth: &[u32], x: usize, out: &mut Vec<u32>) {
    out.clear();
    let mut y = x;
    while depth[y] > 0 {
        if y as u32 > lambda[y] {
            out.push(y as u32);
        }
        y = f[y] as usize;
    }
    out.sort_unstable_by(|a, b| b.cmp(a));
}

/// The decisive element between two valuations with the same cycle maxima:
/// the maximum of all `B_prev(v) Δ B_cur(v)`, or `-1`.
fn decisive_element(prev: &Snapshot, cur: &Snapshot, left: &mut Vec<u32>, right: &mut Vec<u32>) -> i64 {
    let mut delta = -1i64;
    for v in 0..prev.profile.len() {
        let (dp, dc) = (prev.depth[v], cur.depth[v]);
        if dp == 0 && dc == 0 {
            continue;
        }
        if dp > 0 && dc > 0 && prev.profile[v] == cur.profile[v] {
            // Same move: the symmetric difference is that of the successor.
            continue;
        }
        b_set(&prev.profile, &prev.lambda, &prev.depth, v, left);
        b_set(&cur.profile, &cur.lambda, &cur.depth, v, right);
        let mut i = 0;
        let d = loop {
            match (left.get(i), right.get(i)) {
                (None, None) => break -1,
                (Some(a), None) => break *a as i64,
                (None, Some(b)) => break *b as i64,
                (Some(a), Some(b)) if a != b => break (*a).max(*b) as i64,
                _ => i += 1,
            }
        };
        delta = delta.max(d);
    }
    delta
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

/// Solves the given (total) parity game by strategy improvement.
pub fn solve_strategy_improvement<G: PG>(
    game: &G,
    options: &StrategyImprovementOptions,
    compute_strategy: bool,
) -> Result<StrategyImprovementSolution, MercError> {
    debug_assert!(game.is_total(), "Strategy improvement requires a total parity game");

    let relabelled = RelabelledGame::new(game);
    let n = relabelled.num_of_vertices();

    let initial: Vec<u32> = match &options.initial_profile {
        Some(profile) => {
            if profile.len() != n {
                return Err(format!("Initial profile has {} moves, expected {n}", profile.len()).into());
            }
            (0..n)
                .map(|v| {
                    let old = relabelled.perm[v];
                    let w = relabelled.inv[*profile[old]];
                    if relabelled.successors(v).contains(&w) {
                        Ok(w)
                    } else {
                        Err(format!("Initial move {old} -> {} is not an edge", *profile[old]))
                    }
                })
                .collect::<Result<_, _>>()?
        }
        None => (0..n)
            .map(|v| {
                let first = game
                    .outgoing_edges(VertexIndex::new(relabelled.perm[v]))
                    .next()
                    .expect("Strategy improvement requires a total parity game");
                relabelled.inv[*first.to()]
            })
            .collect(),
    };

    let mut solver = StrategyImprovement::new(&relabelled);
    let start = Instant::now();
    let profile = solver.run(options, initial)?;
    solver.statistics.time = start.elapsed();
    debug!("Strategy improvement: {}", solver.statistics);

    // Read off the winners, and the strategies, from the valuation of the final profile.
    let mut w0 = bitvec![usize, Lsb0; 0; n];
    let mut w1 = bitvec![usize, Lsb0; 0; n];
    for v in 0..n {
        let old = relabelled.perm[v];
        if relabelled.even[solver.valuation.lambda[v] as usize] {
            w0.set(old, true);
        } else {
            w1.set(old, true);
        }
    }

    let strategy = compute_strategy.then(|| {
        let mut s0 = Strategy::new();
        let mut s1 = Strategy::new();
        for v in 0..n {
            let from = VertexIndex::new(relabelled.perm[v]);
            let to = VertexIndex::new(relabelled.perm[profile[v] as usize]);
            if relabelled.odd[v] {
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

/// A parity game whose vertices are numbered in relevance order.
pub(crate) struct RelabelledGame {
    /// The owner of every vertex, true for Odd.
    odd: Vec<bool>,
    /// Whether the priority of every vertex is even.
    even: Vec<bool>,
    /// Offsets into `succ`, plus a sentinel.
    offsets: Vec<usize>,
    /// The successors, sorted and without duplicates.
    succ: Vec<u32>,
    /// The original index of every vertex.
    perm: Vec<usize>,
    /// The new index of every original vertex.
    inv: Vec<u32>,
}

impl RelabelledGame {
    /// Relabels the vertices of the game by (priority, index).
    pub(crate) fn new<G: PG>(game: &G) -> Self {
        let n = game.num_of_vertices();
        assert!(n < u32::MAX as usize, "Too many vertices for strategy improvement");

        let mut perm: Vec<usize> = (0..n).collect();
        perm.sort_by_key(|&v| (game.priority(VertexIndex::new(v)).value(), v));

        let mut inv = vec![0u32; n];
        for (new, &old) in perm.iter().enumerate() {
            inv[old] = new as u32;
        }

        let mut odd = Vec::with_capacity(n);
        let mut even = Vec::with_capacity(n);
        let mut offsets = Vec::with_capacity(n + 1);
        let mut succ = Vec::with_capacity(game.num_of_edges());
        for &old in &perm {
            let vertex = VertexIndex::new(old);
            odd.push(game.owner(vertex) == Player::Odd);
            even.push(game.priority(vertex).value().is_multiple_of(2));
            offsets.push(succ.len());

            let start = succ.len();
            succ.extend(game.outgoing_edges(vertex).map(|edge| inv[*edge.to()]));
            succ[start..].sort_unstable();
            let mut unique = start;
            for i in start..succ.len() {
                if i == start || succ[i] != succ[unique - 1] {
                    succ[unique] = succ[i];
                    unique += 1;
                }
            }
            succ.truncate(unique);
        }
        offsets.push(succ.len());

        Self {
            odd,
            even,
            offsets,
            succ,
            perm,
            inv,
        }
    }

    /// Constructs a game that is already in relevance order (for tests).
    #[cfg(test)]
    fn from_lists(odd: Vec<bool>, even: Vec<bool>, successors: Vec<Vec<u32>>) -> Self {
        let n = odd.len();
        let mut offsets = Vec::new();
        let mut succ = Vec::new();
        for s in successors {
            offsets.push(succ.len());
            succ.extend(s);
        }
        offsets.push(succ.len());
        Self {
            odd,
            even,
            offsets,
            succ,
            perm: (0..n).collect(),
            inv: (0..n as u32).collect(),
        }
    }

    /// Returns the number of vertices.
    fn num_of_vertices(&self) -> usize {
        self.odd.len()
    }

    /// Returns the successors of `v`, in relevance order.
    fn successors(&self, v: usize) -> &[u32] {
        &self.succ[self.offsets[v]..self.offsets[v + 1]]
    }
}

/// The play values of a positional profile.
///
/// The vertices whose plays share the cycle maximum `λ` form a tree rooted at
/// `λ` (all vertices of the cycle of `λ` are identified with `λ`).
#[derive(Default)]
struct Valuation {
    /// The cycle maximum `λ` of every vertex.
    lambda: Vec<u32>,
    /// The number of steps `k` to the cycle maximum.
    steps: Vec<u32>,
    /// The depth in the tree of `λ`; zero on the cycle.
    depth: Vec<u32>,
    /// `up[j * n + x]`: the ancestor `2^j` steps above `x` (the root is its own parent).
    up: Vec<u32>,
    /// `best[j * n + x]`: one plus the maximum vertex of `B` among the `2^j`
    /// vertices from `x` upwards, or zero if there is none.
    best: Vec<u32>,
    /// The number of levels of `up` and `best`.
    levels: usize,
    /// Scratch space: the visit state of every vertex, and the current path.
    state: Vec<u8>,
    path: Vec<u32>,
}

impl Valuation {
    /// Computes the play values of the profile `f`, in `O(n log n)`.
    fn compute(&mut self, f: &[u32]) {
        let n = f.len();
        self.lambda.resize(n, 0);
        self.steps.resize(n, 0);
        self.depth.resize(n, 0);
        self.state.clear();
        self.state.resize(n, 0);
        self.up.resize(n, 0);
        self.best.resize(n, 0);

        let mut max_depth = 0;
        for s in 0..n {
            if self.state[s] != 0 {
                continue;
            }

            // Follow the play from `s` until a visited vertex is reached.
            self.path.clear();
            let mut u = s;
            while self.state[u] == 0 {
                self.state[u] = 1;
                self.path.push(u as u32);
                u = f[u] as usize;
            }

            if self.state[u] == 1 {
                // A new cycle was closed at `u`.
                let i = self
                    .path
                    .iter()
                    .position(|&x| x as usize == u)
                    .expect("u is on the current path");
                let cycle = &self.path[i..];
                let (j, &lambda) = cycle
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, x)| **x)
                    .expect("cycles are non-empty");
                let m = cycle.len();
                for (t, &x) in cycle.iter().enumerate() {
                    let x = x as usize;
                    self.lambda[x] = lambda;
                    self.steps[x] = ((j + m - t) % m) as u32;
                    self.depth[x] = 0;
                    self.up[x] = lambda;
                    self.best[x] = 0;
                    self.state[x] = 2;
                }
                self.path.truncate(i);
            }

            // The remaining path is a tail into an already evaluated vertex.
            while let Some(x) = self.path.pop() {
                let x = x as usize;
                let y = f[x] as usize;
                let lambda = self.lambda[y];
                self.lambda[x] = lambda;
                self.steps[x] = self.steps[y] + 1;
                self.depth[x] = self.depth[y] + 1;
                max_depth = max_depth.max(self.depth[x]);
                self.up[x] = if self.depth[y] == 0 { lambda } else { y as u32 };
                self.best[x] = if x as u32 > lambda { x as u32 + 1 } else { 0 };
                self.state[x] = 2;
            }
        }

        // Binary lifting tables.
        self.levels = (u32::BITS - max_depth.leading_zeros()).max(1) as usize;
        self.up.resize(self.levels * n, 0);
        self.best.resize(self.levels * n, 0);
        for j in 1..self.levels {
            let (lower, upper) = self.up.split_at_mut(j * n);
            let (best_lower, best_upper) = self.best.split_at_mut(j * n);
            let prev = &lower[(j - 1) * n..];
            let prev_best = &best_lower[(j - 1) * n..];
            for x in 0..n {
                let mid = prev[x] as usize;
                upper[x] = prev[mid];
                best_upper[x] = prev_best[x].max(prev_best[mid]);
            }
        }
    }

    /// The node representing `v` in the tree of its cycle maximum.
    fn node(&self, v: usize) -> usize {
        if self.depth[v] == 0 { self.lambda[v] as usize } else { v }
    }

    /// Lifts `x` by `d` levels, returning the new node and the best vertex passed.
    fn lift(&self, mut x: usize, d: u32, mut best: u32) -> (usize, u32) {
        let n = self.lambda.len();
        let mut j = 0;
        let mut d = d;
        while d > 0 {
            if d & 1 == 1 {
                best = best.max(self.best[j * n + x]);
                x = self.up[j * n + x] as usize;
            }
            d >>= 1;
            j += 1;
        }
        (x, best)
    }

    /// Compares the sets `B` of two vertices with the same cycle maximum.
    fn compare_b(&self, game: &RelabelledGame, a: usize, b: usize) -> Ordering {
        let n = self.lambda.len();
        let mut a = self.node(a);
        let mut b = self.node(b);
        if a == b {
            return Ordering::Equal;
        }

        let (da, db) = (self.depth[a], self.depth[b]);
        let (mut ma, mut mb) = (0, 0);
        if da > db {
            (a, ma) = self.lift(a, da - db, ma);
        } else if db > da {
            (b, mb) = self.lift(b, db - da, mb);
        }

        if a != b {
            for j in (0..self.levels).rev() {
                let (ua, ub) = (self.up[j * n + a], self.up[j * n + b]);
                if ua != ub {
                    ma = ma.max(self.best[j * n + a]);
                    mb = mb.max(self.best[j * n + b]);
                    a = ua as usize;
                    b = ub as usize;
                }
            }
            ma = ma.max(self.best[a]);
            mb = mb.max(self.best[b]);
        }

        match ma.cmp(&mb) {
            Ordering::Equal => Ordering::Equal,
            Ordering::Greater => {
                if game.even[ma as usize - 1] {
                    Ordering::Greater
                } else {
                    Ordering::Less
                }
            }
            Ordering::Less => {
                if game.even[mb as usize - 1] {
                    Ordering::Less
                } else {
                    Ordering::Greater
                }
            }
        }
    }

    /// The reward of the cycle maximum of `v`.
    fn reward(&self, game: &RelabelledGame, v: usize) -> i64 {
        let lambda = self.lambda[v];
        if game.even[lambda as usize] {
            lambda as i64 + 1
        } else {
            -(lambda as i64 + 1)
        }
    }

    /// Compares the play values of `a` and `b` (greater is better for Even).
    fn compare(&self, game: &RelabelledGame, a: u32, b: u32) -> Ordering {
        let (a, b) = (a as usize, b as usize);
        if a == b {
            return Ordering::Equal;
        }
        if self.lambda[a] != self.lambda[b] {
            return self.reward(game, a).cmp(&self.reward(game, b));
        }
        self.compare_b(game, a, b).then_with(|| {
            if game.even[self.lambda[a] as usize] {
                self.steps[b].cmp(&self.steps[a])
            } else {
                self.steps[a].cmp(&self.steps[b])
            }
        })
    }

    /// The first strictly best successor of `v` for Even (`maximise`) or Odd,
    /// starting from the current move `current`.
    fn best_of(&self, game: &RelabelledGame, v: usize, current: u32, maximise: bool) -> u32 {
        let wanted = if maximise { Ordering::Greater } else { Ordering::Less };
        let mut best = current;
        for &x in game.successors(v) {
            if self.compare(game, x, best) == wanted {
                best = x;
            }
        }
        best
    }
}

/// The state of a strategy improvement run on a relabelled game.
struct StrategyImprovement<'a> {
    game: &'a RelabelledGame,
    /// The valuation `Ξ_σ` of Even's current strategy.
    valuation: Valuation,
    /// The valuation of Even's best response `σ*` to `σ̄`.
    response: Valuation,
    statistics: StrategyImprovementStatistics,
}

impl<'a> StrategyImprovement<'a> {
    fn new(game: &'a RelabelledGame) -> Self {
        Self {
            game,
            valuation: Valuation::default(),
            response: Valuation::default(),
            statistics: StrategyImprovementStatistics::default(),
        }
    }

    /// Replaces the moves of player `odd` in `p` by a best response against the
    /// other player's moves, by switch-all one-player strategy improvement.
    /// Afterwards `valuation` holds the play values of `p`.
    fn best_response(game: &RelabelledGame, valuation: &mut Valuation, rounds: &mut usize, p: &mut [u32], odd: bool) {
        loop {
            valuation.compute(p);
            *rounds += 1;
            let mut changed = false;
            for v in 0..p.len() {
                if game.odd[v] == odd {
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

    /// Runs the algorithm from the profile `p`, returning the final profile:
    /// Even's optimal strategy and Odd's best response to it. Afterwards
    /// `valuation` holds the optimal valuation.
    fn run(&mut self, options: &StrategyImprovementOptions, mut p: Vec<u32>) -> Result<Vec<u32>, MercError> {
        let game = self.game;
        let n = p.len();
        let mut previous_response: Option<Vec<u32>> = None;
        let mut choice_filtered = vec![u32::MAX; n];
        let mut choice_improving = vec![u32::MAX; n];

        // State of the decisive-element statistics.
        let mut decisive = options.decisive.then(|| DecisiveStatistics {
            even_vertices: (0..n).filter(|&v| !game.odd[v]).count(),
            ..Default::default()
        });
        let mut snapshot: Option<Snapshot> = None;
        let mut counts = vec![0usize; n + 1];
        let mut interval_counts = vec![0usize; n + 1];
        let mut touched: Vec<usize> = Vec::new();
        let mut epoch_length = 0;
        let (mut left, mut right) = (Vec::new(), Vec::new());

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

            // The Ξ_σ-best improving edge of every Even vertex.
            let mut improvable = false;
            for v in 0..n {
                choice_improving[v] = u32::MAX;
                if !game.odd[v] {
                    let b = self.valuation.best_of(game, v, p[v], true);
                    if b != p[v] {
                        choice_improving[v] = b;
                        improvable = true;
                    }
                }
            }

            if let Some(statistics) = decisive.as_mut() {
                let current = Snapshot {
                    profile: ps.clone(),
                    lambda: self.valuation.lambda.clone(),
                    depth: self.valuation.depth.clone(),
                };
                if let Some(prev) = &snapshot {
                    if prev.lambda != current.lambda {
                        statistics.l_rounds += 1;
                        for &i in &touched {
                            interval_counts[i] = 0;
                        }
                        touched.clear();
                    } else {
                        let delta = decisive_element(prev, &current, &mut left, &mut right);
                        let index = (delta + 1) as usize;
                        counts[index] += 1;
                        if interval_counts[index] == 0 {
                            touched.push(index);
                        }
                        interval_counts[index] += 1;
                        if counts[index] > statistics.max_count {
                            statistics.max_count = counts[index];
                            statistics.max_count_element = delta;
                        }
                        statistics.max_interval_count = statistics.max_interval_count.max(interval_counts[index]);
                    }
                }
                snapshot = Some(current);
            }

            if !improvable {
                self.statistics.decisive = decisive;
                return Ok(ps);
            }

            // Count the epochs: the changes of Odd's best response.
            let changed = previous_response
                .as_ref()
                .is_none_or(|prev| (0..n).any(|v| game.odd[v] && prev[v] != ps[v]));
            if changed {
                self.statistics.epochs += 1;
                epoch_length = 0;
            }
            epoch_length += 1;
            if let Some(statistics) = decisive.as_mut() {
                statistics.max_epoch_length = statistics.max_epoch_length.max(epoch_length);
            }

            let mut q = match options.odd_warm_start {
                OddWarmStart::Initial => p.clone(),
                OddWarmStart::Previous => ps.clone(),
            };
            for v in 0..n {
                if !game.odd[v] {
                    q[v] = p[v];
                }
            }

            match options.algorithm {
                StrategyImprovementAlgorithm::VogeJurdzinski => {
                    for v in 0..n {
                        if choice_improving[v] != u32::MAX {
                            q[v] = choice_improving[v];
                        }
                    }
                }
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

                    let mut any_filtered = false;
                    for v in 0..n {
                        choice_filtered[v] = u32::MAX;
                        if choice_improving[v] == u32::MAX {
                            continue;
                        }

                        // The first maximum of the valuation of σ* over the successors.
                        let successors = game.successors(v);
                        let mut max = successors[0];
                        for &x in &successors[1..] {
                            if self.response.compare(game, x, max) == Ordering::Greater {
                                max = x;
                            }
                        }

                        // The Ξ_σ-best improving edge that is optimal against σ̄.
                        let current = p[v];
                        let mut best = u32::MAX;
                        for &w in successors {
                            if self.valuation.compare(game, w, current) == Ordering::Greater
                                && self.response.compare(game, w, max) == Ordering::Equal
                                && (best == u32::MAX || self.valuation.compare(game, w, best) == Ordering::Greater)
                            {
                                best = w;
                            }
                        }
                        if best != u32::MAX {
                            choice_filtered[v] = best;
                            any_filtered = true;
                        }
                    }

                    let choice = if any_filtered {
                        &choice_filtered
                    } else {
                        self.statistics.fallbacks += 1;
                        &choice_improving
                    };
                    for v in 0..n {
                        if choice[v] != u32::MAX {
                            q[v] = choice[v];
                        }
                    }
                }
            }

            trace!("Strategy improvement iteration {}", self.statistics.iterations);
            previous_response = Some(ps);
            p = q;
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
                for odd_warm_start in [OddWarmStart::Initial, OddWarmStart::Previous] {
                    let mut options = StrategyImprovementOptions::new(algorithm);
                    options.odd_warm_start = odd_warm_start;
                    let result = solve_strategy_improvement(&game, &options, true).unwrap();
                    assert_eq!(result.solution, expected, "{algorithm:?} disagrees with Zielonka");
                    verify_solution(&game, &result.solution, &result.strategy.unwrap());
                }
            }
        });
    }

    /// Compares the B-set order against the exact sum `Σ_{x ∈ B} sgn(x) 2^x` on small games.
    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_random_valuation_order() {
        random_test(200, |rng| {
            let game = random_parity_game(rng, true, 40, 40, 3);
            let relabelled = RelabelledGame::new(&game);
            let n = relabelled.num_of_vertices();
            let profile: Vec<u32> = (0..n)
                .map(|v| {
                    let s = relabelled.successors(v);
                    s[v % s.len()]
                })
                .collect();

            let mut valuation = Valuation::default();
            valuation.compute(&profile);

            // Reference: follow the play explicitly.
            let key = |v: usize| -> (i64, i128, i64) {
                let lambda = valuation.lambda[v] as usize;
                let mut sum: i128 = 0;
                let mut x = v;
                let mut k = 0;
                while x != lambda {
                    if x > lambda && valuation.depth[x] > 0 {
                        sum += if relabelled.even[x] { 1i128 << x } else { -(1i128 << x) };
                    }
                    x = profile[x] as usize;
                    k += 1;
                }
                assert_eq!(k, valuation.steps[v] as i64);
                let even = relabelled.even[lambda];
                let reward = if even { lambda as i64 + 1 } else { -(lambda as i64 + 1) };
                (reward, sum, if even { -k } else { k })
            };

            for a in 0..n {
                for b in 0..n {
                    assert_eq!(
                        valuation.compare(&relabelled, a as u32, b as u32),
                        key(a).cmp(&key(b)),
                        "Comparison of {a} and {b} is incorrect"
                    );
                }
            }
        });
    }

    /// Iteration counts of the Python reference `algs.run` on fixed games.
    #[test]
    fn test_reference_iteration_counts() {
        for (text, init, hb, vj) in REFERENCE {
            let game = read_pg(text.as_bytes()).unwrap();
            let mut profile: Vec<VertexIndex> = game
                .iter_vertices()
                .map(|v| game.outgoing_edges(v).next().unwrap().to())
                .collect();
            for line in init.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                profile[parts[0].parse::<usize>().unwrap()] = VertexIndex::new(parts[2].parse().unwrap());
            }

            for (algorithm, expected) in [
                (StrategyImprovementAlgorithm::HybridB, hb),
                (StrategyImprovementAlgorithm::VogeJurdzinski, vj),
            ] {
                let mut options = StrategyImprovementOptions::new(algorithm);
                options.initial_profile = Some(profile.clone());
                let result = solve_strategy_improvement(&game, &options, false).unwrap();
                let statistics = &result.statistics;
                assert_eq!(
                    (
                        statistics.iterations,
                        statistics.best_response_rounds,
                        statistics.epochs
                    ),
                    *expected,
                    "{algorithm:?} on\n{text}"
                );
            }
        }
    }

    #[test]
    fn test_relabelled_from_lists() {
        // A two vertex game: Even at 0 (priority 0) with a choice, Odd at 1 (priority 1).
        let game = RelabelledGame::from_lists(vec![false, true], vec![true, false], vec![vec![0, 1], vec![1]]);
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
        let mut profile: Vec<VertexIndex> = game.iter_vertices().collect();
        for line in "0 7\n1 7\n2 0\n3 13\n4 10\n5 9\n6 6\n7 13\n8 0\n9 0\n10 10\n11 7\n12 13\n13 10\n".lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            profile[parts[0].parse::<usize>().unwrap()] = VertexIndex::new(parts[1].parse().unwrap());
        }

        for odd_warm_start in [OddWarmStart::Initial, OddWarmStart::Previous] {
            let mut options = StrategyImprovementOptions::new(StrategyImprovementAlgorithm::HybridB);
            options.initial_profile = Some(profile.clone());
            options.odd_warm_start = odd_warm_start;
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

    include!("strategy_improvement_reference.rs");
}
