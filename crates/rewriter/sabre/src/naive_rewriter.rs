#![forbid(unsafe_code)]

use log::info;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_utilities::debug_trace;

use crate::AnnouncementInnermost;
use crate::RewriteEngine;
use crate::RewriteSpecification;
use crate::RewritingStatistics;
use crate::set_automaton::MatchResult;
use crate::set_automaton::SetAutomaton;
use crate::utilities::DataPositionIndexed;

/// Naive Adaptive Pattern Matching Automaton (APMA) rewrite engine
/// implementation for testing purposes.
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
        // Arguments are normalised before the term is matched, so only root matches are needed.
        // The set automaton for all positions has a destination per subterm to explore, which
        // [NaiveRewriter::find_match] cannot follow: it walks a single chain of states and would
        // cycle through them forever.
        NaiveRewriter {
            apma: SetAutomaton::new(spec, AnnouncementInnermost::new, true),
        }
    }

    /// Rewrites `t` to normal form by first normalising every subterm, then matching the result
    /// at the root: naive because it never observes a symbol below the root before its
    /// subterms are already fully rewritten.
    fn rewrite_aux(
        automaton: &SetAutomaton<AnnouncementInnermost>,
        t: DataExpressionRef<'_>,
        stats: &mut RewritingStatistics,
    ) -> DataExpression {
        // A variable has no head symbol to match on and is its own normal form.
        let Some(symbol) = t.try_data_function_symbol() else {
            return t.protect();
        };

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
            Some(MatchResult::Native(result)) => {
                debug_trace!("native rewrote {} to {}", nf, result);
                result
            }
            Some(MatchResult::Rule(_announcement, ema)) => {
                let result = ema.rhs_stack.evaluate(&nf);
                debug_trace!("rewrote {} to {} using rule {}", nf, result, _announcement.rule);
                NaiveRewriter::rewrite_aux(automaton, result.copy(), stats)
            }
        }
    }

    /// Use the APMA to find a match for the given term: either a rewrite
    /// rule, or — when the term's head symbol is a machine-word operation —
    /// the natively-evaluated result.
    fn find_match<'a>(
        automaton: &'a SetAutomaton<AnnouncementInnermost>,
        t: &DataExpression,
        stats: &mut RewritingStatistics,
    ) -> Option<MatchResult<'a, AnnouncementInnermost>> {
        // Start at the initial state
        let mut state_index = 0;
        loop {
            let state = &automaton.states()[state_index];

            // Get the symbol at the position state.label; a variable there matches no pattern.
            let u = t.get_data_position(state.label());
            let symbol = u.try_data_function_symbol()?;

            // Get the transition for the label and check if there is a pattern match
            {
                let transition = automaton.get_transition(state_index, symbol.operation_id())?;

                // See InnermostRewriter::find_match: only the very first transition
                // observes the term's own head symbol at position ε.
                if state_index == 0
                    && let Some(op) = transition.native
                {
                    return op.evaluate(t.data_arguments()).map(MatchResult::Native);
                }

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
                            let first_pos = iter_pos
                                .next()
                                .expect("An equivalence class with more than one position has a first position");
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
                        return Some(MatchResult::Rule(announcement, ema));
                    }
                }

                // If there is no pattern match we check if the transition has a destination state
                if transition.destinations.is_empty() {
                    // If there is no destination state there is no pattern match
                    return None;
                }

                state_index = transition
                    .destinations
                    .first()
                    .expect("A transition with destinations has a first destination")
                    .1;
            }
        }
    }

    /// Checks whether every condition of `ema` holds for the (already normalised) subterms of `t`.
    fn check_conditions(
        automaton: &SetAutomaton<AnnouncementInnermost>,
        t: &DataExpressionRef<'_>,
        ema: &AnnouncementInnermost,
        stats: &mut RewritingStatistics,
    ) -> bool {
        for c in &ema.conditions {
            let lhs = c.lhs_term_stack.evaluate(t);
            let rhs = c.rhs_term_stack.evaluate(t);

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
