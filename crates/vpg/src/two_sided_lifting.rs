use std::cmp::Ordering;
use std::collections::VecDeque;

use bitvec::bitvec;
use bitvec::order::Lsb0;
use itertools::Either;
use log::debug;
use rustc_hash::FxHashMap;

use crate::PG;
use crate::Player;
use crate::Pred;
use crate::Predecessors;
use crate::Set;
use crate::Strategy;
use crate::Subgame;
use crate::VertexIndex;

/// Solves the given parity game using two-sided progress-measure lifting over
/// instance-derived trees. Experimental: there is no proof that this runs in
/// polynomial time.
///
/// Small progress measures lift over a codomain fixed by the number of vertices
/// and priorities, and the quasi-polynomial variants over universal trees,
/// which are provably quasi-polynomial in size. A progress measure is however
/// sound for any priority-indexed well-founded codomain, so this solver lifts
/// over a tree derived from the game itself (the chain tree). That tree is
/// only small for the winner of a region, so both players lift at once:
///
/// 1. Every round each player performs a bounded number of lifts.
/// 2. The largest set on which a player's current values already form a
///    progress measure is certified as won by that player.
/// 3. Certified sets and their attractors are removed, and the trees are
///    rebuilt for the remaining subgame.
/// 4. If the current choices close a strongly connected set in which every
///    cycle has an odd maximum, a vertex of the lifting player must leave that
///    set in the least fixpoint. All its vertices are raised to the value of
///    the cheapest exit, which skips the step-by-step counting that makes
///    lifting exponential.
///
/// Only the winning regions are computed; the returned strategy is always
/// `None`, regardless of `compute_strategy`.
pub fn solve_two_sided_lifting<G: PG>(game: &G, _compute_strategy: bool) -> ([Set; 2], Option<[Strategy; 2]>) {
    debug_assert!(game.is_total(), "Two-sided lifting requires a total parity game");

    let predecessors = Predecessors::new(game);
    let mut stats = Stats::default();
    let winner = solve_game(game, &predecessors, &mut stats);
    debug!(
        "two-sided lifting: {} steps, {} rounds, {} restarts",
        stats.steps, stats.rounds, stats.restarts
    );

    let mut w0 = bitvec![usize, Lsb0; 0; game.num_of_vertices()];
    let mut w1 = bitvec![usize, Lsb0; 0; game.num_of_vertices()];
    for (v, player) in winner.iter().enumerate() {
        match player {
            Player::Even => w0.set(v, true),
            Player::Odd => w1.set(v, true),
        }
    }
    ([w0, w1], None)
}

/// Counters describing the work done by the solver.
#[derive(Debug, Default)]
struct Stats {
    /// Number of lifts, including raises by acceleration.
    steps: usize,
    /// Number of rounds of interleaved lifting.
    rounds: usize,
    /// Number of times the trees were rebuilt on a smaller subgame.
    restarts: usize,
}

/// The subgame of the remaining vertices as seen by one player: priorities are
/// shifted by one for the odd player, so that the lifting player always wants
/// an even maximum.
struct View<'a, G: PG> {
    subgame: Subgame<'a, G, &'a Predecessors<'a>>,
    player: Player,
}

impl<'a, G: PG> View<'a, G> {
    fn new(game: &'a G, predecessors: &'a Predecessors<'a>, alive: Set, player: Player) -> Self {
        Self {
            subgame: Subgame::new(game, alive, predecessors),
            player,
        }
    }

    /// The size of the vertex index space, including removed vertices.
    fn len(&self) -> usize {
        self.subgame.num_of_vertices()
    }

    fn vertices(&self) -> impl Iterator<Item = usize> + '_ {
        self.subgame.iter_vertices().map(|v| *v)
    }

    fn mine(&self, v: usize) -> bool {
        self.subgame.owner(VertexIndex::new(v)) == self.player
    }

    fn prio(&self, v: usize) -> usize {
        self.subgame.priority(VertexIndex::new(v)).value() + usize::from(self.player == Player::Odd)
    }

    fn succ(&self, v: usize) -> impl Iterator<Item = usize> + '_ {
        self.subgame.outgoing_edges(VertexIndex::new(v)).map(|edge| *edge.to())
    }

    fn pred(&self, v: usize) -> impl Iterator<Item = usize> + '_ {
        self.subgame.predecessors(VertexIndex::new(v)).map(|u| *u)
    }
}

/// One element of a [`ChainTree`] path: the index of the component within its
/// parent, the counter value, and the region the element belongs to. The
/// region is determined by the preceding elements, so including it last does
/// not change the lexicographic order.
type Element = (u32, u32, u32);

/// A leaf of a [`ChainTree`]: one element per odd level.
type Leaf = Vec<Element>;

/// A measure value; `None` is the top element.
type Value = Option<Leaf>;

fn cmp_value(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => x.cmp(y),
    }
}

/// Counter bound and ordered child components of a region.
struct RegionInfo {
    bound: u32,
    components: Vec<u32>,
}

/// A set of vertices at one odd level of a [`ChainTree`].
struct Region {
    vertices: Vec<usize>,
    level: usize,
    info: Option<RegionInfo>,
}

/// The chain tree of a game, which is never materialised.
///
/// At odd level `q` inside a region `R` the counter ranges up to the longest
/// path through the SCC condensation of `R` restricted to priorities at most
/// `q`, where every SCC weighs its number of priority-`q` vertices. Below every
/// counter value the trees of the weakly connected components of `R`
/// restricted to priorities below `q` are placed side by side. On the hard
/// family of Czerwiński et al. this tree has at most `n^2` leaves for the
/// winner, whereas universal trees need quasi-polynomially many.
struct ChainTree {
    /// The odd priorities of the view, in decreasing order.
    levels: Vec<usize>,
    regions: Vec<Region>,
    region_ids: FxHashMap<(usize, Vec<usize>), u32>,
    tarjan: TarjanScratch,
    inside: StampSet,
}

impl ChainTree {
    fn new<G: PG>(view: &View<'_, G>) -> Self {
        let mut levels: Vec<usize> = view
            .vertices()
            .map(|v| view.prio(v))
            .filter(|p| !p.is_multiple_of(2))
            .collect();
        levels.sort_unstable_by(|a, b| b.cmp(a));
        levels.dedup();
        let mut tree = Self {
            levels,
            regions: Vec::new(),
            region_ids: FxHashMap::default(),
            tarjan: TarjanScratch::new(view.len()),
            inside: StampSet::new(view.len()),
        };
        tree.intern(0, view.vertices().collect());
        tree
    }

    fn height(&self) -> usize {
        self.levels.len()
    }

    fn intern(&mut self, level: usize, vertices: Vec<usize>) -> u32 {
        if let Some(&id) = self.region_ids.get(&(level, vertices.clone())) {
            return id;
        }
        let id = self.regions.len() as u32;
        self.region_ids.insert((level, vertices.clone()), id);
        self.regions.push(Region {
            vertices,
            level,
            info: None,
        });
        id
    }

    fn info<G: PG>(&mut self, view: &View<'_, G>, region: u32) -> &RegionInfo {
        let r = region as usize;
        if self.regions[r].info.is_none() {
            let level = self.regions[r].level;
            let q = self.levels[level];
            let vertices = &self.regions[r].vertices;
            let below_q: Vec<usize> = vertices.iter().copied().filter(|&v| view.prio(v) <= q).collect();
            let strictly_below: Vec<usize> = vertices.iter().copied().filter(|&v| view.prio(v) < q).collect();
            let bound = self.chain_bound(view, &below_q, q);
            let mut components = Vec::new();
            if level + 1 < self.height() {
                let mut comps = self.weak_components(view, &strictly_below);
                comps.sort_unstable_by_key(|comp| comp[0]);
                if comps.is_empty() {
                    comps.push(Vec::new());
                }
                for comp in comps {
                    components.push(self.intern(level + 1, comp));
                }
            }
            self.regions[r].info = Some(RegionInfo { bound, components });
        }
        self.regions[r].info.as_ref().expect("computed above")
    }

    /// Longest path through the SCC condensation of the subgraph induced by
    /// `members`, weighting each SCC by its number of priority-`q` vertices.
    fn chain_bound<G: PG>(&mut self, view: &View<'_, G>, members: &[usize], q: usize) -> u32 {
        self.inside.clear();
        for &v in members {
            self.inside.insert(v);
        }
        let inside = &self.inside;
        let comps = strongly_connected_components(&mut self.tarjan, members, |v| view.succ(v), |w| inside.contains(w));
        let mut comp_of: FxHashMap<usize, usize> = FxHashMap::default();
        for (i, comp) in comps.iter().enumerate() {
            for &v in comp {
                comp_of.insert(v, i);
            }
        }
        // Components are produced sinks first, so successors are already known.
        let mut best = vec![0u32; comps.len()];
        for (i, comp) in comps.iter().enumerate() {
            let weight = comp.iter().filter(|&&v| view.prio(v) == q).count() as u32;
            let mut next = 0;
            for &v in comp {
                for w in view.succ(v) {
                    if let Some(&j) = comp_of.get(&w)
                        && j != i
                    {
                        next = next.max(best[j]);
                    }
                }
            }
            best[i] = weight + next;
        }
        best.into_iter().max().unwrap_or(0)
    }

    /// Weakly connected components of the subgraph induced by `members`.
    fn weak_components<G: PG>(&mut self, view: &View<'_, G>, members: &[usize]) -> Vec<Vec<usize>> {
        // Vertices still to be visited are kept in `inside`.
        self.inside.clear();
        for &v in members {
            self.inside.insert(v);
        }
        let mut comps = Vec::new();
        for &root in members {
            if !self.inside.contains(root) {
                continue;
            }
            self.inside.remove(root);
            let mut comp = vec![root];
            let mut stack = vec![root];
            while let Some(x) = stack.pop() {
                for y in view.succ(x).chain(view.pred(x)) {
                    if self.inside.contains(y) {
                        self.inside.remove(y);
                        comp.push(y);
                        stack.push(y);
                    }
                }
            }
            comp.sort_unstable();
            comps.push(comp);
        }
        comps
    }

    /// Extends the prefix with the first child at every remaining level.
    fn leftmost<G: PG>(&mut self, view: &View<'_, G>, prefix: &[Element]) -> Leaf {
        let mut leaf = Vec::with_capacity(self.height());
        leaf.extend_from_slice(prefix);
        while leaf.len() < self.height() {
            let region = match leaf.last() {
                None => 0,
                Some(&(_, _, parent)) => self.info(view, parent).components[0],
            };
            leaf.push((0, 0, region));
        }
        leaf
    }

    /// The next node at the same depth in lexicographic order, if any.
    fn next_node<G: PG>(&mut self, view: &View<'_, G>, prefix: &[Element]) -> Option<Leaf> {
        let mut path = prefix.to_vec();
        while let Some(&(k, c, region)) = path.last() {
            let depth = path.len() - 1;
            if c < self.info(view, region).bound {
                path[depth] = (k, c + 1, region);
                return Some(path);
            }
            if depth > 0 {
                let parent = path[depth - 1].2;
                let siblings = &self.info(view, parent).components;
                if let Some(&next) = siblings.get(k as usize + 1) {
                    path[depth] = (k + 1, 0, next);
                    return Some(path);
                }
            }
            path.pop();
        }
        None
    }
}

/// A set of vertices that can be cleared in constant time.
#[derive(Default)]
struct StampSet {
    stamp: Vec<u32>,
    epoch: u32,
}

impl StampSet {
    fn new(n: usize) -> Self {
        Self {
            stamp: vec![0; n],
            epoch: 1,
        }
    }

    fn clear(&mut self) {
        if self.epoch == u32::MAX {
            self.stamp.fill(0);
            self.epoch = 0;
        }
        self.epoch += 1;
    }

    /// Inserts `v`, returns true if it was not yet present.
    fn insert(&mut self, v: usize) -> bool {
        let fresh = self.stamp[v] != self.epoch;
        self.stamp[v] = self.epoch;
        fresh
    }

    fn remove(&mut self, v: usize) {
        self.stamp[v] = 0;
    }

    fn contains(&self, v: usize) -> bool {
        self.stamp[v] == self.epoch
    }
}

/// Reusable state for [`strongly_connected_components`].
#[derive(Default)]
struct TarjanScratch {
    visited: StampSet,
    on_stack: StampSet,
    index: Vec<usize>,
    low: Vec<usize>,
}

impl TarjanScratch {
    fn new(n: usize) -> Self {
        Self {
            visited: StampSet::new(n),
            on_stack: StampSet::new(n),
            index: vec![0; n],
            low: vec![0; n],
        }
    }
}

/// Tarjan's algorithm on the subgraph induced by `members`, following only the
/// successors for which `inside` holds. Returns the strongly connected
/// components in reverse topological order.
///
/// Unlike `merc_collections::scc_decomposition_iterative` the work is
/// proportional to the members and their edges, which matters because the
/// solver decomposes many small subsets of a large game.
fn strongly_connected_components<I: Iterator<Item = usize>>(
    scratch: &mut TarjanScratch,
    members: &[usize],
    successors: impl Fn(usize) -> I,
    inside: impl Fn(usize) -> bool,
) -> Vec<Vec<usize>> {
    scratch.visited.clear();
    scratch.on_stack.clear();
    let mut stack = Vec::new();
    let mut comps = Vec::new();
    let mut counter = 0;

    for &root in members {
        if !scratch.visited.insert(root) {
            continue;
        }
        scratch.index[root] = counter;
        scratch.low[root] = counter;
        counter += 1;
        stack.push(root);
        scratch.on_stack.insert(root);
        let mut calls: Vec<(usize, I)> = vec![(root, successors(root))];

        while let Some(top) = calls.last_mut() {
            let v = top.0;
            match top.1.next() {
                Some(w) => {
                    if !inside(w) {
                        continue;
                    }
                    if scratch.visited.insert(w) {
                        scratch.index[w] = counter;
                        scratch.low[w] = counter;
                        counter += 1;
                        stack.push(w);
                        scratch.on_stack.insert(w);
                        calls.push((w, successors(w)));
                    } else if scratch.on_stack.contains(w) {
                        scratch.low[v] = scratch.low[v].min(scratch.index[w]);
                    }
                }
                None => {
                    calls.pop();
                    if let Some((parent, _)) = calls.last() {
                        let parent = *parent;
                        scratch.low[parent] = scratch.low[parent].min(scratch.low[v]);
                    }
                    if scratch.low[v] == scratch.index[v] {
                        let mut comp = Vec::new();
                        while let Some(x) = stack.pop() {
                            scratch.on_stack.remove(x);
                            comp.push(x);
                            if x == v {
                                break;
                            }
                        }
                        comps.push(comp);
                    }
                }
            }
        }
    }
    comps
}

/// Reusable vertex sets for acceleration.
#[derive(Default)]
struct AccelerationScratch {
    forward: StampSet,
    scc: StampSet,
    allowed: StampSet,
    inside: StampSet,
    tarjan: TarjanScratch,
}

impl AccelerationScratch {
    fn new(n: usize) -> Self {
        Self {
            forward: StampSet::new(n),
            scc: StampSet::new(n),
            allowed: StampSet::new(n),
            inside: StampSet::new(n),
            tarjan: TarjanScratch::new(n),
        }
    }
}

/// The least value for `v` that satisfies the progress condition towards a
/// successor with value `target`.
fn prog<G: PG>(tree: &mut ChainTree, view: &View<'_, G>, depth: usize, v: usize, target: &Value) -> Value {
    let prefix = &target.as_ref()?[..depth];
    if view.prio(v).is_multiple_of(2) {
        Some(tree.leftmost(view, prefix))
    } else {
        let next = tree.next_node(view, prefix)?;
        Some(tree.leftmost(view, &next))
    }
}

/// The edges kept for acceleration: all edges of the lifting player and the
/// recorded choice of the opponent, which must have been computed.
fn kept<'b, G: PG>(view: &'b View<'_, G>, choice: &'b [usize], u: usize) -> impl Iterator<Item = usize> + 'b {
    if view.mine(u) {
        Either::Left(view.succ(u))
    } else {
        Either::Right(std::iter::once(choice[u]))
    }
}

/// Progress-measure lifting for one player over its chain tree.
struct Lifter<'a, G: PG> {
    view: View<'a, G>,
    tree: ChainTree,
    /// Number of tree levels compared by the progress condition of each vertex.
    depth: Vec<usize>,
    measure: Vec<Value>,
    /// The successor attaining the value of each vertex when it was last
    /// evaluated, `usize::MAX` if it has not been evaluated yet. A stale choice
    /// of an opponent vertex is still an edge, which is all acceleration needs.
    choice: Vec<usize>,
    queue: VecDeque<usize>,
    in_queue: Vec<bool>,
    steps: usize,
    scratch: AccelerationScratch,
}

impl<'a, G: PG> Lifter<'a, G> {
    fn new(game: &'a G, predecessors: &'a Predecessors<'a>, alive: Set, player: Player) -> Self {
        let view = View::new(game, predecessors, alive, player);
        let mut tree = ChainTree::new(&view);
        let n = view.len();
        let mut depth = vec![0; n];
        for v in view.vertices() {
            let p = view.prio(v);
            depth[v] = tree
                .levels
                .iter()
                .filter(|&&q| if p.is_multiple_of(2) { q > p } else { q >= p })
                .count();
        }
        let bottom = Some(tree.leftmost(&view, &[]));
        let queue: VecDeque<usize> = view.vertices().collect();
        let mut in_queue = vec![false; n];
        for &v in &queue {
            in_queue[v] = true;
        }
        Self {
            view,
            tree,
            depth,
            measure: vec![bottom; n],
            choice: vec![usize::MAX; n],
            queue,
            in_queue,
            steps: 0,
            scratch: AccelerationScratch::new(n),
        }
    }

    /// The value `v` must have given its successors, optionally restricted to
    /// the vertices in `allowed`, and the successor attaining it. Ties are
    /// broken towards the first successor for the lifting player and the last
    /// otherwise.
    fn evaluate(&mut self, v: usize, allowed: Option<&[bool]>) -> (Value, Option<usize>) {
        let mine = self.view.mine(v);
        let mut result: Option<(Value, usize)> = None;
        for w in self.view.succ(v) {
            if allowed.is_some_and(|allowed| !allowed[w]) {
                continue;
            }
            let value = prog(&mut self.tree, &self.view, self.depth[v], v, &self.measure[w]);
            let better = match &result {
                None => true,
                Some((current, _)) => {
                    let ord = cmp_value(&value, current);
                    if mine {
                        ord == Ordering::Less
                    } else {
                        ord != Ordering::Less
                    }
                }
            };
            if better {
                result = Some((value, w));
            }
        }
        match result {
            Some((value, w)) => (value, Some(w)),
            None => (None, None),
        }
    }

    fn current_choice(&mut self, v: usize) -> usize {
        if self.choice[v] == usize::MAX {
            let (_, w) = self.evaluate(v, None);
            self.choice[v] = w.expect("the game is total");
        }
        self.choice[v]
    }

    fn requeue(&mut self, v: usize) {
        if !self.in_queue[v] {
            self.in_queue[v] = true;
            self.queue.push_back(v);
        }
    }

    fn requeue_predecessors(&mut self, v: usize) {
        for u in self.view.pred(v) {
            if !self.in_queue[u] {
                self.in_queue[u] = true;
                self.queue.push_back(u);
            }
        }
    }

    /// Performs at most `budget` lifts; returns true when the least fixpoint is
    /// reached.
    fn run(&mut self, budget: usize) -> bool {
        let mut done = 0;
        while done < budget {
            let Some(v) = self.queue.pop_front() else {
                break;
            };
            self.in_queue[v] = false;
            let (value, w) = self.evaluate(v, None);
            self.choice[v] = w.expect("the game is total");
            if cmp_value(&value, &self.measure[v]) == Ordering::Greater {
                self.measure[v] = value;
                self.steps += 1;
                done += 1;
                self.requeue_predecessors(v);
                let raised = self.accelerate(v);
                self.steps += raised;
                done += raised;
            }
        }
        self.queue.is_empty()
    }

    /// The largest set on which the current values form a progress measure.
    fn certified(&mut self) -> Vec<usize> {
        let mut domain = vec![false; self.view.len()];
        let mut queue = VecDeque::new();
        for v in self.view.vertices() {
            if self.measure[v].is_some() {
                domain[v] = true;
                queue.push_back(v);
            }
        }
        let mut queued = domain.clone();
        while let Some(v) = queue.pop_front() {
            queued[v] = false;
            if !domain[v] {
                continue;
            }
            let consistent = if !self.view.mine(v) && self.view.succ(v).any(|w| !domain[w]) {
                false
            } else {
                let (required, _) = self.evaluate(v, Some(&domain));
                cmp_value(&self.measure[v], &required) != Ordering::Less
            };
            if !consistent {
                domain[v] = false;
                for u in self.view.pred(v) {
                    if domain[u] && !queued[u] {
                        queued[u] = true;
                        queue.push_back(u);
                    }
                }
            }
        }
        self.view.vertices().filter(|&v| domain[v]).collect()
    }

    /// The maxima of all cycles with an even maximum among the kept edges
    /// within `members`. An SCC with an even maximum contains a cycle through
    /// every vertex of that priority; otherwise the maxima are removed and the
    /// rest is decomposed again. A cycle with maximum `p` never contains the
    /// removed higher vertices, so every such cycle is found.
    fn even_cycle_maxima(&mut self, members: &[usize]) -> Vec<usize> {
        let mut scratch = std::mem::take(&mut self.scratch);
        let mut bad = Vec::new();
        let mut work = vec![members.to_vec()];
        while let Some(set) = work.pop() {
            scratch.inside.clear();
            for &u in &set {
                scratch.inside.insert(u);
            }
            let inside = &scratch.inside;
            let comps = strongly_connected_components(
                &mut scratch.tarjan,
                &set,
                |u| kept(&self.view, &self.choice, u),
                |w| inside.contains(w),
            );
            for comp in comps {
                let cyclic = comp.len() > 1 || kept(&self.view, &self.choice, comp[0]).any(|w| w == comp[0]);
                if !cyclic {
                    continue;
                }
                let top = comp.iter().map(|&u| self.view.prio(u)).max().expect("non-empty");
                if top.is_multiple_of(2) {
                    bad.extend(comp.iter().copied().filter(|&u| self.view.prio(u) == top));
                }
                let rest: Vec<usize> = comp.into_iter().filter(|&u| self.view.prio(u) != top).collect();
                if !rest.is_empty() {
                    work.push(rest);
                }
            }
        }
        self.scratch = scratch;
        bad
    }

    /// Pump acceleration. Only runs when the current choices of both players
    /// close a cycle through `v`; it then takes the strongly connected set of
    /// `v` in the graph of kept edges, peels off the maxima of cycles with an
    /// even maximum, and raises every vertex of the remaining set to the least
    /// value reachable through an exit. Returns the number of raised vertices.
    fn accelerate(&mut self, v: usize) -> usize {
        // Cheap pump test: do the current choices return to v?
        let mut x = v;
        let mut on_cycle = false;
        for _ in 0..self.view.len() {
            x = self.current_choice(x);
            if x == v {
                on_cycle = true;
                break;
            }
        }
        if !on_cycle {
            return 0;
        }

        let mut restricted = false;
        let members = loop {
            // Vertices reachable from v, within the allowed set once restricted.
            self.scratch.forward.clear();
            self.scratch.forward.insert(v);
            let mut stack = vec![v];
            while let Some(x) = stack.pop() {
                if !self.view.mine(x) {
                    self.current_choice(x);
                }
                for y in kept(&self.view, &self.choice, x) {
                    if (!restricted || self.scratch.allowed.contains(y)) && self.scratch.forward.insert(y) {
                        stack.push(y);
                    }
                }
            }
            // Of those, the vertices that reach v.
            self.scratch.scc.clear();
            self.scratch.scc.insert(v);
            let mut members = vec![v];
            let mut stack = vec![v];
            while let Some(x) = stack.pop() {
                for u in self.view.pred(x) {
                    if self.scratch.forward.contains(u)
                        && !self.scratch.scc.contains(u)
                        && kept(&self.view, &self.choice, u).any(|y| y == x)
                    {
                        self.scratch.scc.insert(u);
                        members.push(u);
                        stack.push(u);
                    }
                }
            }
            if members.len() == 1 && !kept(&self.view, &self.choice, v).any(|y| y == v) {
                return 0;
            }
            let bad = self.even_cycle_maxima(&members);
            if bad.is_empty() {
                break members;
            }
            if bad.contains(&v) {
                return 0;
            }
            self.scratch.allowed.clear();
            for &u in &members {
                self.scratch.allowed.insert(u);
            }
            for u in bad {
                self.scratch.allowed.remove(u);
            }
            restricted = true;
        };
        // On exit from the loop `scc` holds exactly the members.

        // Lower bounds: cheapest exit, propagated backwards along paths in the set.
        let mut bound: FxHashMap<usize, Value> = FxHashMap::default();
        for &s in &members {
            let mut exit: Value = None;
            for w in kept(&self.view, &self.choice, s) {
                if !self.scratch.scc.contains(w) {
                    let value = prog(&mut self.tree, &self.view, self.depth[s], s, &self.measure[w]);
                    if cmp_value(&value, &exit) == Ordering::Less {
                        exit = value;
                    }
                }
            }
            bound.insert(s, exit);
        }
        for _ in 0..members.len() {
            let mut changed = false;
            for &s in &members {
                for w in kept(&self.view, &self.choice, s) {
                    if self.scratch.scc.contains(w) {
                        let value = prog(&mut self.tree, &self.view, self.depth[s], s, &bound[&w]);
                        if cmp_value(&value, &bound[&s]) == Ordering::Less {
                            bound.insert(s, value);
                            changed = true;
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        let mut raised = 0;
        for s in members {
            let value = bound.remove(&s).expect("bounded");
            if cmp_value(&value, &self.measure[s]) == Ordering::Greater {
                self.measure[s] = value;
                raised += 1;
                self.requeue_predecessors(s);
                self.requeue(s);
            }
        }
        raised
    }
}

/// The `player`-attractor of `target` within the alive vertices.
fn attractor<G: PG>(
    game: &G,
    predecessors: &Predecessors<'_>,
    alive: &Set,
    player: Player,
    target: &[usize],
) -> Vec<usize> {
    let mut attracted = bitvec![usize, Lsb0; 0; game.num_of_vertices()];
    // Number of alive successors of an opponent vertex not yet attracted.
    let mut remaining: FxHashMap<usize, usize> = FxHashMap::default();
    let mut queue: Vec<usize> = Vec::new();
    for &v in target {
        if alive[v] && !attracted[v] {
            attracted.set(v, true);
            queue.push(v);
        }
    }
    let mut result = queue.clone();
    while let Some(w) = queue.pop() {
        for u in predecessors.predecessors(VertexIndex::new(w)) {
            let u = *u;
            if !alive[u] || attracted[u] {
                continue;
            }
            let attract = if game.owner(VertexIndex::new(u)) == player {
                true
            } else {
                let count = remaining.entry(u).or_insert_with(|| {
                    game.outgoing_edges(VertexIndex::new(u))
                        .filter(|edge| alive[*edge.to()])
                        .count()
                });
                *count -= 1;
                *count == 0
            };
            if attract {
                attracted.set(u, true);
                queue.push(u);
                result.push(u);
            }
        }
    }
    result
}

/// Solves the game and returns the winner of every vertex.
fn solve_game<G: PG>(game: &G, predecessors: &Predecessors<'_>, stats: &mut Stats) -> Vec<Player> {
    let n = game.num_of_vertices();
    let mut alive = bitvec![usize, Lsb0; 1; n];
    let mut winner = vec![Player::Even; n];
    let mut remaining = n;

    while remaining > 0 {
        stats.restarts += 1;
        let mut lifters = [
            Lifter::new(game, predecessors, alive.clone(), Player::Even),
            Lifter::new(game, predecessors, alive.clone(), Player::Odd),
        ];

        loop {
            stats.rounds += 1;
            for (index, player) in [Player::Even, Player::Odd].into_iter().enumerate() {
                if lifters[index].run(remaining) {
                    // A complete least fixpoint decides the whole subgame.
                    for v in alive.iter_ones() {
                        winner[v] = if lifters[index].measure[v].is_some() {
                            player
                        } else {
                            player.opponent()
                        };
                    }
                    stats.steps += lifters[0].steps + lifters[1].steps;
                    return winner;
                }
            }

            let certified = [lifters[0].certified(), lifters[1].certified()];
            if certified.iter().all(Vec::is_empty) {
                continue;
            }

            for (index, player) in [Player::Even, Player::Odd].into_iter().enumerate() {
                for v in attractor(game, predecessors, &alive, player, &certified[index]) {
                    winner[v] = player;
                    alive.set(v, false);
                    remaining -= 1;
                }
            }
            stats.steps += lifters[0].steps + lifters[1].steps;
            break;
        }
    }
    winner
}

#[cfg(test)]
mod tests {
    use merc_utilities::random_test;

    use crate::PG;
    use crate::random_parity_game;
    use crate::solve_zielonka;

    use super::solve_two_sided_lifting;

    fn assert_matches_zielonka<G: PG>(game: &G) {
        let (lifting_solution, _) = solve_two_sided_lifting(game, false);
        let (zielonka_solution, _) = solve_zielonka(game, false);
        assert_eq!(
            lifting_solution, zielonka_solution,
            "Winning sets differ between two-sided lifting and Zielonka"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Miri is too slow for these tests.
    fn test_two_sided_lifting_matches_zielonka() {
        random_test(100, |rng| {
            assert_matches_zielonka(&random_parity_game(rng, true, 40, 6, 3))
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_two_sided_lifting_many_priorities() {
        // Mostly distinct priorities give deep chain trees.
        random_test(50, |rng| {
            assert_matches_zielonka(&random_parity_game(rng, true, 60, 60, 2))
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_two_sided_lifting_dense() {
        random_test(50, |rng| {
            assert_matches_zielonka(&random_parity_game(rng, true, 30, 3, 10))
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_two_sided_lifting_large() {
        random_test(5, |rng| {
            assert_matches_zielonka(&random_parity_game(rng, true, 1000, 20, 3))
        });
    }
}
