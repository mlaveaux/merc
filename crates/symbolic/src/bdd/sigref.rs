use std::fmt;
use std::ops::Range;

use itertools::Itertools;
use log::debug;
use log::info;
use log::trace;
use merc_io::TimeProgress;
use merc_lts::TransitionLabel;
use merc_utilities::MercError;
use merc_utilities::Timing;
use oxidd::BooleanFunction;
use oxidd::BooleanFunctionQuant;
use oxidd::BooleanOperator;
use oxidd::Function;
use oxidd::HasLevel;
use oxidd::Manager;
use oxidd::ManagerRef;
use oxidd::Node;
use oxidd::VarNo;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;
use oxidd::error::DuplicateVarName;
use oxidd::util::OptBool;
use oxidd::util::OutOfMemory;
use oxidd::util::Ref;
use oxidd_core::function::EdgeOfFunc;
use oxidd_core::function::OwnEdgeOfFunc;
use oxidd_core::util::EdgeDropGuard;
use oxidd_dump::Visualizer;
use oxidd_rules_bdd::simple::BDDTerminal;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;

use crate::CubeIterAll;
use crate::SatCountCache;
use crate::SummandGroupBdd;
use crate::SymbolicLtsBdd;
use crate::ValuesIter;
use crate::approx_satcount;
use crate::collect_children;
use crate::compute_vars_bdd;
use crate::reduce;
use crate::required_bits_64;
use crate::to_value;
use crate::variable_rename;

/// Computes the reduction of the given symbolic LTS using symbolic signature
/// refinement.
///
/// # Details
///
/// The implementation is based on the following paper:
///
/// > Tom van Dijk and Jaco van de Pol. Multi-core Symbolic Bisimulation
/// > Minimization.
pub fn sigref_symbolic<L: TransitionLabel>(
    manager_ref: &BDDManagerRef,
    lts: &SymbolicLtsBdd<L>,
    timing: &Timing,
    split_signature: bool,
    extend_relation: bool,
    merge_transitions: bool,
    visualize: bool,
) -> Result<(BDDFunction, Vec<VarNo>, usize), MercError> {
    if merge_transitions && !extend_relation {
        debug!("merge_transitions requires full-domain transition relations; enabling relation extension");
    }

    let use_extended_relations = extend_relation || merge_transitions;
    let use_split_signature = split_signature && !merge_transitions;

    let preprocessed_lts = preprocess_lts(manager_ref, lts, use_extended_relations, merge_transitions)?;

    sigref_symbolic_impl(manager_ref, &preprocessed_lts, timing, use_split_signature, visualize)
}

/// Preprocess the LTS based on the input options.
///
/// # Details
///
/// When `extend_relations` is set, each transition relation is extended with `x = x'`
/// for every state variable that is not written by the relation; the group's read
/// and write variables then cover the full state domain. When `merge_transitions`
/// is set, the (extended) relations are then combined into a single transition
/// group covering the full state domain.
fn preprocess_lts<L: TransitionLabel>(
    manager_ref: &BDDManagerRef,
    lts: &SymbolicLtsBdd<L>,
    extend_relations: bool,
    merge_transitions: bool,
) -> Result<SymbolicLtsBdd<L>, MercError> {
    if !extend_relations && !merge_transitions {
        let groups = lts
            .transition_groups()
            .iter()
            .map(|group| {
                SummandGroupBdd::new(
                    group.relation().clone(),
                    group.read_variables().to_vec(),
                    group.write_variables().to_vec(),
                )
            })
            .collect();
        return Ok(SymbolicLtsBdd::with_transition_groups(lts, groups));
    }

    // Extend each transition relation to the full state/next-state domain.
    let mut groups: Vec<SummandGroupBdd> = lts
        .transition_groups()
        .iter()
        .map(|group| -> Result<SummandGroupBdd, MercError> {
            let relation = extend_relation(
                manager_ref,
                group.relation(),
                lts.state_variables(),
                lts.next_state_variables(),
                group.write_variables(),
            )?;
            Ok(SummandGroupBdd::new(
                relation,
                lts.state_variables().to_vec(),
                lts.next_state_variables().to_vec(),
            ))
        })
        .collect::<Result<Vec<_>, MercError>>()?;

    if merge_transitions && !groups.is_empty() {
        let mut merged_relation = manager_ref.with_manager_shared(|manager| BDDFunction::f(manager));
        for group in &groups {
            merged_relation = merged_relation.or(group.relation())?;
        }
        groups = vec![SummandGroupBdd::new(
            merged_relation,
            lts.state_variables().to_vec(),
            lts.next_state_variables().to_vec(),
        )];
    }

    Ok(SymbolicLtsBdd::with_transition_groups(lts, groups))
}

/// Implementation of [sigref_symbolic] on an already preprocessed LTS.
fn sigref_symbolic_impl<L: TransitionLabel>(
    manager_ref: &BDDManagerRef,
    lts: &SymbolicLtsBdd<L>,
    timing: &Timing,
    split_signature: bool,
    visualize: bool,
) -> Result<(BDDFunction, Vec<VarNo>, usize), MercError> {
    let split_partition_groups = if split_signature {
        combine_transition_groups(manager_ref, lts)?
    } else {
        Vec::new()
    };

    let mut satcount_cache = SatCountCache::new();
    let num_of_states = approx_satcount(lts.states(), lts.state_variables().len() as VarNo, &mut satcount_cache);
    let num_of_block_bits = (num_of_states.as_f64().log2().ceil() as u32).max(1);
    debug!("Number of block bits: {}", num_of_block_bits);

    let block_variable_names = (0..num_of_block_bits)
        .map(|i| format!("b_{}", i))
        .collect::<Vec<String>>();

    // Create variables in the BDD manager
    let block_variable_indices: Vec<VarNo> = manager_ref
        .with_manager_exclusive(|manager| -> Result<Range<VarNo>, DuplicateVarName> {
            manager.add_named_vars(block_variable_names)
        })
        .map_err(|e| format!("Failed to create variables: {e}"))?
        .collect();

    // Create BDD functions for the block variables
    let block_variables_bdds = compute_vars_bdd(manager_ref, &block_variable_indices)?.0;

    // Caches for [refine]. `refine_cache` is the BDD-level memo and its allocation is reused across
    // iterations (cleared each iteration). `claimed_blocks` records which old block ids have already
    // been claimed (reused as-is) this iteration; `next_fresh_id` is the persistent counter handing
    // out new block ids for splits, so num_of_blocks == next_fresh_id once refine returns.
    let mut refine_cache: FxHashMap<(BDDFunction, BDDFunction), BDDFunction> = FxHashMap::default();
    let mut claimed_blocks: FxHashSet<u64> = FxHashSet::default();
    let mut next_fresh_id: u64 = 1;

    // Substitution to replace state variables with next state variables.
    let state_substitution: Vec<(VarNo, VarNo)> = lts
        .state_variables()
        .iter()
        .cloned()
        .zip(lts.next_state_variables().iter().cloned())
        .collect();

    // Determine the variables in the support of a signature function.
    let signature_variables = lts
        .next_state_variables()
        .iter()
        .chain(lts.action_variables())
        .chain(block_variable_indices.iter())
        .cloned()
        .collect::<Vec<VarNo>>();

    // Determine the variables in the support of a partition function.
    let partition_variables = lts
        .next_state_variables()
        .iter()
        .chain(block_variable_indices.iter())
        .cloned()
        .collect::<Vec<VarNo>>();

    // Stores the partition of the states as BDD.
    let mut partition = lts.states().and(&manager_ref.with_manager_shared(
        |manager| -> Result<BDDFunction, OutOfMemory> {
            Ok(BDDFunction::from_edge(
                manager,
                encode_block(manager, &block_variables_bdds, 0)?,
            ))
        },
    )?)?;

    // In the sigref algorithm, the partition is defined over the next state. When we compute the signature
    // we then get (s, a, b), since in the signature we need to consider the block of the next state.
    partition = variable_rename(manager_ref, &partition, &state_substitution)?;

    // Keep track of local information.
    let mut num_of_blocks = 0;
    let mut iteration = 0usize;

    let progress = TimeProgress::new(
        |(iterations, num_of_blocks): (usize, usize)| {
            info!("iteration {}: {} blocks", iterations, num_of_blocks);
        },
        1,
    );

    if visualize {
        // Visualize the initial partition.
        manager_ref.with_manager_shared(|manager| {
            Visualizer::new()
                .add("initial_partition", manager, [&partition])
                .serve()
        })?;
    }

    trace!(
        "Initial partition: {}",
        PartitionDisplay::new(
            &partition,
            &partition_variables,
            lts.state_variable_num_of_bits(),
            num_of_block_bits
        )
    );

    // Quantification BDD per transition group, derived from the (already
    // preprocessed) write variables of that group.
    let relation_quantified_next_state_vars: Vec<BDDFunction> = lts
        .transition_groups()
        .iter()
        .map(|group| -> Result<_, MercError> { Ok(compute_vars_bdd(manager_ref, group.write_variables())?.1) })
        .collect::<Result<Vec<_>, MercError>>()?;

    let mut signature_index = 0;
    let mut changed_in_cycle = false;
    loop {
        // No fixed point reached yet, so keep refining.
        let old_num_of_blocks = num_of_blocks;
        trace!("Iteration {} ({} blocks)", iteration, num_of_blocks);

        // Compute the new signatures w.r.t. the previous partition.
        let signature = timing.measure("signature", || -> Result<BDDFunction, OutOfMemory> {
            let mut signature = manager_ref.with_manager_shared(|manager| BDDFunction::f(manager));

            // Select which transition groups to combine in this iteration: either a single
            // partition class (split signature) or all groups at once.
            let group_indices: Box<dyn Iterator<Item = usize>> =
                if split_signature && !split_partition_groups.is_empty() {
                    Box::new(split_partition_groups[signature_index].iter().copied())
                } else {
                    Box::new(0..lts.transition_groups().len())
                };

            for index in group_indices {
                // We explicitly do not quantify over next-state variables that are not written
                // by the transition group. Otherwise these s' would become unconstrained and,
                // after the s -> s' rename below, would conflate states.
                let group_signature = timing.measure(&format!("group_signature_{}", index), || {
                    signature_strong(
                        &partition,
                        lts.transition_groups()[index].relation(),
                        &relation_quantified_next_state_vars[index],
                    )
                })?;

                let group_signature = variable_rename(manager_ref, &group_signature, &state_substitution)?;
                signature = timing.measure("signature_or", || signature.or(&group_signature))?;
            }

            Ok(signature)
        })?;

        trace!(
            "Signature at iteration {}: {}",
            iteration,
            SignatureDisplay::new(
                &signature,
                &signature_variables,
                lts.state_variable_num_of_bits(),
                lts.action_variables().len() as u32,
                num_of_block_bits
            )
        );

        if visualize {
            // Visualize the computed signature.
            manager_ref.with_manager_shared(|manager| {
                Visualizer::new()
                    .add(&format!("signature_{iteration}"), manager, [&signature])
                    .serve()
            })?;
        }

        // Build the new partition based on the signatures.
        partition = timing.measure("refine", || {
            refine(
                manager_ref,
                &mut refine_cache,
                &mut claimed_blocks,
                &mut next_fresh_id,
                &block_variables_bdds,
                lts.next_state_variables(),
                &signature,
                &partition,
            )
        })?;

        if visualize {
            // Visualize the current partition.
            manager_ref.with_manager_shared(|manager| {
                Visualizer::new()
                    .add(&format!("partition_{iteration}"), manager, [&partition])
                    .serve()
            })?;
        }

        trace!(
            "Partition at iteration {}: {}",
            iteration,
            PartitionDisplay::new(
                &partition,
                &partition_variables,
                lts.state_variable_num_of_bits(),
                num_of_block_bits
            )
        );

        num_of_blocks = next_fresh_id as usize;
        progress.print((iteration, num_of_blocks));
        iteration += 1;

        if split_signature {
            if num_of_blocks != old_num_of_blocks {
                changed_in_cycle = true;
            }

            signature_index += 1;
            if signature_index >= split_partition_groups.len() {
                // Completed a full sweep over every partition class: we have a fixed point
                // only if not a single class refined the partition during this sweep.
                if !changed_in_cycle {
                    break;
                }
                changed_in_cycle = false;
                signature_index = 0;
            }
        } else if num_of_blocks == old_num_of_blocks {
            break;
        }

        // Per-iteration state is rebuilt; `next_fresh_id` is persistent so new ids never collide
        // with ids carried over from the previous partition.
        claimed_blocks.clear();
        refine_cache.clear();
    }

    info!(
        "Signature refinement completed in {} iterations with {} blocks",
        iteration, num_of_blocks
    );

    Ok((partition, block_variable_indices, num_of_blocks))
}

/// Computes a partitioning of the transition groups for the given LTS based on the split signature option.
///
/// # Details
///
/// Two transition groups are combined if they share action labels, this ensures that in split signature mode
/// the action labels of all transition groups are disjoint.
fn combine_transition_groups<L: TransitionLabel>(
    manager_ref: &BDDManagerRef,
    lts: &SymbolicLtsBdd<L>,
) -> Result<Vec<Vec<usize>>, MercError> {
    // In split signature mode we must ensure that the action labels of all transition groups are disjoint. We do this by merging
    // transition groups that share action labels.

    // Cube over the state and next-state variables; quantifying these out of a relation leaves
    // only the action labels.
    let state_and_next_state_vars = manager_ref.with_manager_shared(|manager| -> Result<_, OutOfMemory> {
        let mut bdd: BDDFunction = BDDFunction::t(manager);

        for var in lts.state_variables().iter().chain(lts.next_state_variables().iter()) {
            let var = BDDFunction::var(manager, *var)?;
            bdd = bdd.and(&var)?;
        }

        Ok(bdd)
    })?;

    // Compute the action labels for all transition groups.
    let representatives = lts
        .transition_groups()
        .iter()
        .map(|group| group.relation().exists(&state_and_next_state_vars))
        .collect::<Result<Vec<BDDFunction>, OutOfMemory>>()?;

    // Build connected components under "shares an action label". For each class we cache the
    // union of its members' action-label BDDs so the overlap test stays a single `and` per
    // pair, and we absorb every overlapping class — the relation is not transitive, so a
    // single-pass check against one representative per class would miss merges that chain
    // through other groups.
    let mut classes: Vec<(Vec<usize>, BDDFunction)> = Vec::new();

    for (i, representative) in representatives.iter().enumerate().take(lts.transition_groups().len()) {
        let mut merged_members = vec![i];
        let mut merged_rep = representative.clone();

        let mut j = 0;
        while j < classes.len() {
            if classes[j].1.and(&merged_rep)?.satisfiable() {
                let (members, rep) = classes.remove(j);
                merged_members.extend(members);
                merged_rep = merged_rep.or(&rep)?;
            } else {
                j += 1;
            }
        }

        classes.push((merged_members, merged_rep));
    }

    let result: Vec<Vec<usize>> = classes.into_iter().map(|(members, _)| members).collect();

    debug!("Combined transition groups: {:?}", result);
    Ok(result)
}

/// Computes the strong signature refinement of the given partition and
/// relation.
///
/// # Details
///
/// For strong bisimulation the signature is defined as follows, where `P` is
/// the previous partition defined over the next state variables, and `relation` is defined
/// over the states, next states and the action label bits.
///
/// > ∃ s'. (relation(s, s', a) ∧ P(s', b))
fn signature_strong(
    partition: &BDDFunction,
    relation: &BDDFunction,
    next_state_vars: &BDDFunction,
) -> Result<BDDFunction, OutOfMemory> {
    partition.apply_exists(BooleanOperator::And, relation, next_state_vars)
}

/// Refines the partition w.r.t. the given signature by assigning block numbers
/// to signatures.
///
/// # Details
///
/// This function assumes that in the partition only a single block number is
/// assigned to each state, which should be by definition.
///
/// This function computes the new partition `P` such that:
///
/// > For all states s, t it holds that P(s) == P(t) iff signature(s) == signature(t)
#[allow(clippy::too_many_arguments)]
fn refine(
    manager_ref: &BDDManagerRef,
    cache: &mut FxHashMap<(BDDFunction, BDDFunction), BDDFunction>,
    claimed_blocks: &mut FxHashSet<u64>,
    next_fresh_id: &mut u64,
    block_variables_bdds: &[BDDFunction],
    next_state_variables: &[VarNo],
    signature: &BDDFunction,
    partition: &BDDFunction,
) -> Result<BDDFunction, MercError> {
    debug_assert!(
        check_partition_function(manager_ref, partition, next_state_variables)?,
        "The given partition function is not a valid partition function"
    );

    manager_ref.with_manager_shared(|manager| {
        let mut ctx = RefineContext {
            manager,
            cache,
            claimed_blocks,
            next_fresh_id,
            block_variables_bdds,
            next_state_variables,
        };

        Ok(BDDFunction::from_edge(
            manager,
            refine_edge(&mut ctx, signature.as_edge(manager), partition.as_edge(manager))?,
        ))
    })
}

/// Shared state threaded through the [refine_edge] recursion.
struct RefineContext<'a, 'id: 'a> {
    manager: &'a <BDDFunction as Function>::Manager<'id>,
    cache: &'a mut FxHashMap<(BDDFunction, BDDFunction), BDDFunction>,
    /// Old block ids already claimed by some signature this iteration; their states' partition BDD
    /// is returned as-is so the id is reused without reallocation.
    claimed_blocks: &'a mut FxHashSet<u64>,
    /// Strictly increasing counter for block ids allocated due to splits. Persisted across
    /// iterations so fresh ids never collide with ids carried over from the previous partition.
    next_fresh_id: &'a mut u64,
    block_variables_bdds: &'a [BDDFunction],
    next_state_variables: &'a [VarNo],
}

/// Recursive implementation of the [refine] function.
fn refine_edge<'id>(
    ctx: &mut RefineContext<'_, 'id>,
    signature: Ref<EdgeOfFunc<'id, BDDFunction>>,
    partition: Ref<EdgeOfFunc<'id, BDDFunction>>,
) -> Result<OwnEdgeOfFunc<'id, BDDFunction>, OutOfMemory> {
    let manager = ctx.manager;

    let plevel = match manager.get_node(partition) {
        Node::Terminal(terminal) => {
            if terminal == BDDTerminal::False {
                // In this case the state is not part of the partition function, so return empty.
                return Ok(manager.clone_edge(partition));
            }

            unreachable!("There are always block variables, so cannot be true terminal");
        }
        Node::Inner(node) => node.level(),
    };

    if let Some(cached) = ctx.cache.get(&(
        BDDFunction::from_edge(manager, manager.clone_edge(signature)),
        BDDFunction::from_edge(manager, manager.clone_edge(partition)),
    )) {
        return Ok(manager.clone_edge(cached.as_edge(manager)));
    }

    // topVar
    let lowest_level = {
        let slevel = match manager.get_node(signature) {
            Node::Terminal(_) => plevel,
            Node::Inner(node) => node.level(),
        };
        plevel.min(slevel)
    };

    let result = if ctx.next_state_variables.contains(&lowest_level) {
        // Match paths on the level s'_i, for irrelevant variables we take both paths.
        let (s_high, s_low) = match manager.get_node(signature) {
            Node::Inner(node) if node.level() == lowest_level => collect_children(node),
            _ => (signature, signature),
        };
        let (p_high, p_low) = match manager.get_node(partition) {
            Node::Inner(node) if node.level() == lowest_level => collect_children(node),
            _ => (partition, partition),
        };

        let low = refine_edge(ctx, s_low, p_low)?;
        let high = refine_edge(ctx, s_high, p_high)?;

        // 7. result := BDDnode(topVar, high, low)
        Ok(reduce(manager, lowest_level, high, low)?)
    } else {
        // Leaf: \sigma encodes the (action, target block) signature and P encodes the old block id.
        // Reuse the old id for the first signature that claims it (returning the partition edge
        // unchanged); any further signatures sharing that old block are splits and get a fresh id
        // from the persistent counter. The outer BDD memo above keys by (sig, partition) so
        // repeated (sig, partition) pairs reuse this assignment without re-decoding.
        let block_index = decode_block(manager, partition);
        if ctx.claimed_blocks.insert(block_index) {
            trace!("Reusing old block {block_index}");
            Ok(manager.clone_edge(partition))
        } else {
            let new_block_index = *ctx.next_fresh_id;
            *ctx.next_fresh_id += 1;
            trace!("Allocating fresh block {new_block_index} (split from {block_index})");
            Ok(encode_block(manager, ctx.block_variables_bdds, new_block_index)?)
        }
    }?;

    ctx.cache.insert(
        (
            BDDFunction::from_edge(manager, manager.clone_edge(signature)),
            BDDFunction::from_edge(manager, manager.clone_edge(partition)),
        ),
        BDDFunction::from_edge(manager, manager.clone_edge(result.borrowed())),
    );

    Ok(result)
}

/// Checks if the given BDD contains a function from `domain` to `range`.
///
/// # Details
///
/// Assumes that the domain variables all occur strictly before the range
/// variables.
fn check_partition_function(
    manager_ref: &BDDManagerRef,
    bdd: &BDDFunction,
    domain: &[VarNo],
) -> Result<bool, MercError> {
    manager_ref.with_manager_shared(|manager| {
        let mut cache = FxHashMap::default();

        check_partition_function_edge(manager, &mut cache, bdd.as_edge(manager), domain)
    })
}

/// The recursive implementation of [check_partition_function] on edges.
fn check_partition_function_edge<'id>(
    manager: &<BDDFunction as Function>::Manager<'id>,
    cache: &mut FxHashMap<BDDFunction, bool>,
    bdd: Ref<EdgeOfFunc<'id, BDDFunction>>,
    domain: &[VarNo],
) -> Result<bool, MercError> {
    if let Some(result) = cache.get(&BDDFunction::from_edge(manager, manager.clone_edge(bdd))) {
        return Ok(*result);
    }

    let bdd_level = match manager.get_node(bdd) {
        Node::Terminal(_terminal) => {
            return Ok(true);
        }
        Node::Inner(node) => node.level(),
    };

    let result = if let Some(domain_var) = domain.first() {
        if bdd_level > *domain_var {
            // Skipped levels, so catch up in the domain.
            check_partition_function_edge(manager, cache, bdd, &domain[1..])
        } else if bdd_level == *domain_var {
            let (high, low) = collect_children(manager.get_node(bdd).unwrap_inner());

            let high_result = check_partition_function_edge(manager, cache, high, &domain[1..])?;
            let low_result = check_partition_function_edge(manager, cache, low, &domain[1..])?;
            Ok(high_result && low_result)
        } else {
            // There are variables in the range that are not in the domain, so this cannot be a function from domain to range.
            return Ok(false);
        }
    } else {
        // The domain was completely visited, so now we check whether there is
        // exactly one assignment to the range variables.
        is_bdd_cube_edge(manager, bdd)
    }?;

    cache.insert(BDDFunction::from_edge(manager, manager.clone_edge(bdd)), result);
    Ok(result)
}

/// Checks if the given BDD is a cube, i.e., it represents a single bitvector over the given variables.
fn is_bdd_cube_edge<'id>(
    manager: &<BDDFunction as Function>::Manager<'id>,
    bdd: Ref<EdgeOfFunc<'id, BDDFunction>>,
) -> Result<bool, MercError> {
    match manager.get_node(bdd) {
        Node::Terminal(terminal) => match terminal {
            BDDTerminal::True => Ok(true),
            BDDTerminal::False => Ok(false),
        },
        Node::Inner(node) => {
            let (high, low) = collect_children(node);

            let high_is_cube = is_bdd_cube_edge(manager, high)?;
            let low_is_cube = is_bdd_cube_edge(manager, low)?;

            Ok(high_is_cube ^ low_is_cube) // Exactly one of them should be a cube.
        }
    }
}

/// Extend a transition relation to a larger domain by introducing x = x' for
/// the irrelevant variables.
///
/// # Details
///
/// The resulting transition can then be used without considering the support
/// explicitly.
pub(crate) fn extend_relation(
    manager_ref: &BDDManagerRef,
    relation: &BDDFunction,
    state_variables: &[VarNo],
    next_state_variables: &[VarNo],
    write_variables: &[VarNo],
) -> Result<BDDFunction, OutOfMemory> {
    manager_ref.with_manager_shared(|manager| {
        Ok(BDDFunction::from_edge(
            manager,
            extend_relation_edge(
                manager,
                relation.as_edge(manager),
                state_variables,
                next_state_variables,
                write_variables,
            )?,
        ))
    })
}

/// Extends a transition relation to the full domain by adding x = x' for every
/// state variable that is not written by the transition group.
fn extend_relation_edge<'id>(
    manager: &<BDDFunction as Function>::Manager<'id>,
    relation: Ref<EdgeOfFunc<'id, BDDFunction>>,
    state_variables: &[VarNo],
    next_state_variables: &[VarNo],
    write_variables: &[VarNo],
) -> Result<OwnEdgeOfFunc<'id, BDDFunction>, OutOfMemory> {
    debug_assert_eq!(
        state_variables.len(),
        next_state_variables.len(),
        "state and next-state domains should have matching arity"
    );

    // Build a single BDD encoding x = x' for all non-written variables.
    let mut eq = EdgeDropGuard::new(manager, BDDFunction::t_edge(manager));
    let f_edge = EdgeDropGuard::new(manager, BDDFunction::f_edge(manager));

    for (state_var, next_state_var) in state_variables.iter().zip(next_state_variables.iter()).rev() {
        if write_variables.contains(next_state_var) {
            continue;
        }

        let low = EdgeDropGuard::new(
            manager,
            reduce(
                manager,
                *next_state_var,
                manager.clone_edge(eq.borrowed()),
                manager.clone_edge(f_edge.borrowed()),
            )?,
        );
        let high = EdgeDropGuard::new(
            manager,
            reduce(
                manager,
                *next_state_var,
                manager.clone_edge(f_edge.borrowed()),
                manager.clone_edge(eq.borrowed()),
            )?,
        );
        eq = EdgeDropGuard::new(manager, reduce(manager, *state_var, low.into_edge(), high.into_edge())?);
    }

    BDDFunction::and_edge(manager, relation, eq.borrowed())
}

/// Encodes the given block number into a BDD using the given variables as bits.
///
/// # Details
///
/// Encodes the bits starting with the least significant bit, which is the
/// inverse of the encoding used in [crate::ldd_to_bdd]. The intuition is
/// (potentially) that the block numbers are often small numbers, so the most
/// significant bits are more likely to be 0 and these will collapse to singular
/// nodes at the bottom layers.
pub(crate) fn encode_block<'id>(
    manager: &<BDDFunction as Function>::Manager<'id>,
    variables: &[BDDFunction],
    block_no: u64,
) -> Result<OwnEdgeOfFunc<'id, BDDFunction>, OutOfMemory> {
    debug_assert!(
        variables.len() >= required_bits_64(block_no) as usize,
        "Not enough variables to encode block number {}",
        block_no
    );

    let mut result = EdgeDropGuard::new(manager, BDDFunction::t_edge(manager));
    let f_edge = EdgeDropGuard::new(manager, BDDFunction::f_edge(manager));

    for (i, var) in variables.iter().enumerate() {
        if block_no & (1 << i) != 0 {
            // bit is 1
            result = EdgeDropGuard::new(
                manager,
                BDDFunction::ite_edge(manager, var.as_edge(manager), result.borrowed(), f_edge.borrowed())?,
            );
        } else {
            // bit is 0
            result = EdgeDropGuard::new(
                manager,
                BDDFunction::ite_edge(manager, var.as_edge(manager), f_edge.borrowed(), result.borrowed())?,
            );
        }
    }

    Ok(result.into_edge())
}

/// Decodes the given block number from a BDD using the given variables as bits.
///
/// # Details
///
/// Should be the inverse of [encode_block].
fn decode_block<'id>(
    manager: &<BDDFunction as Function>::Manager<'id>,
    block: Ref<EdgeOfFunc<'id, BDDFunction>>,
) -> u64 {
    let mut result = 0u64;
    let mut mask = 1u64;
    let mut block = block;

    let f_edge = EdgeDropGuard::new(manager, BDDFunction::f_edge(manager));
    debug_assert!(block != f_edge.borrowed(), "decode_block called on the false terminal");
    while let Node::Inner(node) = manager.get_node(block) {
        let (b_high, b_low) = collect_children(node);
        // For a cube exactly one child is false; the other branch encodes the bit.
        if b_low != f_edge.borrowed() {
            debug_assert!(b_high == f_edge.borrowed(), "decode_block input is not a cube");
            block = b_low;
        } else {
            debug_assert!(b_high != f_edge.borrowed(), "decode_block input is not a cube");
            result |= mask;
            block = b_high;
        }
        mask <<= 1;
    }

    result
}

/// Display helper that prints all vectors represented by the given signature BDD as numbers, by decoding
/// the BDD layers as `bits`, see [crate::ldd_to_bdd].
pub struct SignatureDisplay<'a> {
    signature: &'a BDDFunction,

    /// The number of bits per state variable, the action bits and block bits.
    num_of_bits: &'a [u32],
    action_bits: u32,
    block_bits: u32,

    /// The variables that contribute to the signature.
    variables: &'a [VarNo],
}

impl<'a> SignatureDisplay<'a> {
    /// Creates a new partition display helper.
    fn new(
        signature: &'a BDDFunction,
        variables: &'a [VarNo],
        num_of_bits: &'a [u32],
        action_bits: u32,
        block_bits: u32,
    ) -> Self {
        Self {
            signature,
            num_of_bits,
            action_bits,
            block_bits,
            variables,
        }
    }
}

/// Display helper that prints all vectors represented by the given signature BDD as numbers, by decoding
/// the BDD layers as `bits`, see [crate::ldd_to_bdd].
pub struct PartitionDisplay<'a> {
    signature: &'a BDDFunction,

    /// The number of bits per state variable and block bits.
    num_of_bits: &'a [u32],
    block_bits: u32,

    /// The variables that contribute to the signature.
    variables: &'a Vec<VarNo>,
}

impl<'a> PartitionDisplay<'a> {
    /// Creates a new partition display helper.
    fn new(signature: &'a BDDFunction, variables: &'a Vec<VarNo>, num_of_bits: &'a [u32], block_bits: u32) -> Self {
        Self {
            signature,
            num_of_bits,
            block_bits,
            variables,
        }
    }
}

impl fmt::Display for PartitionDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Total number of bits for the state variables.
        let total_num_of_bits: u32 = self.num_of_bits.iter().sum();

        // We ignore the output cube, so just pass no variables.
        let mut first = true;
        for cube in CubeIterAll::with_variables(self.signature, self.variables) {
            let cube = cube.map_err(|_| fmt::Error)?;

            debug_assert_eq!(
                cube.len(),
                (total_num_of_bits + self.block_bits) as usize,
                "Unexpected number of bits found"
            );

            if !first {
                writeln!(f)?;
            }
            first = false;

            let (state_bits, block_bits) = cube.split_at(total_num_of_bits as usize);
            debug_assert_eq!(
                block_bits.len(),
                self.block_bits as usize,
                "Unexpected number of block bits found"
            );

            write!(
                f,
                "[{}] -> {}",
                ValuesIter::new(state_bits, self.num_of_bits).format(", "),
                to_block_index(block_bits)
            )?;
        }

        Ok(())
    }
}

impl fmt::Display for SignatureDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Total number of bits for the state variables.
        let total_num_of_bits: u32 = self.num_of_bits.iter().sum();

        // We ignore the output cube, so just pass no variables.
        let mut first = true;
        for cube in CubeIterAll::with_variables(self.signature, self.variables) {
            let cube = cube.map_err(|_| fmt::Error)?;

            debug_assert_eq!(
                cube.len(),
                (total_num_of_bits + self.action_bits + self.block_bits) as usize,
                "Unexpected number of bits found"
            );

            if !first {
                writeln!(f)?;
            }
            first = false;

            let (state_bits, rest) = cube.split_at(total_num_of_bits as usize);
            let (action_bits, block_bits) = rest.split_at(self.action_bits as usize);
            debug_assert_eq!(
                block_bits.len(),
                self.block_bits as usize,
                "Unexpected number of block bits found"
            );

            write!(
                f,
                "[{}] -> ({}, {})",
                ValuesIter::new(state_bits, self.num_of_bits).format(", "),
                to_value(action_bits),
                to_block_index(block_bits)
            )?;
        }

        Ok(())
    }
}

/// Reconstruct the block index represented by the bits, this uses the same encoding
/// as [encode_block]. This is least significant bit first.
fn to_block_index(bits: &[OptBool]) -> u64 {
    let mut value = 0u64;
    for (i, bit) in bits.iter().enumerate() {
        if *bit == OptBool::True {
            value |= 1 << i;
        }
    }

    value
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use oxidd::BooleanFunction;
    use oxidd::Function;
    use oxidd::Manager;
    use oxidd::ManagerRef;
    use oxidd::VarNo;
    use oxidd::bdd::BDDFunction;
    use oxidd::error::DuplicateVarName;
    use rand::RngExt;

    use merc_lts::LTS;
    use merc_lts::LtsBuilderMem;
    use merc_reduction::Equivalence;
    use merc_reduction::compare_lts;
    use merc_reduction::reduce_lts;
    use merc_utilities::Timing;
    use merc_utilities::random_test;

    use super::decode_block;
    use super::encode_block;
    use super::is_bdd_cube_edge;
    use crate::BDD_CACHE_CAPACITY;
    use crate::BDD_NODE_CAPACITY;
    use crate::LDD_CACHE_CAPACITY;
    use crate::LDD_NODE_CAPACITY;
    use crate::SymbolicLtsBdd;
    use crate::convert_symbolic_lts;
    use crate::convert_symbolic_lts_bdd;
    use crate::quotient_symbolic;
    use crate::random_bdd;
    use crate::random_symbolic_lts;
    use crate::read_symbolic_lts;
    use crate::required_bits_64;
    use crate::sigref_symbolic;

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_encode_blocks() {
        random_test(100, |rng| {
            let manager_ref = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);

            let block_number: u64 = rng.random();

            let num_of_bits = required_bits_64(block_number);
            let block_variable_names = (0..num_of_bits).map(|i| format!("b_{}", i)).collect::<Vec<String>>();

            // Create variables in the BDD manager
            let block_variables = manager_ref
                .with_manager_exclusive(|manager| -> Result<Range<VarNo>, DuplicateVarName> {
                    manager.add_named_vars(block_variable_names)
                })
                .unwrap();

            manager_ref.with_manager_shared(|manager| {
                let block_variables_bdds = block_variables
                    .map(|var_no| BDDFunction::var(manager, var_no))
                    .collect::<Result<Vec<BDDFunction>, oxidd::util::OutOfMemory>>()
                    .unwrap();

                let encoded = encode_block(manager, &block_variables_bdds, block_number).unwrap();
                let decoded = decode_block(manager, encoded.borrowed());

                assert_eq!(
                    block_number, decoded,
                    "Decoding the block number did not yield the original"
                );
            });
        })
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_sigref_split_signature() {
        random_test(100, |rng| {
            let ldd_manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);

            let lts = random_symbolic_lts(rng, &ldd_manager, 10, 5).unwrap();

            let bdd_manager = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);
            let lts_bdd = SymbolicLtsBdd::from_symbolic_lts(&ldd_manager, &bdd_manager, &lts).unwrap();

            let (_, _, expected_num_blocks) =
                sigref_symbolic(&bdd_manager, &lts_bdd, &Timing::new(), false, false, false, false).unwrap();

            // Create a separate manager since sigref_symbolic creates new block variables.
            let bdd_manager_split = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);
            let lts_bdd_split = SymbolicLtsBdd::from_symbolic_lts(&ldd_manager, &bdd_manager_split, &lts).unwrap();
            let (_, _, split_num_blocks) = sigref_symbolic(
                &bdd_manager_split,
                &lts_bdd_split,
                &Timing::new(),
                true,
                false,
                false,
                false,
            )
            .unwrap();

            assert_eq!(
                expected_num_blocks, split_num_blocks,
                "Split signature approach does not match actual signature refinement"
            );
        });
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_szymanski_symbolic_refinement() {
        let ldd_manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);
        let lts_bdd = read_symbolic_lts(
            &ldd_manager,
            include_bytes!("../../../../examples/lts/Szymanski_3-bit_lin_wait_alt.sym") as &[u8],
        )
        .unwrap();

        let bdd_manager = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);
        let lts_bdd = SymbolicLtsBdd::from_symbolic_lts(&ldd_manager, &bdd_manager, &lts_bdd).unwrap();

        let (_, _, num_of_blocks) =
            sigref_symbolic(&bdd_manager, &lts_bdd, &Timing::new(), false, false, false, false).unwrap();
        assert_eq!(
            num_of_blocks, 1791,
            "The Szymanski example has 1791 bisimulation blocks"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_symbolic_signature_refinement_abp() {
        let ldd_manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);
        let lts = read_symbolic_lts(
            &ldd_manager,
            include_bytes!("../../../../examples/lts/abp.sym") as &[u8],
        )
        .unwrap();

        let bdd_manager = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);
        let lts_bdd = SymbolicLtsBdd::from_symbolic_lts(&ldd_manager, &bdd_manager, &lts).unwrap();

        let (_, _, num_of_blocks) =
            sigref_symbolic(&bdd_manager, &lts_bdd, &Timing::new(), false, false, false, false).unwrap();
        assert_eq!(num_of_blocks, 68, "The ABP examples has 68 bisimulation blocks");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_is_cube() {
        random_test(100, |rng| {
            let ldd_manager = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);

            // Create variables in the BDD manager
            let vars: Vec<VarNo> =
                ldd_manager.with_manager_exclusive(|manager| manager.add_vars(8).collect::<Vec<VarNo>>());

            let bdd_vars = ldd_manager
                .with_manager_exclusive(|manager| {
                    vars.iter()
                        .map(|v| BDDFunction::var(manager, *v))
                        .collect::<Result<Vec<BDDFunction>, _>>()
                })
                .unwrap();

            let bdd = random_bdd(&ldd_manager, rng, &bdd_vars, 1).unwrap();

            ldd_manager.with_manager_shared(|manager| {
                assert!(
                    !bdd.satisfiable() || is_bdd_cube_edge(manager, bdd.as_edge(manager)).unwrap(),
                    "The bdd was created as a cube, so it should be a cube"
                );
            })
        })
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_sigref() {
        random_test(100, |rng| {
            let ldd_manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);

            let lts = random_symbolic_lts(rng, &ldd_manager, 10, 5).unwrap();

            let bdd_manager = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);
            let lts_bdd = SymbolicLtsBdd::from_symbolic_lts(&ldd_manager, &bdd_manager, &lts).unwrap();

            let mut builder = LtsBuilderMem::new(Vec::new(), Vec::new());
            let explicit_lts = convert_symbolic_lts(&ldd_manager, &mut builder, &lts).unwrap();
            let explicit_lts_reduced =
                reduce_lts(explicit_lts.clone(), Equivalence::StrongBisim, false, &Timing::new());

            let (partition, block_vars, _num_of_blocks) =
                sigref_symbolic(&bdd_manager, &lts_bdd, &Timing::new(), false, false, false, false).unwrap();

            let quotient_lts = quotient_symbolic(&bdd_manager, &lts_bdd, &partition, &block_vars).unwrap();

            let mut builder = LtsBuilderMem::new(Vec::new(), Vec::new());
            let symbolic_lts_reduced = convert_symbolic_lts_bdd(&bdd_manager, &mut builder, &quotient_lts).unwrap();

            println!(
                "Explicit LTS has {} states and {} transitions",
                explicit_lts.num_of_states(),
                explicit_lts.num_of_transitions()
            );

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
                "Both the explicit LTS and the one converted from the symbolic LTS should be bisimilar"
            );
        });
    }
}
