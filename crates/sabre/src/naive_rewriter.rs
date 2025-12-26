#![forbid(unsafe_code)]

use log::info;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_utilities::debug_trace;

use crate::AnnouncementInnermost;
use crate::MatchAnnouncement;
use crate::RewriteEngine;
use crate::RewriteSpecification;
use crate::RewritingStatistics;
use crate::set_automaton::SetAutomaton;
use crate::utilities::DataPositionIndexed;

/// Innermost Adaptive Pattern Matching Automaton (APMA) rewrite engine.
///
/// # Details
///
/// Implements the RewriteEngine trait. An APMA uses a modified SetAutomaton.
/// The [SetAutomaton::new] constructor has an 'apma' parameter. If it is set to
/// true. An APMA is created. Construction is almost identical with the
/// difference that no fresh goals are created. It only matches on the root
/// position.
pub struct NaiveRewriter {
    apma: SetAutomaton<AnnouncementInnermost>,
}

impl RewriteEngine for NaiveRewriter {
    fn rewrite(&mut self, t: &DataExpression) -> DataExpression {
        let mut stats = RewritingStatistics::default();

        let result = NaiveRewriter::rewrite_aux(&self.apma, t.copy(), &mut stats);

        info!(
            "{} rewrites, {} single steps and {} symbol comparisons",
            stats.recursions, stats.rewrite_steps, stats.symbol_comparisons
        );
        result
    }
}

impl NaiveRewriter {
    pub fn new(spec: &RewriteSpecification) -> NaiveRewriter {
        NaiveRewriter {
            apma: SetAutomaton::new(spec, AnnouncementInnermost::new, false),
        }
    }

    /// Function to rewrite a term 't'. The elements of the automaton 'states' and 'tp' are passed
    /// as separate parameters to satisfy the borrow checker.
    fn rewrite_aux(
        automaton: &SetAutomaton<AnnouncementInnermost>,
        t: DataExpressionRef<'_>,
        stats: &mut RewritingStatistics,
    ) -> DataExpression {
        let symbol = t.data_function_symbol();

        // Recursively call rewrite_aux on all the subterms.
        let mut arguments = vec![];
        for t in t.data_arguments() {
            arguments.push(NaiveRewriter::rewrite_aux(automaton, t, stats));
        }

        let nf: DataExpression = if arguments.is_empty() {
            symbol.protect().into()
        } else {
            DataApplication::with_args(&symbol, &arguments).into()
        };

        match NaiveRewriter::find_match(automaton, &nf, stats) {
            None => nf,
            Some((_announcement, ema)) => {
                let result = ema.rhs_stack.evaluate(&nf);
                debug_trace!("rewrote {} to {} using rule {}", nf, result, _announcement.rule);
                NaiveRewriter::rewrite_aux(automaton, result.copy(), stats)
            }
        }
    }

    /// Use the APMA to find a match for the given term.
    fn find_match<'a>(
        automaton: &'a SetAutomaton<AnnouncementInnermost>,
        t: &DataExpression,
        stats: &mut RewritingStatistics,
    ) -> Option<(&'a MatchAnnouncement, &'a AnnouncementInnermost)> {
        // Start at the initial state
        let mut state_index = 0;
        loop {
            let state = &automaton.states()[state_index];

            // Get the symbol at the position state.label
            let u = t.get_data_position(state.label());
            let symbol = u.data_function_symbol();

            // Get the transition for the label and check if there is a pattern match
            if let Some(transition) = automaton.transitions().get(&(state_index, symbol.operation_id())) {
                for (announcement, ema) in &transition.announcements {
                    let mut conditions_hold = true;

                    // Check conditions if there are any
                    if !ema.conditions.is_empty() {
                        conditions_hold = NaiveRewriter::check_conditions(automaton, &t.copy(), ema, stats);
                    }

                    // Check equivalence of subterms for non-linear patterns
                    'ec_check: for ec in &ema.equivalence_classes {
                        if ec.positions.len() > 1 {
                            let mut iter_pos = ec.positions.iter();
                            let first_pos = iter_pos.next().unwrap();
                            let first_term = t.get_data_position(first_pos);

                            for other_pos in iter_pos {
                                let other_term = t.get_data_position(other_pos);
                                if first_term != other_term {
                                    conditions_hold = false;
                                    break 'ec_check;
                                }
                            }
                        }
                    }

                    if conditions_hold {
                        // We found a matching pattern
                        return Some((announcement, ema));
                    }
                }

                // If there is no pattern match we check if the transition has a destination state
                if transition.destinations.is_empty() {
                    // If there is no destination state there is no pattern match
                    return None;
                }

                state_index = transition.destinations.first().unwrap().1;
            } else {
                // If there is no transition for the symbol, there is no match
                return None;
            }
        }
    }

    /// Given a term with head symbol 't_head' and subterms 't_subterms' and an EnhancedMatchAnnouncement,
    /// check if the conditions hold.
    fn check_conditions(
        automaton: &SetAutomaton<AnnouncementInnermost>,
        t: &DataExpressionRef<'_>,
        ema: &AnnouncementInnermost,
        stats: &mut RewritingStatistics,
    ) -> bool {
        for c in &ema.conditions {
            let rhs = c.lhs_term_stack.evaluate(t);
            let lhs = c.rhs_term_stack.evaluate(t);

            let rhs_normal = NaiveRewriter::rewrite_aux(automaton, rhs.copy(), stats);
            let lhs_normal = NaiveRewriter::rewrite_aux(automaton, lhs.copy(), stats);

            let holds = (lhs_normal == rhs_normal && c.equality) || (lhs_normal != rhs_normal && !c.equality);
            if !holds {
                return false;
            }
        }

        true
    }
}
