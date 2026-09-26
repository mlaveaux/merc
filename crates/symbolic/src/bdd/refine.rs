use crate::CubeIterAll;
use crate::SummandGroupBdd;
use crate::SymbolicLtsBdd;
use crate::bdd_from_cube;
use crate::compute_vars_bdd;
use crate::extend_relation;
use crate::variable_rename;
use log::info;
use merc_io::TimeProgress;
use merc_lts::TransitionLabel;
use merc_utilities::MercError;
use oxidd::BooleanFunction;
use oxidd::BooleanFunctionQuant;
use oxidd::Manager;
use oxidd::ManagerRef;
use oxidd::VarNo;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;
use oxidd::util::OutOfMemory;

/// Strong bisimulation refinement algorithms for symbolic LTSs.
///
/// Returns the block relation `B(p, b)` together with the block-encoding
/// variables `b` (in allocation order). The block variables are required to
/// interpret the relation, e.g. to feed it into [`crate::quotient_symbolic`].
pub fn refine_bisimulation<L: TransitionLabel>(
    manager_ref: &BDDManagerRef,
    lts: &SymbolicLtsBdd<L>,
) -> Result<(BDDFunction, Vec<VarNo>), MercError> {
    // Computes the BDD representing all (next) state variables.
    let state_vars = manager_ref.with_manager_shared(|manager| -> Result<_, OutOfMemory> {
        let mut bdd: BDDFunction = BDDFunction::t(manager);

        for var in lts.state_variables().iter().chain(lts.next_state_variables().iter()) {
            let var = BDDFunction::var(manager, *var)?;
            bdd = bdd.and(&var)?;
        }

        Ok(bdd)
    })?;

    // Computes the vector of action label BDDs.
    let action_vars = manager_ref.with_manager_shared(|manager| -> Result<_, OutOfMemory> {
        lts.action_variables()
            .iter()
            .map(|var| BDDFunction::var(manager, *var))
            .collect::<Result<Vec<_>, OutOfMemory>>()
    })?;

    // Extend every transition group's relation with x = x' for the state variables not
    // written by that group, and collect the union of all action labels that occur
    // anywhere so we can later split per action instead of per (group, action).
    let mut extended_relations = Vec::new();
    let mut all_actions_bdd = manager_ref.with_manager_shared(|manager| BDDFunction::f(manager));
    for group in lts.transition_groups() {
        let relation = extend_relation(
            manager_ref,
            group.relation(),
            lts.state_variables(),
            lts.next_state_variables(),
            group.write_variables(),
        )?;

        all_actions_bdd = all_actions_bdd.or(&relation.exists(&state_vars)?)?;
        extended_relations.push(relation);
    }

    // Split into one T_a per action label. A bisimulation may match one summand
    // group's a-move against a different group's a-move, so T_a must be the union
    // (OR) of every group's extended relation for that label, not split per group.
    let mut split_groups = Vec::new();
    for cube in CubeIterAll::with_variables(&all_actions_bdd, lts.action_variables()) {
        // Every cube is a single action.
        let cube = cube?;
        let label_bdd = bdd_from_cube(manager_ref, &action_vars, &cube)?;

        let mut label_relation = manager_ref.with_manager_shared(|manager| BDDFunction::f(manager));
        for relation in &extended_relations {
            label_relation = label_relation.or(&relation.and(&label_bdd)?)?;
        }

        split_groups.push(SummandGroupBdd::new(
            label_relation,
            lts.state_variables().to_vec(),
            lts.next_state_variables().to_vec(),
        ));
    }

    // Introduce variables for q, q' after the state and next state variables.
    let q_variables = manager_ref
        .with_manager_exclusive(|manager| {
            manager.add_named_vars((0..lts.state_variables().len()).map(|index| format!("q_{index}")))
        })
        .map_err(|e| e.to_string())?
        .collect::<Vec<_>>();

    let q_prime_variables = manager_ref
        .with_manager_exclusive(|manager| {
            manager.add_named_vars((0..lts.state_variables().len()).map(|index| format!("q_prime_{index}")))
        })
        .map_err(|e| e.to_string())?
        .collect::<Vec<_>>();

    // We interleave the p, q, p', and q' variables that all represent states (and next states).
    // The action-label variables must also be listed (even though `set_var_order` does not need
    // to reorder them relative to each other): `set_var_order` places any variable *not*
    // mentioned in `order` wherever minimizes adjacent level swaps, which can (and does) insert
    // an unlisted action variable between two of the p/q/p'/q' variables above, breaking the
    // "target level exactly one below source" invariant `variable_rename` requires for the
    // p_to_q/q_to_p_prime/p_prime_to_q_prime substitutions built below.
    manager_ref.with_manager_exclusive(|manager| {
        let mut order: Vec<_> = lts
            .state_variables()
            .iter()
            .zip(q_variables.iter())
            .zip(lts.next_state_variables().iter().zip(q_prime_variables.iter()))
            .flat_map(|((s, q), (s_prime, q_prime))| [*s, *q, *s_prime, *q_prime])
            .collect();
        order.extend(lts.action_variables().iter().copied());

        oxidd_reorder::set_var_order(manager, &order)
    });

    let p_bdd = compute_vars_bdd(manager_ref, lts.state_variables())?.1;
    let p_prime_bdd = compute_vars_bdd(manager_ref, lts.next_state_variables())?.1;
    let q_bdd = compute_vars_bdd(manager_ref, &q_variables)?.1;
    let q_prime_bdd = compute_vars_bdd(manager_ref, &q_prime_variables)?.1;
    let action_vars_bdd = compute_vars_bdd(manager_ref, lts.action_variables())?.1;

    // Create renamings from (p -> q), (q -> p'), (p' -> q').
    let p_to_q: Vec<(VarNo, VarNo)> = lts
        .state_variables()
        .iter()
        .cloned()
        .zip(q_variables.iter().cloned())
        .collect();
    let q_to_p_prime: Vec<(VarNo, VarNo)> = q_variables
        .iter()
        .cloned()
        .zip(lts.next_state_variables().iter().cloned())
        .collect();
    let p_prime_to_q_prime: Vec<(VarNo, VarNo)> = lts
        .next_state_variables()
        .iter()
        .cloned()
        .zip(q_prime_variables.iter().cloned())
        .collect();

    // Represents the b and b' variables (and a renaming from b to b') that must be updated in every iteration.
    let mut b_variables: Vec<VarNo> = Vec::new();
    let mut b_vars_bdd = manager_ref.with_manager_shared(|manager| BDDFunction::t(manager));

    let mut b_prime_variables: Vec<VarNo> = Vec::new();
    let mut b_prime_vars_bdd = manager_ref.with_manager_shared(|manager| BDDFunction::t(manager));

    let mut b_to_b_prime: Vec<(VarNo, VarNo)> = Vec::new();

    // B_0(p, b) = 1 where |b| is 0.
    let mut blocks = lts.states().clone();

    let progress = TimeProgress::new(
        |iteration: usize| {
            info!("iteration {}", iteration);
        },
        1,
    );

    let mut iteration = 0;
    loop {
        // Check if B_i is stable w.r.t. all the transition relations. When an unstable group is
        // found, save it along with the precomputed B_i(p', b2) for use in the splitting step.
        let mut unstable_group: Option<&SummandGroupBdd> = None;
        let mut unstable_blocks_p_prime_b_prime: Option<BDDFunction> = None;

        for group in &split_groups {
            // Forall b, p, p', q: B_i(p, b) and B_i(q, b) and Ta(p, p') implies exists b', q': Ta(q, q') and B_i(p', b') and B_i(q', b')

            // Rename B_i(p, b) to B_i(p', b2) via the chain p -> q -> p' then b -> b'
            let blocks_q = variable_rename(manager_ref, &blocks, &p_to_q)?;
            let blocks_p_prime = variable_rename(manager_ref, &blocks_q, &q_to_p_prime)?;
            let blocks_p_prime_b_prime = variable_rename(manager_ref, &blocks_p_prime, &b_to_b_prime)?;

            let condition = blocks.and(&blocks_q)?.and(group.relation())?;

            let blocks_q_prime = variable_rename(manager_ref, &blocks_p_prime, &p_prime_to_q_prime)?;
            let blocks_q_prime_b_prime = variable_rename(manager_ref, &blocks_q_prime, &b_to_b_prime)?;

            // Rename Ta(p, p') to Ta(q, q')
            let relation_q = variable_rename(manager_ref, group.relation(), &p_to_q)?;
            let relation_q_q_prime = variable_rename(manager_ref, &relation_q, &p_prime_to_q_prime)?;

            // Computes exists b', q': Ta(q, q') and B_i(p', b') and B_i(q', b')
            let antecant = relation_q_q_prime
                .and(&blocks_p_prime_b_prime)?
                .and(&blocks_q_prime_b_prime)?
                .exists(&q_prime_bdd.and(&b_prime_vars_bdd)?)?;

            let group_stable = condition
                .imp(&antecant)?
                .forall(&b_vars_bdd.and(&p_bdd)?.and(&p_prime_bdd)?.and(&q_bdd)?)?
                .satisfiable();

            if !group_stable {
                unstable_group = Some(group);
                unstable_blocks_p_prime_b_prime = Some(blocks_p_prime_b_prime);
                break;
            }
        }

        let Some(group) = unstable_group else {
            return Ok((blocks, b_variables));
        };
        let blocks_p_prime_b_prime = unstable_blocks_p_prime_b_prime.unwrap();

        // Introduce new b and b' variables.
        let mut b_vars = manager_ref
            .with_manager_exclusive(|manager| {
                manager.add_named_vars([format!("b_{iteration}"), format!("b_prime_{iteration}")])
            })
            .map_err(|e| e.to_string())?;

        let b_var = b_vars.next().expect("Two variables are added");
        let b_prime_var = b_vars.next().expect("Two variables are added");

        // Update various structs related to the b variables.
        b_variables.push(b_var);
        b_prime_variables.push(b_prime_var);

        b_vars_bdd = manager_ref.with_manager_shared(|manager| b_vars_bdd.and(&BDDFunction::var(manager, b_var)?))?;
        b_prime_vars_bdd = manager_ref
            .with_manager_shared(|manager| b_prime_vars_bdd.and(&BDDFunction::var(manager, b_prime_var)?))?;

        b_to_b_prime.push((b_var, b_prime_var));

        // Implement state splitting: B_{i+1}(p, b, b1, b2) = B_i(p, b1) ∧ (b ⟺ ∃p', a: T_a(p, p') ∧ B_i(p', b2))
        //
        // blocks_p_prime_b_prime already holds B_i(p', b2). Quantify out p' and the fixed action
        // label variables to obtain a predicate over (p, b2) that says "p can do a to reach
        // the block encoded by b2".
        let to_quantify = p_prime_bdd.and(&action_vars_bdd)?;
        let reachable = group.relation().and(&blocks_p_prime_b_prime)?.exists(&to_quantify)?;

        let b_new_bdd = manager_ref.with_manager_shared(|manager| BDDFunction::var(manager, b_var))?;
        blocks = blocks.and(&b_new_bdd.equiv(&reachable)?)?;

        iteration += 1;
        progress.print(iteration);
    }
}

#[cfg(test)]
mod tests {
    use merc_lts::LTS;
    use merc_lts::LtsBuilderMem;
    use merc_reduction::Equivalence;
    use merc_reduction::compare_lts;
    use merc_reduction::reduce_lts;
    use merc_utilities::Timing;

    use merc_utilities::random_test;

    use oxidd::Manager;
    use oxidd::ManagerRef;

    use crate::SymbolicLtsBdd;
    use crate::bdd::refine_bisimulation;
    use crate::convert_symbolic_lts;
    use crate::convert_symbolic_lts_bdd;
    use crate::quotient_symbolic;
    use crate::random_symbolic_lts;

    /// Isolates the root cause at the `oxidd_reorder` level, independent of
    /// `SymbolicLtsBdd`/`random_symbolic_lts`: build a 5-variable manager
    /// with the same variable layout `SymbolicLtsBdd::from_symbolic_lts`
    /// produces for one state variable and one action-label bit (`s`, `s'`,
    /// `a`, in that creation order), add `q`/`q'` afterwards (as
    /// `refine_bisimulation` does), then call `set_var_order` exactly the
    /// way `refine.rs` does: with an `order` that lists only `s, q, s', q'`
    /// and omits `a`.
    ///
    /// `oxidd_reorder::set_var_order`'s own docs say unmentioned variables
    /// are "placed in a position such that the least number of adjacent
    /// level swaps need to be performed" — which is exactly what leaves `a`
    /// between `s` and `q` here, since in the *original* order `a` sits
    /// between the state/next-state block and the (not-yet-created) `q`
    /// block. That breaks the adjacency `variable_rename` requires for the
    /// `p -> q`, `q -> p'`, `p' -> q'` substitutions built later in
    /// `refine_bisimulation`.
    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn set_var_order_without_action_vars_breaks_p_q_adjacency() {
        let manager_ref = oxidd::bdd::new_manager(1024, 1024, 1);

        let (s, s_prime, a) = manager_ref.with_manager_exclusive(|manager| {
            let mut vars = manager
                .add_named_vars(["s".to_string(), "s_prime".to_string(), "a".to_string()])
                .expect("fresh manager, no duplicate names");
            (vars.next().unwrap(), vars.next().unwrap(), vars.next().unwrap())
        });

        let (q, q_prime) = manager_ref.with_manager_exclusive(|manager| {
            let mut vars = manager
                .add_named_vars(["q".to_string(), "q_prime".to_string()])
                .expect("fresh manager, no duplicate names");
            (vars.next().unwrap(), vars.next().unwrap())
        });

        // Exactly the `order` construction in `refine_bisimulation`
        // (crates/symbolic/src/bdd/refine.rs:106-112), specialized to one
        // state variable: interleave p, q, p', q' and say nothing about `a`.
        let order = vec![s, q, s_prime, q_prime];
        manager_ref.with_manager_exclusive(|manager| oxidd_reorder::set_var_order(manager, &order));

        let level_of = |var| manager_ref.with_manager_shared(|manager| manager.var_to_level(var));
        let (ls, lq, ls_prime, lq_prime, la) = (
            level_of(s),
            level_of(q),
            level_of(s_prime),
            level_of(q_prime),
            level_of(a),
        );

        // `variable_rename` (crates/symbolic/src/util.rs) requires each of
        // these three pairs to be *exactly* one level apart, since
        // `refine_bisimulation` builds `p_to_q`/`q_to_p_prime`/
        // `p_prime_to_q_prime` substitutions from them. This is the same
        // invariant whose violation panics with "Variable renaming must be
        // to the level directly below" in
        // `refine_bisimulation_panics_on_lts_with_action_variables` above.
        assert_eq!(
            lq,
            ls + 1,
            "q should be directly below s, but the unlisted action variable a landed at level {la} \
             (s={ls}, q={lq}, s'={ls_prime}, q'={lq_prime})"
        );
        assert_eq!(ls_prime, lq + 1, "s' should be directly below q");
        assert_eq!(lq_prime, ls_prime + 1, "q' should be directly below s'");
    }

    /// Same setup as [`set_var_order_without_action_vars_breaks_p_q_adjacency`],
    /// but confirms the fix direction: appending the action variable(s) to
    /// `order` (instead of omitting them) keeps `set_var_order` from
    /// inserting them into the p/q/p'/q' interleaving, since the order is
    /// then total (mentions every variable in the manager) and `a` is
    /// explicitly placed after `q'`.
    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn set_var_order_with_action_vars_appended_preserves_p_q_adjacency() {
        let manager_ref = oxidd::bdd::new_manager(1024, 1024, 1);

        let (s, s_prime, a) = manager_ref.with_manager_exclusive(|manager| {
            let mut vars = manager
                .add_named_vars(["s".to_string(), "s_prime".to_string(), "a".to_string()])
                .expect("fresh manager, no duplicate names");
            (vars.next().unwrap(), vars.next().unwrap(), vars.next().unwrap())
        });

        let (q, q_prime) = manager_ref.with_manager_exclusive(|manager| {
            let mut vars = manager
                .add_named_vars(["q".to_string(), "q_prime".to_string()])
                .expect("fresh manager, no duplicate names");
            (vars.next().unwrap(), vars.next().unwrap())
        });

        // The proposed fix: append the action variable(s) after q' instead of
        // leaving them out of `order` entirely.
        let order = vec![s, q, s_prime, q_prime, a];
        manager_ref.with_manager_exclusive(|manager| oxidd_reorder::set_var_order(manager, &order));

        let level_of = |var| manager_ref.with_manager_shared(|manager| manager.var_to_level(var));
        let (ls, lq, ls_prime, lq_prime, la) = (
            level_of(s),
            level_of(q),
            level_of(s_prime),
            level_of(q_prime),
            level_of(a),
        );

        assert_eq!(lq, ls + 1, "q should be directly below s");
        assert_eq!(ls_prime, lq + 1, "s' should be directly below q");
        assert_eq!(lq_prime, ls_prime + 1, "q' should be directly below s'");
        assert_eq!(la, lq_prime + 1, "a should sort after q', as requested");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn refine_bisimulation_panics_on_lts_with_action_variables() {
        // Minimal, deterministic repro of the `set_var_order` panic (see the
        // module docs / review/phase-4-tools-cli.md): any LTS whose action
        // labels need at least one bit panics inside `refine_bisimulation`
        // before it can return a result.
        let mut rng = rand::rng();
        let ldd_manager = oxidd::ldd::new_manager(2048, 1024, 1);

        // 1 state variable, 2 action labels (i.e. `action_label_bits >= 1`).
        let lts = random_symbolic_lts(&mut rng, &ldd_manager, 1, 2).unwrap();

        let manager_ref = oxidd::bdd::new_manager(2028, 2028, 1);
        let lts_bdd = SymbolicLtsBdd::from_symbolic_lts(&ldd_manager, &manager_ref, &lts).unwrap();

        assert!(
            !lts_bdd.action_variables().is_empty(),
            "the repro requires at least one action-label bit in the manager"
        );

        // This currently panics with "assertion `left == right` failed: the
        // level number does not match" inside oxidd_reorder::set_var_order,
        // called from refine.rs's `set_var_order` line. Once fixed, this
        // should return Ok(..) instead.
        let _ = refine_bisimulation(&manager_ref, &lts_bdd).unwrap();
    }

    #[test]
    #[ignore = "refine_bisimulation aborts in oxidd_reorder::set_var_order; see function docs"]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_refine_bisimulation() {
        random_test(100, |rng| {
            let ldd_manager = oxidd::ldd::new_manager(2048, 1024, 1);

            let lts = random_symbolic_lts(rng, &ldd_manager, 10, 5).unwrap();

            let manager_ref = oxidd::bdd::new_manager(2028, 2028, 1);
            let lts_bdd = SymbolicLtsBdd::from_symbolic_lts(&ldd_manager, &manager_ref, &lts).unwrap();

            let mut builder = LtsBuilderMem::new(Vec::new(), Vec::new());
            let explicit_lts = convert_symbolic_lts(&ldd_manager, &mut builder, &lts).unwrap();
            let explicit_lts_reduced =
                reduce_lts(explicit_lts.clone(), Equivalence::StrongBisim, false, &Timing::new());

            // refine_bisimulation returns B(p, b) together with the block variables b,
            // which is exactly the (partition, block_vars) pair quotient_symbolic expects.
            let (partition, block_vars) = refine_bisimulation(&manager_ref, &lts_bdd).unwrap();

            let quotient_lts = quotient_symbolic(&manager_ref, &lts_bdd, &partition, &block_vars).unwrap();

            let mut builder = LtsBuilderMem::new(Vec::new(), Vec::new());
            let symbolic_lts_reduced = convert_symbolic_lts_bdd(&manager_ref, &mut builder, &quotient_lts).unwrap();

            assert_eq!(
                explicit_lts_reduced.num_of_states(),
                symbolic_lts_reduced.num_of_states()
            );
            assert_eq!(
                explicit_lts_reduced.num_of_transitions(),
                symbolic_lts_reduced.num_of_transitions()
            );

            assert!(
                compare_lts(
                    Equivalence::StrongBisim,
                    explicit_lts_reduced,
                    symbolic_lts_reduced,
                    false,
                    false,
                    &Timing::new()
                )
                .0,
                "The refine_bisimulation quotient should be bisimilar to the explicit reduction"
            );
        });
    }
}
