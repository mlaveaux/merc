//! To keep with the theory, we use capitalized variable names for sets of vertices.
//! Authors: Maurice Laveaux, Sjef van Loo, Erik de Vink and Tim A.C. Willemse
#![allow(nonstandard_style)]
#![allow(unused)]

use std::fmt;
use std::ops::Index;

use bitvec::bitvec;
use bitvec::order::Lsb0;
use bitvec::vec::BitVec;
use clap::ValueEnum;
use log::debug;
use log::info;
use log::trace;
use merc_symbolic::FormatConfig;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;
use oxidd::util::AllocResult;
use oxidd::util::OptBool;
use oxidd::BooleanFunction;
use oxidd::Edge;
use oxidd::Function;
use oxidd::Manager;
use oxidd::ManagerRef;
use oxidd_core::util::EdgeDropGuard;

use merc_symbolic::minus;
use merc_symbolic::minus_edge;
use merc_symbolic::FormatConfigSet;
use merc_utilities::MercError;
use merc_utilities::Timing;

use crate::combine;
use crate::compute_reachable;
use crate::project_variability_parity_games_iter;
use crate::solve_zielonka;
use crate::x_and_not_x;
use crate::Player;
use crate::Priority;
use crate::Repeat;
use crate::Set;
use crate::Submap;
use crate::VariabilityParityGame;
use crate::VariabilityPredecessors;
use crate::VertexIndex;
use crate::PG;

/// Variant of the Zielonka algorithm to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum ZielonkaVariant {
    /// Product-based Zielonka variant.
    Product,
    /// Standard family-based Zielonka algorithm.
    Family,
    /// Left-optimised family-based Zielonka variant.
    FamilyOptimisedLeft,
}

/// Solves the given variability parity game using the specified Zielonka algorithm variant.
pub fn solve_variability_zielonka(
    manager_ref: &BDDManagerRef,
    game: &VariabilityParityGame,
    variant: ZielonkaVariant,
    alternative_solving: bool,
) -> Result<[Submap; 2], MercError> {
    debug_assert!(
        game.is_total(manager_ref)?,
        "Zielonka solver requires a total parity game"
    );

    let mut zielonka = VariabilityZielonkaSolver::new(manager_ref, game, alternative_solving);

    // Determine the initial set of vertices V
    let V = Submap::new(
        manager_ref,
        if alternative_solving {
            manager_ref.with_manager_shared(|manager| BDDFunction::t(manager))
        } else {
            game.configuration().clone()
        },
        game.num_of_vertices(),
    );

    let full_V = V.clone();
    let (W0, W1) = match variant {
        ZielonkaVariant::Family => zielonka.solve_recursive(V, 0)?,
        ZielonkaVariant::FamilyOptimisedLeft => zielonka.zielonka_family_optimised(V, 0)?,
        ZielonkaVariant::Product => {
            panic!("Product-based Zielonka is implemented in solve_product_zielonka");
        }
    };

    debug!("Performed {} recursive calls", zielonka.recursive_calls);
    if cfg!(debug_assertions) {
        zielonka.check_partition(&W0, &W1, &full_V)?;
    }

    let (W0, W1) = if alternative_solving {
        // Intersect the results with the game's configuration
        let config = game.configuration();
        (
            W0.and_function(manager_ref, config)?,
            W1.and_function(manager_ref, config)?,
        )
    } else {
        (W0, W1)
    };

    Ok([W0, W1])
}

/// Solves the given variability parity game using the product-based Zielonka algorithm.
pub fn solve_variability_product_zielonka<'a>(
    vpg: &'a VariabilityParityGame,
    timing: &'a Timing,
) -> impl Iterator<Item = Result<(Vec<OptBool>, BDDFunction, [Set; 2]), MercError>> + 'a {
    project_variability_parity_games_iter(vpg, timing).map(|result| {
        match result {
            Ok(((cube, bdd, pg), timing)) => {
                let mut reachable_time = timing.start("reachable");
                let (reachable_pg, projection) = compute_reachable(&pg);
                reachable_time.finish();

                debug!("Solving projection on {}...", FormatConfig(&cube));

                let pg_solution = solve_zielonka(&reachable_pg);
                let mut new_solution = [
                    bitvec![usize, Lsb0; 0; vpg.num_of_vertices()],
                    bitvec![usize, Lsb0; 0; vpg.num_of_vertices()],
                ];
                for v in pg.iter_vertices() {
                    if let Some(proj_v) = projection[*v] {
                        // Vertex is reachable in the projection, set its solution
                        if pg_solution[0][proj_v] {
                            new_solution[0].set(*v, true);
                        }
                        if pg_solution[1][proj_v] {
                            new_solution[1].set(*v, true);
                        }
                    }
                }

                Ok((cube, bdd, new_solution))
            }
            Err(result) => Err(result),
        }
    })
}

/// Verifies that the solution obtained from the variability product-based Zielonka solver
/// is consistent with the solution of the variability parity game.
pub fn verify_variability_product_zielonka_solution(
    vpg: &VariabilityParityGame,
    solution: &[Submap; 2],
    timing: &Timing,
) -> Result<(), MercError> {
    info!("Verifying variability product-based Zielonka solution...");
    solve_variability_product_zielonka(vpg, timing).try_for_each(|res| {
        match res {
            Ok((bits, cube, pg_solution)) => {
                for v in vpg.iter_vertices() {
                    if pg_solution[0][*v] {
                        // Won by Even
                        assert!(
                            solution[0][v].and(&cube)?.satisfiable(),
                            "Projection {}, vertex {v} is won by even in the product, but not in the vpg",
                            FormatConfig(&bits)
                        );
                    }

                    if pg_solution[1][*v] {
                        // Won by Odd
                        assert!(
                            solution[1][v].and(&cube)?.satisfiable(),
                            "Projection {}, vertex {v} is won by odd in the product, but not in the vpg",
                            FormatConfig(&bits)
                        );
                    }
                }

                Ok(())
            }
            Err(res) => Err(res),
        }
    })?;

    Ok(())
}

struct VariabilityZielonkaSolver<'a> {
    game: &'a VariabilityParityGame,

    manager_ref: &'a BDDManagerRef,

    /// Instead of solving the game only for the valid configurations, solve for
    /// all configurations and then restrict the result at the end.
    alternative_solving: bool,

    /// Reused temporary queue for attractor computation.
    temp_queue: Vec<VertexIndex>,

    /// Keep track of the vertices in the temp_queue above in the attractor computation.
    temp_vertices: BitVec<usize, Lsb0>,

    /// Stores the predecessors of the game.
    predecessors: VariabilityPredecessors,

    /// Temporary storage for vertices per priority.
    priority_vertices: Vec<Vec<VertexIndex>>,

    /// The BDD function representing the universe configuration.
    true_bdd: BDDFunction,

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

        let true_bdd = manager_ref.with_manager_shared(|manager| BDDFunction::t(manager));
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
            true_bdd,
            false_bdd,
        }
    }

    /// Solves the variability parity game for the given set of vertices V.
    fn solve_recursive(&mut self, gamma: Submap, depth: usize) -> Result<(Submap, Submap), MercError> {
        self.recursive_calls += 1;

        // For debugging mostly
        let indent = Repeat::new(" ", depth);
        let gamma_copy = gamma.clone();

        // 1. if \gamma == \epsilon then
        if gamma.is_empty() {
            return Ok((gamma.clone(), gamma));
        }

        // 5. m := max { p(v) | v in V && \gamma(v) \neq \emptyset }
        let (highest_prio, lowest_prio) = self.get_highest_lowest_prio(&gamma);

        // 6. x := m mod 2
        let x = Player::from_priority(&highest_prio);
        let not_x = x.opponent();

        // 7. \mu := lambda v in V. bigcup { \gamma(v) | p(v) = m }
        let mut mu = Submap::new(self.manager_ref, self.false_bdd.clone(), self.game.num_of_vertices());

        self.manager_ref
            .with_manager_shared(|manager| -> Result<(), MercError> {
                for v in &self.priority_vertices[*highest_prio] {
                    mu.set(manager, *v, gamma[*v].clone());
                }

                Ok(())
            })?;

        debug!(
            "|gamma| = {}, m = {}, l = {}, x = {}, |mu| = {}",
            gamma.number_of_non_empty(),
            highest_prio,
            lowest_prio,
            x,
            mu.number_of_non_empty()
        );

        trace!("{indent}Vertices in gamma: {:?}", gamma);
        trace!("{indent}Vertices in mu: {:?}", mu);
        let alpha = self.attractor(x, &gamma, mu)?;
        trace!("{indent}Vertices in alpha: {:?}", alpha);

        // 9. (omega'_0, omega'_1) := solve(\gamma \ \alpha)
        debug!(
            "{indent}zielonka_family(gamma \\ alpha), |alpha| = {}",
            alpha.number_of_non_empty()
        );
        let (omega1_0, omega1_1) = self.solve_recursive(
            gamma
                .clone()
                .minus(self.manager_ref, &alpha)?,
            depth + 1,
        )?;

        let (mut omega1_x, mut omega1_not_x) = x_and_not_x(omega1_0, omega1_1, x);
        if omega1_not_x.is_empty() {
            // 11. omega_x := omega'_x \cup alpha
            omega1_x = omega1_x.or(self.manager_ref, &alpha)?;
            // 20. return (omega_0, omega_1)
            Ok(combine(omega1_x, omega1_not_x, x))
        } else {
            // 14. \beta := attr_notalpha(\omega'_notx)
            let beta = self.attractor(not_x, &gamma, omega1_not_x)?;
            // 15. (omega''_0, omega''_1) := solve(gamma \ beta)
            debug!(
                "{indent}solve_rec(gamma \\ beta), |beta| = {}",
                beta.number_of_non_empty()
            );
            trace!("{indent}Vertices in beta: {:?}", beta);

            let (mut omega2_0, mut omega2_1) =
                self.solve_recursive(gamma.minus(self.manager_ref, &beta)?, depth + 1)?;

            // 17. omega''_notx := omega''_notx \cup \beta
            let (omega2_x, mut omega2_not_x) = x_and_not_x(omega2_0, omega2_1, x);
            omega2_not_x = omega2_not_x.or(self.manager_ref, &beta)?;

            // 20. return (omega_0, omega_1)
            if cfg!(debug_assertions) {
                self.check_partition(&omega2_x, &omega2_not_x, &gamma_copy)?;
            }
            Ok(combine(omega2_x, omega2_not_x, x))
        }
    }

    /// Left-optimised Zielonka solver that has improved theoretical complexity, but might be slower in practice.
    fn zielonka_family_optimised(&mut self, gamma: Submap, depth: usize) -> Result<(Submap, Submap), MercError> {
        self.recursive_calls += 1;
        let indent = Repeat::new(" ", depth);
        let gamma_copy = gamma.clone();

        // 1. if \gamma == \epsilon then
        if gamma.is_empty() {
            // 2. return (\epsilon, \epsilon)
            return Ok((gamma.clone(), gamma));
        }

        // 5. m := max { p(v) | v in V && \gamma(v) \neq \emptyset }
        let (highest_prio, lowest_prio) = self.get_highest_lowest_prio(&gamma);

        // 6. x := m mod 2
        let x = Player::from_priority(&highest_prio);
        let not_x = x.opponent();

        // 7. C := { c in \bigC | exists v in V : p(v) = m && c in \gamma(v) }
        // 8. \mu := lambda v in V. bigcup { \gamma(v) | p(v) = m }
        let mut mu = Submap::new(self.manager_ref, self.false_bdd.clone(), self.game.num_of_vertices());

        let mut C = self.false_bdd.clone();

        self.manager_ref
            .with_manager_shared(|manager| -> Result<(), MercError> {
                for v in &self.priority_vertices[*highest_prio] {
                    mu.set(manager, *v, gamma[*v].clone());
                    C = C.or(&gamma[*v])?;
                }

                Ok(())
            })?;

        debug!(
            "{indent}|gamma| = {}, m = {}, l = {}, x = {}, |mu| = {}",
            gamma.number_of_non_empty(),
            highest_prio,
            lowest_prio,
            x,
            mu.number_of_non_empty()
        );

        // 9. alpha := attr_x(\mu).
        trace!("{indent}gamma: {:?}", gamma);
        trace!("{indent}C: {}", FormatConfigSet(&C));
        let alpha = self.attractor(x, &gamma, mu)?;
        trace!("{indent}alpha: {:?}", alpha);

        // 10. (omega'_0, omega'_1) := solve(gamma \ alpha)
        debug!(
            "{indent}zielonka_family_opt(gamma \\ alpha) |alpha| = {}",
            alpha.number_of_non_empty()
        );
        let (omega1_0, omega1_1) = self.zielonka_family_optimised(
            gamma
                .clone()
                .minus(self.manager_ref, &alpha)?,
            depth + 1,
        )?;

        // omega_prime[not_x] restricted to (gamma \ C)
        let C_restricted = minus(
            &if !self.alternative_solving {
                self.true_bdd.clone()
            } else {
                self.game.configuration().clone()
            },
            &C,
        )?;

        let (mut omega1_x, omega1_not_x) = x_and_not_x(omega1_0, omega1_1, x);
        let omega1_not_x_restricted = omega1_not_x
            .clone()
            .minus_function(self.manager_ref, &C_restricted)?;

        // 10.
        if omega1_not_x_restricted.is_empty() {
            // 11. omega'_x := omega'_x \cup A
            omega1_x = omega1_x.or(self.manager_ref, &alpha)?;
            if cfg!(debug_assertions) {
                self.check_partition(&omega1_x, &omega1_not_x, &gamma_copy)?;
            }

            // 22. return (omega_0, omega_1)
            Ok(combine(omega1_x, omega1_not_x, x))
        } else {
            // C' := { c in C | exists v: c in omega'_not_x(v) }
            let mut C1 = self.false_bdd.clone();
            for (_v, func) in omega1_not_x.iter() {
                C1 = C1.or(func)?;
            }
            C1 = C1.and(&C)?;

            // beta := attr_not_x(omega'_not_x | C')
            let C1_restricted = minus(
                &if self.alternative_solving {
                    self.true_bdd.clone()
                } else {
                    self.game.configuration().clone()
                },
                &C1,
            )?;

            let omega1_not_x_restricted1 = omega1_not_x
                .clone()
                .minus_function(self.manager_ref, &C1_restricted)?;
            trace!("{indent}omega'_notx_restricted: {:?}", omega1_not_x_restricted1);
            let alpha1 = self.attractor(not_x, &gamma, omega1_not_x_restricted1)?;
            trace!("{indent}alpha': {:?}", alpha1);

            // Solve on (gamma | C') \ alpha'
            let gamma_restricted = gamma.minus_function(self.manager_ref, &C1_restricted)?;

            debug!("{indent}zielonka_family_opt((gamma | C') \\ alpha')");
            let (omega2_0, omega2_1) =
                self.zielonka_family_optimised(gamma_restricted.minus(self.manager_ref, &alpha1)?, depth + 1)?;

            // 18. omega'_x := omega'_x\C' cup alpha\C' cup omega''_x
            // 19. omega_not_x := omega'_not_x\C' cup omega''_x cup beta
            let (omega2_x, omega2_not_x) = x_and_not_x(omega2_0, omega2_1, x);
            let omega1_x_restricted = omega1_x.minus_function(self.manager_ref, &C1)?;
            let omega1_not_x_restricted = omega1_not_x.minus_function(self.manager_ref, &C1)?;

            let alpha_restricted = alpha.minus_function(self.manager_ref, &C1)?;
            let omega2_x_result = omega2_x.or(
                self.manager_ref,
                &omega1_x_restricted.or(self.manager_ref, &alpha_restricted)?,
            )?;
            let omega2_not_x_result = omega2_not_x
                .or(self.manager_ref, &omega1_not_x_restricted)?
                .or(self.manager_ref, &alpha1)?;

            debug!("{indent}return (omega''_0, omega''_1)");
            Ok(combine(omega2_x_result, omega2_not_x_result, x))
        }
    }

    /// Computes the attractor for `player` to the set `A` within the set of vertices `gamma`.
    ///
    /// # Details
    ///
    /// The definition of the attractor is as follows:
    ///     Attrx,γ (β) = intersection { α ⊆ γ | ∀v ∈ V, c ∈ C: (c ∈ β(v) ⇒ c ∈ α(v)) ∧
    ///          (v ∈ Vx ∧ (∃w ∈ V : v c −→ γ w ∧ c ∈ α(w)) ⇒ c ∈ α(v)) ∧
    ///          (v ∈ V¯x ∧ (∀w ∈ V : v c −→ γ w ⇒ c ∈ α(w)) ⇒ c ∈ α(v)) }
    ///
    /// The relation to the implementation is not entirely straightforward. The player `x` is called alpha here, and A is the beta set.
    fn attractor(&mut self, alpha: Player, gamma: &Submap, mut A: Submap) -> Result<Submap, MercError> {
        // 2. Queue Q := {v \in V | A(v) != \emptyset }
        debug_assert!(
            self.temp_queue.is_empty(),
            "temp_queue should be empty at the start of attractor computation"
        );

        self.manager_ref.with_manager_shared(|manager| {
            for v in A.iter_vertices(manager) {
                self.temp_queue.push(v);

                // temp_vertices keeps track of which vertices are in the queue.
                self.temp_vertices.set(*v, true);
            }
        });

        // 4. While Q not empty do
        // 5. w := Q.pop()
        self.manager_ref
            .with_manager_shared(|manager| -> Result<(), MercError> {
                // Used for satisfiability checks
                let f_edge = EdgeDropGuard::new(manager, BDDFunction::f_edge(manager));

                while let Some(w) = self.temp_queue.pop() {
                    self.temp_vertices.set(*w, false);

                    // For every v \in Ew do
                    for (v, edge_guard) in self.predecessors.predecessors(w) {
                        let mut a = EdgeDropGuard::new(
                            manager,
                            BDDFunction::and_edge(
                                manager,
                                &EdgeDropGuard::new(
                                    manager,
                                    BDDFunction::and_edge(manager, gamma[v].as_edge(manager), A[w].as_edge(manager))?,
                                ),
                                edge_guard.as_edge(manager),
                            )?,
                        );

                        if *a != *f_edge {
                            // 7. if v in V_\alpha
                            if self.game.owner(v) == alpha {
                                // 8. a := gamma(v) \intersect \theta(v, w) \intersect A(w)
                                // This assignment has already been computed above.
                            } else {
                                // 10. a := gamma(v)
                                a = EdgeDropGuard::new(manager, gamma[v].clone().into_edge(manager));
                                // 11. for w' \in vE such that gamma(v) && theta(v, w') && \gamma(w') != \emptyset do
                                for edge_w1 in self.game.outgoing_conf_edges(v) {
                                    let tmp = EdgeDropGuard::new(
                                        manager,
                                        BDDFunction::and_edge(
                                            manager,
                                            &EdgeDropGuard::new(
                                                manager,
                                                BDDFunction::and_edge(
                                                    manager,
                                                    gamma[v].as_edge(manager),
                                                    edge_w1.configuration().as_edge(manager),
                                                )?,
                                            ),
                                            &gamma[edge_w1.to()].as_edge(manager),
                                        )?,
                                    );

                                    if *tmp != *f_edge {
                                        // 12. a := a && ((C \ (theta(v, w') && \gamma(w'))) \cup A(w'))
                                        let tmp = EdgeDropGuard::new(
                                            manager,
                                            BDDFunction::and_edge(
                                                manager,
                                                edge_w1.configuration().as_edge(manager),
                                                &gamma[edge_w1.to()].as_edge(manager),
                                            )?,
                                        );

                                        a = EdgeDropGuard::new(
                                            manager,
                                            BDDFunction::and_edge(
                                                manager,
                                                &a,
                                                &EdgeDropGuard::new(
                                                    manager,
                                                    BDDFunction::or_edge(
                                                        manager,
                                                        &EdgeDropGuard::new(
                                                            manager,
                                                            minus_edge(
                                                                manager,
                                                                if self.alternative_solving {
                                                                    self.true_bdd.as_edge(manager)
                                                                } else {
                                                                    self.game.configuration().as_edge(manager)
                                                                },
                                                                &tmp,
                                                            )?,
                                                        ),
                                                        A[edge_w1.to()].as_edge(manager),
                                                    )?,
                                                ),
                                            )?,
                                        );
                                    }
                                }
                            }

                            // 15. a \ A(v) != \emptyset
                            if *EdgeDropGuard::new(manager, minus_edge(manager, &a, A[v].as_edge(manager))?) != *f_edge
                            {
                                // 16. A(v) := A(v) \cup a
                                let was_empty = *A[v].as_edge(manager) == *f_edge;
                                let update = BDDFunction::or_edge(manager, A[v].as_edge(manager), &a)?;
                                let is_empty = update == *f_edge;

                                A.set(manager, v, BDDFunction::from_edge(manager, update));

                                // 17. if v not in Q then Q.push(v)
                                if !self.temp_vertices[*v] {
                                    self.temp_queue.push(v);
                                    self.temp_vertices.set(*v, true);
                                }
                            }
                        }
                    }
                }

                Ok(())
            })?;

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

        self.manager_ref.with_manager_shared(|manager| {
            for v in V.iter_vertices(manager) {
                let prio = self.game.priority(v);
                highest = highest.max(*prio);
                lowest = lowest.min(*prio);
            }
        });
        
        (Priority::new(highest), Priority::new(lowest))
    }

    /// Checks that the sets W0 and W1 form a  partition w.r.t the submap V, i.e., their union is V and their intersection is empty.
    fn check_partition(&self, W0: &Submap, W1: &Submap, V: &Submap) -> Result<(), MercError> {
        self.manager_ref.with_manager_shared(|manager| -> Result<(), MercError> {
            for v in V.iter_vertices(manager) {
                let tmp = W0[v].or(&W1[v])?;

                // The union of both solutions should be the entire set of vertices.
                assert!(
                    tmp == V[v],
                    "The union of both solutions should be the entire set of vertices, but vertex {v} is missing."
                );

                assert!(
                    !W0[v].and(&W1[v])?.satisfiable(),
                    "The intersection of both solutions should be empty, but vertex {v} has non-empty intersection."
                );
            }

            Ok(())
        })?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use merc_io::DumpFiles;
    use merc_macros::merc_test;
    use merc_utilities::Timing;
    use oxidd::bdd::BDDFunction;
    use oxidd::util::AllocResult;
    use oxidd::BooleanFunction;
    use oxidd::Manager;
    use oxidd::ManagerRef;

    use merc_utilities::random_test;

    use crate::project_variability_parity_games_iter;
    use crate::random_variability_parity_game;
    use crate::solve_variability_product_zielonka;
    use crate::solve_variability_zielonka;
    use crate::solve_zielonka;
    use crate::verify_variability_product_zielonka_solution;
    use crate::write_vpg;
    use crate::Submap;
    use crate::VertexIndex;
    use crate::ZielonkaVariant;
    use crate::PG;

    #[merc_test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_variability_parity_game_solve() {
        random_test(100, |rng| {
            let mut files = DumpFiles::new("test_random_variability_parity_game_solve");

            let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);
            let vpg = random_variability_parity_game(&manager_ref, rng, true, 20, 3, 3, 3).unwrap();

            files.dump("input.vpg", |w| write_vpg(w, &vpg)).unwrap();

            let solution = solve_variability_zielonka(&manager_ref, &vpg, ZielonkaVariant::Family, false).unwrap();
            verify_variability_product_zielonka_solution(&vpg, &solution, &Timing::new()).unwrap();
        })
    }

    #[merc_test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_variability_parity_game_solve_optimised_left() {
        random_test(100, |rng| {
            let mut files = DumpFiles::new("test_random_variability_parity_game_solve_optimised_left");

            let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);
            let vpg = random_variability_parity_game(&manager_ref, rng, true, 20, 3, 3, 3).unwrap();

            files.dump("input.vpg", |w| write_vpg(w, &vpg)).unwrap();

            let solution =
                solve_variability_zielonka(&manager_ref, &vpg, ZielonkaVariant::FamilyOptimisedLeft, false).unwrap();
            let solution_expected =
                solve_variability_zielonka(&manager_ref, &vpg, ZielonkaVariant::Family, false).unwrap();

            debug_assert_eq!(solution[0], solution_expected[0]);
            debug_assert_eq!(solution[1], solution_expected[1]);
        })
    }
}
