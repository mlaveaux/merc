#![allow(nonstandard_style)]
#![allow(unused)]
//! To keep with the theory, we use capitalized variable names for sets of vertices.
//! Authors: Maurice Laveaux, Sjef van Loo, Erik de Vink and Tim A.C. Willemse

use std::fmt;
use std::ops::Index;

use bitvec::order::Lsb0;
use bitvec::vec::BitVec;
use clap::ValueEnum;
use log::debug;
use log::trace;
use merc_utilities::MercError;
use oxidd::BooleanFunction;
use oxidd::ManagerRef;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;
use oxidd::util::AllocResult;

use crate::PG;
use crate::Player;
use crate::Priority;
use crate::VariabilityParityGame;
use crate::VariabilityPredecessors;
use crate::VertexIndex;

/// Variant of the Zielonka algorithm to use.
#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZielonkaVariant {
    /// Product-based Zielonka variant.
    Product,
    /// Standard Family-based Zielonka algorithm.
    Standard,
    /// Left-optimised Family-based Zielonka variant.
    OptimisedLeft,
}

/// Solves the given variability parity game using the specified Zielonka algorithm variant.
pub fn solve_variability_zielonka(
    manager_ref: &BDDManagerRef,
    game: &VariabilityParityGame,
    variant: ZielonkaVariant,
    alternative_solving: bool,
) -> Result<[Submap; 2], MercError> {
    debug_assert!(game.is_total(), "Zielonka solver requires a total parity game");

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

    let mut W = match variant {
        ZielonkaVariant::Standard => zielonka.solve_recursive(V)?,
        ZielonkaVariant::OptimisedLeft => zielonka.solve_optimised_left_recursive(V)?,
        ZielonkaVariant::Product => {
            panic!("Product-based Zielonka is implemented in solve_product_zielonka");
        }
    };

    debug!("Performed {} recursive calls", zielonka.recursive_calls);
    zielonka.check_partition(&W)?;

    if alternative_solving {
        // Intersect the results with the game's configuration
        let config = game.configuration();
        W[0].and_function(&config)?;
        W[1].and_function(&config)?;
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

    /// Keep track of the vertices in the temp_queue above in the attractor computation.
    temp_vertices: BitVec<usize, Lsb0>,

    /// Stores the predecessors of the game.
    predecessors: VariabilityPredecessors,

    /// Temporary storage for vertices per priority.
    priority_vertices: Vec<Vec<VertexIndex>>,

    /// The BDD function representing the empty configuration.
    false_bdd: BDDFunction,

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

        let false_bdd = manager_ref.with_manager_shared(|manager| BDDFunction::f(manager));

        Self {
            game,
            manager_ref,
            temp_queue: Vec::new(),
            temp_vertices: BitVec::repeat(false, game.num_of_vertices()),
            predecessors: VariabilityPredecessors::new(manager_ref, game),
            priority_vertices,
            recursive_calls: 0,
            alternative_solving,
            false_bdd,
        }
    }

    /// Solves the variability parity game for the given set of vertices V.
    fn solve_recursive(&mut self, gamma: Submap) -> Result<(Submap, Submap), MercError> {
        self.recursive_calls += 1;

        // 1. if \gamma == \epsilon then
        if gamma.is_empty() {
            trace!("Empty subgame");
            return Ok((gamma.clone(), gamma));
        }

        // 5. m := max { p(v) | v in V && \gamma(v) \neq \emptyset }
        let (highest_prio, lowest_prio) = self.get_highest_lowest_prio(&gamma);

        // 6. x := m mod 2
        let x = Player::from_priority(&highest_prio);
        let not_x = x.opponent();

        // 7. \mu := lambda v in V. bigcup { \gamma(v) | p(v) = m }
        let mut mu = Submap::new(
            self.manager_ref.with_manager_shared(|manager| BDDFunction::f(manager)),
            self.false_bdd.clone(),
            self.game.num_of_vertices(),
        );

        for v in &self.priority_vertices[*highest_prio] {
            mu.set(*v, gamma[*v].clone());
        }

        debug!(
            "solve_rec(gamma) |gamma| = {}, m = {}, l = {}, x = {}, |mu| = {}",
            gamma.mapping.iter().filter(|f| f.satisfiable()).count(),
            highest_prio,
            lowest_prio,
            x,
            mu.number_of_non_empty()
        );

        let alpha = self.attractor(x, &gamma, mu)?;

        // 9. (omega'_0, omega'_1) := solve(\gamma \ \alpha)
        debug!("begin solve_rec(gamma \\ alpha)");
        let (mut omega1_0, mut omega1_1) = self.solve_recursive(gamma.clone().minus(&alpha)?)?;
        debug!("end solve_rec(gamma \\ alpha)");
        debug!(
            "|omega'_0| = {}, |omega'_1| = {}",
            omega1_0.number_of_non_empty(),
            omega1_1.number_of_non_empty(),
        );

        if index(&mut omega1_0, &mut omega1_1, not_x).is_empty() {
            // 11. omega_x := omega'_x \cup alpha
            *index(&mut omega1_0, &mut omega1_1, x) = gamma;
            index(&mut omega1_0, &mut omega1_1, not_x).clear()?;
            // 20. return (omega_0, omega_1)
            debug!("return (omega'_0, omega'_1)");
            return Ok((omega1_0, omega1_1));
        }

        // 14. \beta := attr_notalpha(\omega'_notx)
        let omega_prime_opponent = match not_x {
            Player::Even => omega1_0,
            Player::Odd => omega1_1,
        };

        let beta = self.attractor(not_x, &gamma, omega_prime_opponent)?;

        // 15. (omega''_0, omega''_1) := solve(gamma \ beta)
        debug!("begin solve_rec(gamma \\ beta)");
        let (mut omega2_0, mut omega2_1) = self.solve_recursive(gamma.minus(&beta)?)?;
        debug!("end solve_rec(gamma \\ beta)");

        // 17. omega''_notx := omega''_notx \cup \beta
        let omega2_opponent = index(&mut omega2_0, &mut omega2_1, not_x);

        // Not completely optimal.
        *omega2_opponent = omega2_opponent.clone().or(&beta)?;

        // 20. return (omega_0, omega_1)
        debug!("return (omega''_0, omega''_1)");
        Ok((omega2_0, omega2_1))
    }

    /// Left-optimised Zielonka solver that has improved theoretical complexity, but might be slower in practice.
    fn solve_optimised_left_recursive(&mut self, gamma: Submap) -> Result<[Submap; 2], MercError> {
        self.recursive_calls += 1;
        let gamma_copy = gamma.clone();

        if gamma.is_empty() {
            debug!("empty subgame");
            return Ok([gamma.clone(), gamma]);
        }

        let (highest_prio, lowest_prio) = self.get_highest_lowest_prio(&gamma);
        let x = Player::from_priority(&highest_prio);
        let not_x = x.opponent();

        // mu and C from max-priority vertices
        let mut mu = Submap::new(
            self.manager_ref.with_manager_shared(|manager| BDDFunction::f(manager)),
            self.game.num_of_vertices(),
        );
        let mut C = self.manager_ref.with_manager_shared(|m| BDDFunction::f(m));
        for v in &self.priority_vertices[*highest_prio] {
            mu.set(*v, gamma[*v].clone());
            C = C.or(&gamma[*v])?;
        }

        debug!(
            "solve_optimised_left_rec(gamma) |gamma| = {}, m = {}, l = {}, x = {}, |mu| = {}",
            gamma.mapping.iter().filter(|f| f.satisfiable()).count(),
            highest_prio,
            lowest_prio,
            x,
            mu.number_of_non_empty()
        );

        // alpha := attr_x(mu)
        let alpha = self.attractor(x, &gamma, mu)?;

        // Solve on gamma \ alpha
        debug!("begin solve_optimised_left_rec(gamma \\ alpha)");
        let mut omega_prime = self.solve_optimised_left_recursive(gamma.clone().minus(&alpha)?)?;
        debug!("end solve_optimised_left_rec(gamma \\ alpha)");

        // Restrict opponent part to C
        let mut omega_prime_not_x_restricted = omega_prime[not_x.to_index()].clone();
        {
            let indices: Vec<_> = omega_prime_not_x_restricted.iter().map(|(v, _)| v).collect();
            for v in indices {
                let func = omega_prime_not_x_restricted[v].clone();
                let newf = func.and(&C)?;
                omega_prime_not_x_restricted.set(v, newf);
            }
        }

        if omega_prime_not_x_restricted.is_empty() {
            // Winner x gets alpha as well
            debug!("return (omega'_0, omega'_1)");
            let tmp = omega_prime[x.to_index()].clone().or(&alpha)?;
            omega_prime[x.to_index()] = tmp;
            self.check_partition(&omega_prime)?;
            return Ok(omega_prime);
        }

        // C' := { c in C | exists v: c in omega'_not_x(v) }
        let mut C_prime = self.manager_ref.with_manager_shared(|m| BDDFunction::f(m));
        for (v, func) in omega_prime[not_x.to_index()].iter() {
            C_prime = C_prime.or(func)?;
        }
        C_prime = C_prime.and(&C)?;

        // Restrict omega'_not_x to C'
        let mut omega_prime_not_x_restricted_prime = omega_prime[not_x.to_index()].clone();
        {
            let indices: Vec<_> = omega_prime_not_x_restricted_prime.iter().map(|(v, _)| v).collect();
            for v in indices {
                let func = omega_prime_not_x_restricted_prime[v].clone();
                let newf = func.and(&C_prime)?;
                omega_prime_not_x_restricted_prime.set(v, newf);
            }
        }

        // beta := attr_not_x(omega'_not_x | C')
        let alpha_prime = self.attractor(not_x, &gamma, omega_prime_not_x_restricted_prime)?;

        // Solve on (gamma | C') \ alpha'
        // First restrict gamma to C'
        let mut gamma_restricted = gamma.clone();
        {
            let indices: Vec<_> = gamma_restricted.iter().map(|(v, _)| v).collect();
            for v in indices {
                let func = gamma_restricted[v].clone();
                let newf = func.and(&C_prime)?;
                gamma_restricted.set(v, newf);
            }
        }
        debug!("begin solve_optimised_left_rec((gamma | C') \\ alpha')");
        let omega_doubleprime = self.solve_optimised_left_recursive(gamma_restricted.minus(&alpha_prime)?)?;
        debug!("end solve_optimised_left_rec((gamma | C') \\ alpha')");

        // Compose final sets
        let mut omega_x = omega_prime[x.to_index()].clone();
        {
            let cp_not = C_prime.not()?;
            let indices: Vec<_> = omega_x.iter().map(|(v, _)| v).collect();
            for v in indices {
                let func = omega_x[v].clone();
                let newf = func.and(&cp_not)?;
                omega_x.set(v, newf);
            }
        }
        let mut omega_notx = omega_prime[not_x.to_index()].clone();
        {
            let cp_not = C_prime.not()?;
            let indices: Vec<_> = omega_notx.iter().map(|(v, _)| v).collect();
            for v in indices {
                let func = omega_notx[v].clone();
                let newf = func.and(&cp_not)?;
                omega_notx.set(v, newf);
            }
        }

        let mut result = omega_doubleprime;
        {
            let tmp = result[x.to_index()].clone().or(&omega_x)?;
            result[x.to_index()] = tmp;
        }
        // alpha minus C'
        let mut alpha_no_Cp = alpha.clone();
        {
            let cp_not = C_prime.not()?;
            let indices: Vec<_> = alpha_no_Cp.iter().map(|(v, _)| v).collect();
            for v in indices {
                let func = alpha_no_Cp[v].clone();
                let newf = func.and(&cp_not)?;
                alpha_no_Cp.set(v, newf);
            }
        }
        {
            let tmp = result[x.to_index()].clone().or(&alpha_no_Cp)?;
            result[x.to_index()] = tmp;
        }
        {
            let tmp = result[not_x.to_index()].clone().or(&omega_notx)?;
            result[not_x.to_index()] = tmp;
        }
        {
            let tmp = result[not_x.to_index()].clone().or(&alpha_prime)?;
            result[not_x.to_index()] = tmp;
        }

        debug!("return (omega''_0, omega''_1)");
        self.check_partition(&result)?;
        Ok(result)
    }

    /// Computes the attractor for `player` to the set `A` within the set of vertices `gamma`.
    fn attractor(&mut self, alpha: Player, gamma: &Submap, mut A: Submap) -> Result<Submap, MercError> {
        // 2. Queue Q := {v \in V | A(v) != \emptyset }
        self.temp_vertices.fill(false);
        for v in A.iter_vertices() {
            self.temp_queue.push(v);
            self.temp_vertices.set(*v, true);
        }

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
                        for edge in self.game.outgoing_conf_edges(v) {
                            let tmp = gamma[v].and(edge.configuration())?.and(&gamma[edge.to()])?;

                            if tmp.satisfiable() {
                                // 12. a := a && ((C \ (theta(v, w') && \gamma(w'))) \cup A(w'))
                                let tmp = edge.configuration().and(&gamma[edge.to()])?;

                                a = a.and(&minus(self.game.configuration(), &tmp)?.or(&A[edge.to()])?)?;
                            }
                        }
                    }

                    // 15. a \ A(v) != \emptyset
                    if minus(&a, &A[v])?.satisfiable() {
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

        debug_assert!(
            !self.temp_vertices.any(),
            "temp_vertices should be empty after attractor computation"
        );

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

    /// Checks that the given partition is valid.
    fn check_partition(&self, W: &[Submap; 2]) -> Result<(), MercError> {
        // Check that the result is a valid partition
        if cfg!(debug_assertions) {
            for v in self.game.iter_vertices() {
                let tmp = W[0][v].or(&W[1][v])?;

                // The union of both solutions should be the entire set of vertices.
                debug_assert!(
                    tmp == self.manager_ref.with_manager_shared(|manager| {
                        if self.alternative_solving {
                            BDDFunction::t(manager)
                        } else {
                            self.game.configuration().clone()
                        }
                    }),
                    "The union of both solutions should be the entire set of vertices, but vertex {v} is missing."
                );

                debug_assert!(
                    !W[0][v].and(&W[1][v])?.satisfiable(),
                    "The intersection of both solutions should be empty, but vertex {v} has non-empty intersection."
                );
            }
        }

        Ok(())
    }
}

/// Returns the boolean set difference of two BDD functions: lhs \ rhs.
/// Implemented as lhs AND (NOT rhs).
pub fn minus(lhs: &BDDFunction, rhs: &BDDFunction) -> AllocResult<BDDFunction> {
    lhs.and(&rhs.not()?)
}

/// A mapping from vertices to configurations.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Submap {
    /// The mapping from vertex indices to BDD functions.
    mapping: Vec<BDDFunction>,

    /// Invariant: counts the number of non-empty positions in the mapping.
    non_empty_count: usize,

    /// The BDD function representing the empty configuration.
    false_bdd: BDDFunction,
}

impl Submap {
    /// Creates a new empty Submap for the given number of vertices.
    fn new(initial: BDDFunction, false_bdd: BDDFunction, num_of_vertices: usize) -> Self {
        Self {
            mapping: vec![initial.clone(); num_of_vertices],
            false_bdd,
            non_empty_count: if initial.satisfiable() {
                num_of_vertices // If the initial function is satisfiable, all entries are non-empty.
            } else {
                0
            },
        }
    }

    /// Returns an iterator over the vertices in the submap whose configuration is satisfiable.
    pub fn iter_vertices(&self) -> impl Iterator<Item = VertexIndex> + '_ {
        self.mapping.iter().enumerate().filter_map(|(i, func)| {
            if func.satisfiable() {
                Some(VertexIndex::new(i))
            } else {
                None
            }
        })
    }

    /// Returns the number of non-empty entries in the submap.
    pub fn number_of_non_empty(&self) -> usize {
        self.non_empty_count
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

    /// Returns the number of entries in the submap.
    fn len(&self) -> usize {
        self.mapping.len()
    }

    /// Clears the submap, setting all entries to the empty function.
    fn clear(&mut self) -> Result<(), MercError> {
        for func in self.mapping.iter_mut() {
            *func = self.false_bdd.clone();
        }
        self.non_empty_count = 0;

        Ok(())
    }

    /// Computes the difference between this submap and another submap.
    fn minus(mut self, other: &Submap) -> Result<Submap, MercError> {
        for (i, func) in self.mapping.iter_mut().enumerate() {
            let was_satisfiable = func.satisfiable();
            *func = minus(func, &other.mapping[i])?;
            let is_satisfiable = func.satisfiable();

            if was_satisfiable && !is_satisfiable {
                self.non_empty_count -= 1;
            }
        }

        Ok(self)
    }

    /// Computes the union between this submap and another submap.
    fn or(mut self, other: &Submap) -> Result<Submap, MercError> {
        for (i, func) in self.mapping.iter_mut().enumerate() {
            let was_satisfiable = func.satisfiable();
            *func = func.or(&other.mapping[i])?;
            let is_satisfiable = func.satisfiable();

            if !was_satisfiable && is_satisfiable {
                self.non_empty_count += 1;
            }
        }

        Ok(self)
    }

    /// Computes the intersection between this submap and another function.
    fn and_function(&mut self, configuration: &BDDFunction) -> Result<(), MercError> {
        for (i, func) in self.mapping.iter_mut().enumerate() {
            let was_satisfiable = func.satisfiable();
            *func = func.and(&configuration)?;
            let is_satisfiable = func.satisfiable();

            if was_satisfiable && !is_satisfiable {
                self.non_empty_count -= 1;
            }
        }

        Ok(())
    }

    /// Computes the difference between this submap and another function.
    fn minus_function(&mut self, configuration: &BDDFunction) -> Result<(), MercError> {
        for (i, func) in self.mapping.iter_mut().enumerate() {
            let was_satisfiable = func.satisfiable();
            *func = minus(func, &configuration)?;
            let is_satisfiable = func.satisfiable();

            if was_satisfiable && !is_satisfiable {
                self.non_empty_count -= 1;
            }
        }

        Ok(())
    }

    /// Returns an iterator over all entries.
    pub fn iter(&self) -> impl Iterator<Item = (VertexIndex, &BDDFunction)> {
        self.mapping
            .iter()
            .enumerate()
            .map(|(i, func)| (VertexIndex::new(i), func))
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

#[cfg(test)]
mod tests {
    use merc_macros::merc_test;
    use oxidd::bdd::BDDFunction;
    use oxidd::util::AllocResult;
    use oxidd::BooleanFunction;
    use oxidd::Manager;
    use oxidd::ManagerRef;

    use merc_utilities::random_test;

    use crate::FormatConfig;
    use crate::project_variability_parity_games_iter;
    use crate::random_variability_parity_game;
    use crate::solve_variability_product_zielonka;
    use crate::solve_variability_zielonka;
    use crate::solve_zielonka;
    use crate::VertexIndex;
    use crate::ZielonkaVariant;
    use crate::PG;

    #[merc_test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_submap() {
        let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);
        let vars: Vec<BDDFunction> = manager_ref
            .with_manager_exclusive(|manager| {
                AllocResult::from_iter(manager.add_vars(3).map(|i| BDDFunction::var(manager, i)))
            })
            .expect("Could not create variables");

        let false_bdd = manager_ref.with_manager_shared(|manager| BDDFunction::f(manager));
        let mut submap = super::Submap::new(false_bdd.clone(), false_bdd, 3);

        assert_eq!(submap.len(), 3);
        assert_eq!(submap.non_empty_count, 0);
        submap.set(VertexIndex::new(1), vars[0].clone());

        assert_eq!(submap.non_empty_count, 1);
    }

    // #[merc_test]
    // #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    // fn test_random_variability_parity_game_solve() {
    //     random_test(100, |rng| {
    //         let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);
    //         let vpg = random_variability_parity_game(&manager_ref, rng, true, 20, 3, 3, 3).unwrap();
    //         println!("Solving VPG {}", vpg);

    //         crate::write_vpg(&mut std::io::stdout(), &vpg).unwrap();

    //         let solution = solve_variability_zielonka(&manager_ref, &vpg, ZielonkaVariant::Standard, false).unwrap();

    //         for game in project_variability_parity_games_iter(&vpg) {
    //             let (cube, pg) = game.unwrap();
    //             let pg_solution = solve_zielonka(&pg);

    //             for v in pg.iter_vertices() {
    //                 if pg_solution[0].get(*v).is_some() {
    //                     // Won by Even
    //                     debug_assert!(solution[0][v].and(&cube).unwrap().satisfiable());
    //                 }
    //             }
    //         }
    //     })
    // }

    // #[merc_test]
    // #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    // fn test_random_variability_parity_game_solve_optimised_left() {
    //     random_test(100, |rng| {
    //         let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);
    //         let vpg = random_variability_parity_game(&manager_ref, rng, true, 20, 3, 3, 3).unwrap();

    //         let solution = solve_variability_zielonka(&manager_ref, &vpg, ZielonkaVariant::OptimisedLeft, false).unwrap();
    //         let solution_expected = solve_variability_zielonka(&manager_ref, &vpg, ZielonkaVariant::Standard, false).unwrap();

    //         debug_assert_eq!(solution[0], solution_expected[0]);
    //         debug_assert_eq!(solution[1], solution_expected[1]);
    //     })
    // }
}
