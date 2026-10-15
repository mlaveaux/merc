use super::sigref::encode_block;
use crate::CubeIterAll;
use crate::SatCountCache;
use crate::SummandGroupBdd;
use crate::SymbolicLtsBdd;
use crate::approx_satcount;
use crate::bdd_from_cube;
use crate::collect_children;
use crate::compute_vars_bdd;
use crate::extend_relation;
use crate::reduce;
use crate::variable_rename;
use log::info;
use merc_io::TimeProgress;
use merc_lts::TransitionLabel;
use merc_utilities::MercError;
use oxidd::BooleanFunction;
use oxidd::BooleanFunctionQuant;
use oxidd::Function;
use oxidd::FunctionSubst;
use oxidd::HasLevel;
use oxidd::LevelNo;
use oxidd::Manager;
use oxidd::ManagerRef;
use oxidd::Node;
use oxidd::Subst;
use oxidd::VarNo;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;
use oxidd::util::OutOfMemory;
use oxidd::util::Ref;
use oxidd_core::function::EdgeOfFunc;
use oxidd_core::function::OwnEdgeOfFunc;
use oxidd_rules_bdd::simple::BDDTerminal;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;

/// Strong bisimulation refinement algorithms for symbolic LTSs.
///
/// Returns the block relation `B(p', b)`, defined over the next state
/// variables, together with the block-encoding variables `b` (least
/// significant bit first). Every reachable state is assigned exactly one block
/// number. The block variables are required to interpret the relation, e.g. to
/// feed it into [`crate::quotient_symbolic`].
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

    // Every class is identified by a single block number over the fixed block variables b. The
    // paper's encoding instead appends 1 + 2|b| fresh bits per split, which makes the set of
    // b-vectors of a state grow beyond a singleton from the second split onwards. Then the
    // stability condition tests overlap of vector sets instead of class equality, which is not
    // an equivalence relation and stops the refinement from making progress. Renumbering after
    // every split (the paper's ⌈log2 k⌉ re-encoding) keeps one vector per state, so the stability
    // condition is exactly the classical one and the block count strictly increases.
    let num_of_states = approx_satcount(
        lts.states(),
        lts.state_variables().len() as VarNo,
        &mut SatCountCache::new(),
    );
    let num_of_block_bits = (num_of_states.as_f64().log2().ceil() as usize).max(1);

    // Introduce variables for q, q', the block variables b, b' and the splitter variable c.
    let q_variables = add_vars(
        manager_ref,
        (0..lts.state_variables().len()).map(|index| format!("q_{index}")),
    )?;
    let q_prime_variables = add_vars(
        manager_ref,
        (0..lts.state_variables().len()).map(|index| format!("q_prime_{index}")),
    )?;
    let b_variables = add_vars(manager_ref, (0..num_of_block_bits).map(|index| format!("b_{index}")))?;
    let b_prime_variables = add_vars(
        manager_ref,
        (0..num_of_block_bits).map(|index| format!("b_prime_{index}")),
    )?;
    let c_variable = add_vars(manager_ref, ["c".to_string()])?[0];

    // The order consists of the interleaved p and p' variables, followed by the interleaved q and q'
    // variables, the action variables, the splitter variable and the interleaved b and b' variables.
    //
    // This keeps every p' directly below p, as required by [variable_rename] here and in
    // [crate::quotient_symbolic], and it matches the existing order of the p, p' and action
    // variables. So only the new (empty) levels are moved, which avoids expensive level swaps. The q
    // variables are not interleaved with the p variables, since then B_i(p, b) ∧ B_i(q, b) is
    // essentially the product of both BDDs. All block related variables must be below the state
    // variables, see [renumber_blocks].
    manager_ref.with_manager_exclusive(|manager| {
        let order: Vec<_> = lts
            .state_variables()
            .iter()
            .zip(lts.next_state_variables().iter())
            .flat_map(|(s, s_prime)| [*s, *s_prime])
            .chain(
                q_variables
                    .iter()
                    .zip(q_prime_variables.iter())
                    .flat_map(|(q, q_prime)| [*q, *q_prime]),
            )
            .chain(lts.action_variables().iter().cloned())
            .chain([c_variable])
            .chain(
                b_variables
                    .iter()
                    .zip(b_prime_variables.iter())
                    .flat_map(|(b, b_prime)| [*b, *b_prime]),
            )
            .collect();

        oxidd_reorder::set_var_order(manager, &order)
    });

    let p_bdd = compute_vars_bdd(manager_ref, lts.state_variables())?.1;
    let p_prime_bdd = compute_vars_bdd(manager_ref, lts.next_state_variables())?.1;
    let q_bdd = compute_vars_bdd(manager_ref, &q_variables)?.1;
    let q_prime_bdd = compute_vars_bdd(manager_ref, &q_prime_variables)?.1;
    let action_vars_bdd = compute_vars_bdd(manager_ref, lts.action_variables())?.1;
    let (b_variables_bdds, b_vars_bdd) = compute_vars_bdd(manager_ref, &b_variables)?;
    let b_prime_vars_bdd = compute_vars_bdd(manager_ref, &b_prime_variables)?.1;
    let c_bdd = manager_ref.with_manager_shared(|manager| BDDFunction::var(manager, c_variable))?;

    // Create the renamings (p -> p') and (b -> b'), which are to the level directly below, and the
    // substitutions (p -> q, p' -> q') and (p' -> q'), which are not.
    let p_to_p_prime: Vec<(VarNo, VarNo)> = lts
        .state_variables()
        .iter()
        .cloned()
        .zip(lts.next_state_variables().iter().cloned())
        .collect();
    let b_to_b_prime: Vec<(VarNo, VarNo)> = b_variables
        .iter()
        .cloned()
        .zip(b_prime_variables.iter().cloned())
        .collect();

    let q_variables_bdds = compute_vars_bdd(manager_ref, &q_variables)?.0;
    let q_prime_variables_bdds = compute_vars_bdd(manager_ref, &q_prime_variables)?.0;
    let p_to_q_subst = Subst::new(
        lts.state_variables()
            .iter()
            .chain(lts.next_state_variables().iter())
            .cloned()
            .collect::<Vec<_>>(),
        q_variables_bdds
            .iter()
            .chain(q_prime_variables_bdds.iter())
            .cloned()
            .collect::<Vec<_>>(),
    );
    let p_prime_to_q_prime_subst = Subst::new(lts.next_state_variables().to_vec(), q_prime_variables_bdds);

    // Rename Ta(p, p') to Ta(q, q') once, these do not change during the refinement.
    let relations_q_q_prime = split_groups
        .iter()
        .map(|group| group.relation().substitute(&p_to_q_subst))
        .collect::<Result<Vec<_>, OutOfMemory>>()?;

    // The universally quantified variables of the stability condition. The action variables are
    // included since T_a(p, p') contains them, otherwise the result is a predicate over the
    // action label that is trivially satisfiable for every other label.
    let forall_vars_bdd = b_vars_bdd
        .and(&p_bdd)?
        .and(&p_prime_bdd)?
        .and(&q_bdd)?
        .and(&action_vars_bdd)?;
    let exists_vars_bdd = q_prime_bdd.and(&b_prime_vars_bdd)?;

    // B_0(p, b) holds for the reachable states p and block number 0.
    let mut blocks = lts.states().and(&manager_ref.with_manager_shared(
        |manager| -> Result<BDDFunction, OutOfMemory> {
            Ok(BDDFunction::from_edge(
                manager,
                encode_block(manager, &b_variables_bdds, 0)?,
            ))
        },
    )?)?;
    let mut num_of_blocks = 1;

    let progress = TimeProgress::new(
        |(iteration, num_of_blocks): (usize, u64)| {
            info!("iteration {}: {} blocks", iteration, num_of_blocks);
        },
        1,
    );

    let mut iteration = 0;
    loop {
        // Rename B_i(p, b) to B_i(q, b), B_i(p', b') and B_i(q', b').
        let blocks_q = blocks.substitute(&p_to_q_subst)?;
        let blocks_p_prime = variable_rename(manager_ref, &blocks, &p_to_p_prime)?;
        let blocks_p_prime_b_prime = variable_rename(manager_ref, &blocks_p_prime, &b_to_b_prime)?;
        let blocks_q_prime_b_prime = blocks_p_prime_b_prime.substitute(&p_prime_to_q_prime_subst)?;

        // B_i(p, b) and B_i(q, b), i.e., p and q are in the same block.
        let same_block = blocks.and(&blocks_q)?;
        // B_i(p', b') and B_i(q', b'), i.e., p' and q' are in the same block.
        let same_block_prime = blocks_p_prime_b_prime.and(&blocks_q_prime_b_prime)?;

        // Check if B_i is stable w.r.t. all the transition relations, and find an unstable one.
        let mut unstable_group: Option<&SummandGroupBdd> = None;
        for (group, relation_q_q_prime) in split_groups.iter().zip(relations_q_q_prime.iter()) {
            // Forall b, p, p', q: B_i(p, b) and B_i(q, b) and Ta(p, p') implies exists b', q': Ta(q, q') and B_i(p', b') and B_i(q', b')
            let condition = same_block.and(group.relation())?;
            let consequent = relation_q_q_prime.and(&same_block_prime)?.exists(&exists_vars_bdd)?;

            if !condition.imp(&consequent)?.forall(&forall_vars_bdd)?.valid() {
                unstable_group = Some(group);
                break;
            }
        }

        let Some(group) = unstable_group else {
            info!(
                "Refinement completed in {} iterations with {} blocks",
                iteration, num_of_blocks
            );

            // Return the partition over the next state variables, as expected by [crate::quotient_symbolic].
            return Ok((blocks_p_prime, b_variables));
        };

        // Implement state splitting: B'(p, c, b, b') = B_i(p, b) ∧ (c ⟺ ∃p', a: T_a(p, p') ∧ B_i(p', b'))
        //
        // Quantify out p' and the fixed action label variables to obtain a predicate over (p, b')
        // that says "p can do a to reach the block encoded by b'". Two states end up with the
        // same cofactor of B' iff they were in the same block and can reach the same blocks.
        let to_quantify = p_prime_bdd.and(&action_vars_bdd)?;
        let reachable = group.relation().and(&blocks_p_prime_b_prime)?.exists(&to_quantify)?;
        let split = blocks.and(&c_bdd.equiv(&reachable)?)?;

        // Assign every distinct cofactor of B' a block number to obtain B_{i+1}(p, b).
        let (new_blocks, new_num_of_blocks) =
            renumber_blocks(manager_ref, &split, lts.state_variables(), &b_variables_bdds)?;

        if new_num_of_blocks > 1 << num_of_block_bits {
            return Err(format!(
                "The {new_num_of_blocks} blocks cannot be encoded using {num_of_block_bits} block variables"
            )
            .into());
        }

        if new_num_of_blocks <= num_of_blocks {
            return Err(format!(
                "Splitting an unstable partition did not increase the number of blocks ({num_of_blocks})"
            )
            .into());
        }

        blocks = new_blocks;
        num_of_blocks = new_num_of_blocks;

        iteration += 1;
        progress.print((iteration, num_of_blocks));
    }
}

/// Adds the given named variables to the manager and returns them.
fn add_vars(manager_ref: &BDDManagerRef, names: impl IntoIterator<Item = String>) -> Result<Vec<VarNo>, MercError> {
    Ok(manager_ref
        .with_manager_exclusive(|manager| manager.add_named_vars(names))
        .map_err(|e| e.to_string())?
        .collect())
}

/// Replaces every distinct cofactor of `relation(p, ...)` w.r.t. the state
/// variables `p` by a unique block number encoded over `block_variables`.
///
/// Returns the new partition `B(p, b)` and the number of blocks.
///
/// # Details
///
/// Assumes that all non-state variables in the support of `relation` are
/// below the state variables in the variable order. Then two states have the
/// same cofactor iff their paths reach the same node on the boundary, i.e., the
/// first node that is not labelled by a state variable. The false terminal
/// corresponds to unreachable states and is kept as is.
fn renumber_blocks(
    manager_ref: &BDDManagerRef,
    relation: &BDDFunction,
    state_variables: &[VarNo],
    block_variables: &[BDDFunction],
) -> Result<(BDDFunction, u64), OutOfMemory> {
    manager_ref.with_manager_shared(|manager| {
        let state_levels: FxHashSet<LevelNo> = state_variables.iter().map(|var| manager.var_to_level(*var)).collect();

        let mut ctx = RenumberContext {
            manager,
            cache: FxHashMap::default(),
            state_levels,
            block_variables,
            next_block: 0,
        };

        let result = renumber_blocks_edge(&mut ctx, relation.as_edge(manager))?;
        Ok((BDDFunction::from_edge(manager, result), ctx.next_block))
    })
}

/// Shared state threaded through the [renumber_blocks_edge] recursion.
struct RenumberContext<'a, 'id: 'a> {
    manager: &'a <BDDFunction as Function>::Manager<'id>,
    /// Maps both inner nodes on state levels and boundary nodes to their result.
    cache: FxHashMap<BDDFunction, BDDFunction>,
    state_levels: FxHashSet<LevelNo>,
    block_variables: &'a [BDDFunction],
    next_block: u64,
}

/// Recursive implementation of [renumber_blocks].
fn renumber_blocks_edge<'id>(
    ctx: &mut RenumberContext<'_, 'id>,
    edge: Ref<EdgeOfFunc<'id, BDDFunction>>,
) -> Result<OwnEdgeOfFunc<'id, BDDFunction>, OutOfMemory> {
    let manager = ctx.manager;

    let state_node = match manager.get_node(edge) {
        Node::Terminal(BDDTerminal::False) => return Ok(manager.clone_edge(edge)),
        Node::Terminal(BDDTerminal::True) => None,
        Node::Inner(node) => ctx.state_levels.contains(&node.level()).then_some(node),
    };

    let key = BDDFunction::from_edge(manager, manager.clone_edge(edge));
    if let Some(cached) = ctx.cache.get(&key) {
        return Ok(manager.clone_edge(cached.as_edge(manager)));
    }

    let result = if let Some(node) = state_node {
        let (high, low) = collect_children(node);
        let high = renumber_blocks_edge(ctx, high)?;
        let low = renumber_blocks_edge(ctx, low)?;
        reduce(manager, node.level(), high, low)?
    } else {
        // A boundary node: all states reaching it are in the same block.
        let block = ctx.next_block;
        ctx.next_block += 1;
        encode_block(manager, ctx.block_variables, block)?
    };

    ctx.cache.insert(
        key,
        BDDFunction::from_edge(manager, manager.clone_edge(result.borrowed())),
    );
    Ok(result)
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

    use crate::BDD_CACHE_CAPACITY;
    use crate::BDD_NODE_CAPACITY;
    use crate::LDD_CACHE_CAPACITY;
    use crate::LDD_NODE_CAPACITY;
    use crate::SymbolicLtsBdd;
    use crate::bdd::refine_bisimulation;
    use crate::convert_symbolic_lts;
    use crate::convert_symbolic_lts_bdd;
    use crate::quotient_symbolic;
    use crate::random_symbolic_lts;

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_refine_bisimulation() {
        random_test(100, |rng| {
            let ldd_manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);

            let lts = random_symbolic_lts(rng, &ldd_manager, 10, 5).unwrap();

            let manager_ref = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);
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
