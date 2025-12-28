#![forbid(unsafe_code)]

use log::info;

use merc_aterm::storage::THREAD_TERM_POOL;
use merc_aterm::storage::ThreadTermPool;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_utilities::debug_trace;

use crate::RewriteSpecification;
use crate::matching::nonlinear::check_equivalence_classes;
use crate::set_automaton::MatchAnnouncement;
use crate::set_automaton::SetAutomaton;
use crate::utilities::AnnouncementSabre;
use crate::utilities::ConfigurationStack;
use crate::utilities::DataPositionIndexed;
use crate::utilities::SideInfo;
use crate::utilities::SideInfoType;

/// A shared trait for all the rewriters
pub trait RewriteEngine {
    /// Rewrites the given term into normal form.
    fn rewrite(&mut self, term: &DataExpression) -> DataExpression;
}

#[derive(Default)]
pub struct RewritingStatistics {
    /// Count the number of rewrite rules applied
    pub rewrite_steps: usize,
    /// Counts the number of times symbols are compared.
    pub symbol_comparisons: usize,
    /// The number of times rewrite is called recursively (to rewrite conditions etc)
    pub recursions: usize,
}

/// The Set Automaton based Rewrite Engine implementation.
pub struct SabreRewriter {
    automaton: SetAutomaton<AnnouncementSabre>,
}

impl RewriteEngine for SabreRewriter {
    fn rewrite(&mut self, term: &DataExpression) -> DataExpression {
        self.stack_based_normalise(term)
    }
}

impl SabreRewriter {
    pub fn new(spec: &RewriteSpecification) -> Self {
        let automaton = SetAutomaton::new(spec, AnnouncementSabre::new, false);

        SabreRewriter { automaton }
    }

    /// Function to rewrite a term. See the module documentation.
    pub fn stack_based_normalise(&mut self, t: &DataExpression) -> DataExpression {
        let mut stats = RewritingStatistics::default();

        let result = THREAD_TERM_POOL
            .with_borrow(|tp| SabreRewriter::stack_based_normalise_aux(tp, &self.automaton, t, &mut stats));

        info!(
            "{} rewrites, {} single steps and {} symbol comparisons",
            stats.recursions, stats.rewrite_steps, stats.symbol_comparisons
        );

        result
    }

    /// The _aux function splits the [TermPool] pool and the [SetAutomaton] to make borrow checker happy.
    /// We can now mutate the term pool and read the state and transition information at the same time
    fn stack_based_normalise_aux(
        tp: &ThreadTermPool,
        automaton: &SetAutomaton<AnnouncementSabre>,
        t: &DataExpression,
        stats: &mut RewritingStatistics,
    ) -> DataExpression {
        stats.recursions += 1;

        // We explore the configuration tree depth first using a ConfigurationStack
        let mut cs = ConfigurationStack::new(0, t);

        // Big loop until we know we have a normal form
        'outer: loop {
            // Inner loop so that we can easily break; to the next iteration
            'skip_point: loop {
                debug_trace!("{}", cs);

                // Check if there is any configuration leaf left to explore, if not we have found a normal form
                if let Some(leaf_index) = cs.get_unexplored_leaf() {
                    let leaf = &mut cs.stack[leaf_index];
                    let read_terms = cs.terms.read();
                    let leaf_term = &read_terms[leaf_index];

                    match ConfigurationStack::pop_side_branch_leaf(&mut cs.side_branch_stack, leaf_index) {
                        None => {
                            // Observe a symbol according to the state label of the set automaton.
                            let pos: DataExpressionRef =
                                leaf_term.get_data_position(automaton.states()[leaf.state].label());

                            let function_symbol = pos.data_function_symbol();
                            stats.symbol_comparisons += 1;

                            // Get the transition belonging to the observed symbol
                            if let Some(tr) = automaton
                                .transitions()
                                .get(&(leaf.state, function_symbol.operation_id()))
                            {
                                // Loop over the match announcements of the transition
                                for (announcement, annotation) in &tr.announcements {
                                    if annotation.conditions.is_empty() && annotation.equivalence_classes.is_empty() {
                                        if annotation.is_duplicating {
                                            debug_trace!("Delaying duplicating rule {}", announcement.rule);

                                            // We do not want to apply duplicating rules straight away
                                            cs.side_branch_stack.push(SideInfo {
                                                corresponding_configuration: leaf_index,
                                                info: SideInfoType::DelayedRewriteRule(announcement, annotation),
                                            });
                                        } else {
                                            // For a rewrite rule that is not duplicating or has a condition we just apply it straight away
                                            drop(read_terms);
                                            SabreRewriter::apply_rewrite_rule(
                                                tp,
                                                automaton,
                                                announcement,
                                                annotation,
                                                leaf_index,
                                                &mut cs,
                                                stats,
                                            );
                                            break 'skip_point;
                                        }
                                    } else {
                                        // We delay the condition checks
                                        debug_trace!("Delaying condition check for rule {}", announcement.rule);

                                        cs.side_branch_stack.push(SideInfo {
                                            corresponding_configuration: leaf_index,
                                            info: SideInfoType::EquivalenceAndConditionCheck(announcement, annotation),
                                        });
                                    }
                                }

                                drop(read_terms);
                                if tr.destinations.is_empty() {
                                    // If there is no destination we are done matching and go back to the previous
                                    // configuration on the stack with information on the side stack.
                                    // Note, it could be that we stay at the same configuration and apply a rewrite
                                    // rule that was just discovered whilst exploring this configuration.
                                    let prev = cs.get_prev_with_side_info();
                                    cs.current_node = prev;
                                    if let Some(n) = prev {
                                        cs.jump_back(n, tp);
                                    }
                                } else {
                                    // Grow the bud; if there is more than one destination a SideBranch object will be placed on the side stack
                                    let tr_slice = tr.destinations.as_slice();
                                    cs.grow(leaf_index, tr_slice);
                                }
                            } else {
                                let prev = cs.get_prev_with_side_info();
                                cs.current_node = prev;
                                if let Some(n) = prev {
                                    drop(read_terms);
                                    cs.jump_back(n, tp);
                                }
                            }
                        }
                        Some(sit) => {
                            match sit {
                                SideInfoType::SideBranch(sb) => {
                                    // If there is a SideBranch pick the next child configuration
                                    drop(read_terms);
                                    cs.grow(leaf_index, sb);
                                }
                                SideInfoType::DelayedRewriteRule(announcement, annotation) => {
                                    drop(read_terms);
                                    // apply the delayed rewrite rule
                                    SabreRewriter::apply_rewrite_rule(
                                        tp,
                                        automaton,
                                        announcement,
                                        annotation,
                                        leaf_index,
                                        &mut cs,
                                        stats,
                                    );
                                }
                                SideInfoType::EquivalenceAndConditionCheck(announcement, annotation) => {
                                    // Apply the delayed rewrite rule if the conditions hold
                                    if check_equivalence_classes(leaf_term, &annotation.equivalence_classes)
                                        && SabreRewriter::conditions_hold(
                                            tp,
                                            automaton,
                                            announcement,
                                            annotation,
                                            leaf_term,
                                            stats,
                                        )
                                    {
                                        drop(read_terms);
                                        SabreRewriter::apply_rewrite_rule(
                                            tp,
                                            automaton,
                                            announcement,
                                            annotation,
                                            leaf_index,
                                            &mut cs,
                                            stats,
                                        );
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // No configuration left to explore, we have found a normal form
                    break 'outer;
                }
            }
        }

        cs.compute_final_term(tp)
    }

    /// Apply a rewrite rule and prune back
    fn apply_rewrite_rule(
        tp: &ThreadTermPool,
        automaton: &SetAutomaton<AnnouncementSabre>,
        announcement: &MatchAnnouncement,
        annotation: &AnnouncementSabre,
        leaf_index: usize,
        cs: &mut ConfigurationStack<'_>,
        stats: &mut RewritingStatistics,
    ) {
        stats.rewrite_steps += 1;

        let read_terms = cs.terms.read();
        let leaf_subterm: &DataExpressionRef<'_> = &read_terms[leaf_index];

        // Computes the new subterm of the configuration
        let new_subterm = annotation
            .rhs_term_stack
            .evaluate(&leaf_subterm.get_data_position(&announcement.position));

        debug_trace!(
            "rewrote {} to {} using rule {}",
            &leaf_subterm,
            &new_subterm,
            announcement.rule
        );

        // The match announcement tells us how far we need to prune back.
        let prune_point = leaf_index - announcement.symbols_seen;
        drop(read_terms);
        cs.prune(tp, automaton, prune_point, new_subterm);
    }

    /// Checks conditions and subterm equality of non-linear patterns.
    fn conditions_hold(
        tp: &ThreadTermPool,
        automaton: &SetAutomaton<AnnouncementSabre>,
        announcement: &MatchAnnouncement,
        annotation: &AnnouncementSabre,
        subterm: &DataExpressionRef<'_>,
        stats: &mut RewritingStatistics,
    ) -> bool {
        for c in &annotation.conditions {
            let subterm = subterm.get_data_position(&announcement.position);

            let rhs: DataExpression = c.rhs_term_stack.evaluate(&subterm);
            let lhs: DataExpression = c.lhs_term_stack.evaluate(&subterm);

            // Equality => lhs == rhs.
            if !c.equality || lhs != rhs {
                let rhs_normal = SabreRewriter::stack_based_normalise_aux(tp, automaton, &rhs, stats);
                let lhs_normal = SabreRewriter::stack_based_normalise_aux(tp, automaton, &lhs, stats);

                // If lhs != rhs && !equality OR equality && lhs == rhs.
                if (!c.equality && lhs_normal == rhs_normal) || (c.equality && lhs_normal != rhs_normal) {
                    return false;
                }
            }
        }

        true
    }
}
