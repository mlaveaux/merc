#![allow(nonstandard_style)]
#![allow(unused)]
//! To keep with the theory, we use capitalized variable names for sets of vertices.
//! Authors: Maurice Laveaux, Sjef van Loo, Erik de Vink and Tim A.C. Willemse

use std::fmt;
use std::ops::Index;

use bitvec::order::Lsb0;
use bitvec::vec::BitVec;
use log::debug;
use log::trace;
use merc_utilities::MercError;
use oxidd::BooleanFunction;
use oxidd::ManagerRef;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;

use crate::Player;
use crate::Priority;
use crate::VariabilityParityGame;
use crate::VariabilityPredecessors;
use crate::VertexIndex;

/// Solves the given variability parity game using the Zielonka algorithm.
pub fn solve_variability_zielonka(
    manager_ref: &BDDManagerRef,
    game: &VariabilityParityGame,
    alternative_solving: bool,
) -> Result<[Submap; 2], MercError> {
    let mut zielonka = VariabilityZielonkaSolver::new(manager_ref, game, alternative_solving);

    // Determine the initial set of vertices V
    let V = Submap::new(
        manager_ref.with_manager_shared(|manager| {
            if alternative_solving {
                BDDFunction::t(manager)
            } else {
                game.configuration().clone()
            }
        }),
        game.num_of_vertices(),
    );

    let W = zielonka.solve_recursive(V)?;

    // Check that the result is a valid partition
    if cfg!(debug_assertions) {
        for v in game.iter_vertices() {
            let tmp = W[0][v].or(&W[1][v])?;
            
            // The union of both solutions should be the entire set of vertices.
            debug_assert!(tmp == manager_ref.with_manager_shared(|manager| {
                if alternative_solving {
                    BDDFunction::t(manager)
                } else {
                    game.configuration().clone()
                }
            }));
        }
    }

    Ok(W)
}

struct VariabilityZielonkaSolver<'a> {
    game: &'a VariabilityParityGame,

    manager_ref: &'a BDDManagerRef,

    /// Whether to use an alternative solving method.
    alternative_solving: bool,

    /// Reused temporary queue for attractor computation.
    temp_queue: Vec<VertexIndex>,

    /// Reused temporary vertices for attractor computation.
    temp_vertices: BitVec<usize, Lsb0>,

    /// Stores the predecessors of the game.
    predecessors: VariabilityPredecessors,

    /// Temporary storage for vertices per priority.
    priority_vertices: Vec<Vec<VertexIndex>>,

    /// Keeps track of the total number of recursive calls.
    recursive_calls: usize,
}

impl<'a> VariabilityZielonkaSolver<'a> {
    /// Creates a new VariabilityZielonkaSolver for the given game.
    pub fn new(manager_ref: &'a BDDManagerRef, game: &'a VariabilityParityGame, alternative_solving: bool) -> Self {
        // Keep track of the vertices for each priority
        let mut priority_vertices = Vec::new();

        for v in game.iter_vertices() {
            let prio = game.priority(v);

            while prio >= priority_vertices.len() {
                priority_vertices.push(Vec::new());
            }

            priority_vertices[prio].push(v);
        }

        Self {
            game,
            manager_ref,
            temp_queue: Vec::new(),
            temp_vertices: BitVec::repeat(false, game.num_of_vertices()),
            predecessors: VariabilityPredecessors::new(manager_ref, game),
            priority_vertices,
            recursive_calls: 0,
            alternative_solving,
        }
    }

    /// Solves the variability parity game for the given set of vertices V.
    fn solve_recursive(&mut self, gamma: Submap) -> Result<[Submap; 2], MercError> {
        self.recursive_calls += 1;

        // 1. if \gamma == \epsilon then
        if gamma.is_empty() {
            trace!("Empty subgame");
            return Ok([gamma.clone(), gamma]);
        }

        // 5. m := max { p(v) | v in V && \gamma(v) \neq \emptyset }
        let (highest_prio, lowest_prio) = self.get_highest_lowest_prio(&gamma);

        // 6. x := m mod 2
        let x = Player::from_priority(&highest_prio);
        let not_x = x.opponent();

        // 7. \mu := lambda v in V. bigcup { \gamma(v) | p(v) = m }
        let mut mu = Submap::new(
            self.manager_ref.with_manager_shared(|manager| BDDFunction::f(manager)),
            self.game.num_of_vertices(),
        );

        for v in &self.priority_vertices[*highest_prio] {
            mu.set(*v, gamma[*v].clone());
        }

        debug!("solve_rec(gamma) |gamma| = {}, highest prio = {}, lowest prio = {}, player = {}, |mu| = {}",
            gamma.mapping.iter().filter(|f| f.satisfiable()).count(),
            highest_prio,
            lowest_prio,
            x,
            mu.len()
        );

        let alpha = self.attractor(x, &gamma, mu)?;

        // 9. (omega'_0, omega'_1) := solve(\gamma \ \alpha)
        debug!("begin solve_rec(gamma \\ alpha)");
        let mut omega_prime = self.solve_recursive(gamma.clone().minus(&alpha)?)?;
        debug!("end solve_rec(gamma \\ alpha)");
        debug!("|omega'_0| = {}, |omega'_1| = {}",
            omega_prime[0].len(),
            omega_prime[1].len(),
        );

        if omega_prime[not_x.to_index()].is_empty() {
            // 11. omega_x := omega'_x \cup alpha
            omega_prime[x.to_index()] = gamma;
            omega_prime[not_x.to_index()].clear();
            // 20. return (omega_0, omega_1) 
            debug!("return (omega'_0, omega'_1)");
            return Ok(omega_prime)
        }

        // 14. \beta := attr_notalpha(\omega'_notalpha)
        let mut omega_prime_opponent = std::mem::take(&mut omega_prime[not_x.to_index()]);
        let beta = self.attractor(not_x, &gamma, omega_prime_opponent.clone())?;

        // 15. (omega''_0, omega''_1) := solve(gamma \ beta)
        debug!("begin solve_rec(gamma \\ beta)");
        let mut omega_double_prime = self.solve_recursive(gamma.minus(&beta)?)?;
        debug!("end solve_rec(gamma \\ beta)");

        // 17. omega_notx := omega'_notx \cup \beta
        omega_double_prime[not_x.to_index()] = omega_prime_opponent.or(&beta)?;

        // 20. return (omega_0, omega_1) 

        Ok(omega_double_prime)
    }

    /// Computes the attractor for `player` to the set `A` within the set of vertices `gamma`.
    fn attractor(&mut self, alpha: Player, gamma: &Submap, mut A: Submap) -> Result<Submap, MercError> {
        self.temp_queue.clear();

        // 2. Queue Q := {v \in V | U(v) != \emptset }
        for v in gamma.iter_vertices() {
            self.temp_queue.push(v);
        }

        /// 3. A := U

        // 4. While Q not empty do
        // 5. w := Q.pop()
        while let Some(w) = self.temp_queue.pop() {
            self.temp_vertices.set(*w, false);

            // For every v \in Ew do
            for (v, edge_guard) in self.predecessors.predecessors(w) {
                let mut a = gamma[v].and(&A[w])?.and(edge_guard)?;

                if a.satisfiable() {
                    // 7. if v in V_\alpha
                    if self.game.owner(v) == alpha {
                        // 8. a := gamma(v) \intersect \theta(v, w) \intersect A(w)
                        // This assignment has already been computed above.
                    } else {
                        // 10. a := gamma(v)
                        a = gamma[v].clone();
                        // 11. for w' \in vE such that gamma(v) && theta(v, w') && \gamma(w') != \emptyset do
                        for edge in self.game.outgoing_edges(v) {
                            let tmp = gamma[v].and(edge.configuration())?.and(&gamma[edge.to()])?;

                            if tmp.satisfiable() {
                                // 12. a := a && (C \ (theta(v, w') && \gamma(w'))) \cup A(w')
                                a = a.and(&self.game.configuration().min(edge.configuration()).and(&gamma[edge.to()])?)?.or(&A[edge.to()])?;
                            }
                        }
                    }
                    
                    // 15. a \ A(v) != \emptyset
                    if a.and(&A[v].not()?)?.satisfiable() {
                        // 16. A(v) := A(v) \cup a
                        A.set(v, A[v].or(&a)?);

                        // 17. if v not in Q then Q.push(v)
                        if !self.temp_vertices[*v] {
                            self.temp_queue.push(v);
                            self.temp_vertices.set(*v, true);
                        }
                    }
                }
            }
        }

        Ok(A)
    }

    /// Returns the highest and lowest priority in the given set of vertices V.
    fn get_highest_lowest_prio(&self, V: &Submap) -> (Priority, Priority) {
        let mut highest = usize::MIN;
        let mut lowest = usize::MAX;

        for v in V.iter_vertices() {
            let prio = self.game.priority(v);
            highest = highest.max(*prio);
            lowest = lowest.min(*prio);
        }

        (Priority::new(highest), Priority::new(lowest))
    }
}

/// A mapping from vertices to configurations.
#[derive(Clone, Default)]
pub struct Submap {
    /// The mapping from vertex indices to BDD functions.
    mapping: Vec<BDDFunction>,

    /// Invariant: counts the number of empty positions in the mapping.
    non_empty_count: usize,
}

impl Submap {
    /// Creates a new empty Submap for the given number of vertices.
    fn new(initial: BDDFunction, num_of_vertices: usize) -> Self {
        Self {
            mapping: vec![initial.clone(); num_of_vertices],
            non_empty_count: 0,
        }
    }

    /// Returns an iterator over the vertices in the submap that are non-empty.
    pub fn iter_vertices(&self) -> impl Iterator<Item = VertexIndex> + '_ {
        self.mapping.iter().enumerate().filter_map(|(i, func)| {
            if func.satisfiable() {
                None
            } else {
                Some(VertexIndex::new(i))
            }
        })
    }

    /// Sets the function for the given vertex index.
    fn set(&mut self, index: VertexIndex, func: BDDFunction) {
        let was_empty = !self.mapping[*index].satisfiable();
        let is_empty = !func.satisfiable();

        self.mapping[*index] = func;

        // Update the non-empty count invariant.
        if was_empty && !is_empty {
            self.non_empty_count += 1;
        } else if !was_empty && is_empty {
            self.non_empty_count -= 1;
        }
    }

    /// Returns true iff the submap is empty.
    fn is_empty(&self) -> bool {
        self.non_empty_count == 0
    }

    /// Returns the number of non-empty entries in the submap.
    fn len(&self) -> usize {
        self.non_empty_count
    }

    /// Clears the submap, setting all entries to the empty function.
    fn clear(&mut self) -> Result<(), MercError> {
        for func in self.mapping.iter_mut() {
            *func = func.nor(&func)?;
        }
        self.non_empty_count = 0;

        Ok(())
    }

    /// Computes the difference between this submap and another submap.
    fn minus(mut self, other: &Submap) -> Result<Submap, MercError> {
        for (i, func) in self.mapping.iter_mut().enumerate() {
            *func = func.and(&other.mapping[i].not()?)?;
        }

        Ok(self)
    }

    /// Computes the union between this submap and another submap.
    fn or(mut self, other: &Submap) -> Result<Submap, MercError> {
        for (i, func) in self.mapping.iter_mut().enumerate() {
            *func = func.or(&other.mapping[i])?;
        }

        Ok(self)
    }
}

impl Index<VertexIndex> for Submap {
    type Output = BDDFunction;

    fn index(&self, index: VertexIndex) -> &Self::Output {
        &self.mapping[*index]
    }
}

impl fmt::Debug for Submap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, func) in self.mapping.iter().enumerate() {
            writeln!(f, "  {}", i)?;
        }
        Ok(())
    }
}
