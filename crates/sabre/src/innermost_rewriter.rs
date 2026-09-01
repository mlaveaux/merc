use log::info;

use merc_aterm::storage::THREAD_TERM_POOL;
use merc_aterm::storage::ThreadTermPool;
use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_data::DataVariableRef;
use merc_data::is_data_machine_number;

use crate::RewriteEngine;
use crate::RewriteSpecification;
use crate::RewritingStatistics;
use crate::Rule;
use crate::matching::condition_cache::ConditionCache;
use crate::matching::condition_cache::build_condition_cache;
use crate::matching::condition_cache::check_conditions_with_cache;
use crate::matching::conditions::EMACondition;
use crate::matching::conditions::extend_conditions;
use crate::matching::nonlinear::EquivalenceClass;
use crate::matching::nonlinear::check_equivalence_classes;
use crate::matching::nonlinear::derive_equivalence_classes;
use crate::set_automaton::MatchResult;
use crate::set_automaton::SetAutomaton;
use crate::set_automaton::machine_number_symbol;
use crate::utilities::Config;
use crate::utilities::DataPositionIndexed;
use crate::utilities::EmptySubstitution;
use crate::utilities::InnermostStack;
use crate::utilities::RewriteSubstitution;
use crate::utilities::TermStack;
use crate::utilities::TermStackBuilder;
use merc_utilities::debug_trace;

impl RewriteEngine for InnermostRewriter {
    fn rewrite(&mut self, t: &DataExpression) -> DataExpression {
        self.rewrite_with_statistics(t).0
    }

    fn rewrite_with<S: RewriteSubstitution>(&mut self, t: &DataExpression, sigma: &S) -> DataExpression {
        self.rewrite_under_with_statistics(t, sigma).0
    }
}

impl InnermostRewriter {
    /// Rewrites `t` to normal form and returns the normal form together with the
    /// [RewritingStatistics] gathered while doing so, most notably the number of
    /// applied rewrite steps.
    pub fn rewrite_with_statistics(&mut self, t: &DataExpression) -> (DataExpression, RewritingStatistics) {
        self.rewrite_under_with_statistics(t, &EmptySubstitution)
    }

    /// Replaces the free variables of `t` according to `sigma` and rewrites the result to normal
    /// form, returning it together with the gathered [RewritingStatistics].
    ///
    /// `sigma` must map every variable to a term already in normal form: replacements are spliced
    /// in without being rewritten again.
    pub fn rewrite_under_with_statistics<S: RewriteSubstitution>(
        &mut self,
        t: &DataExpression,
        sigma: &S,
    ) -> (DataExpression, RewritingStatistics) {
        let mut stats = RewritingStatistics::default();

        debug_trace!("input: {}", t);

        let result = THREAD_TERM_POOL.with(|tp| {
            InnermostRewriter::rewrite_aux(tp, &mut self.stack, &mut self.builder, &mut stats, &self.apma, t, sigma)
        });

        info!(
            "{} rewrites, {} single steps and {} symbol comparisons",
            stats.recursions, stats.rewrite_steps, stats.symbol_comparisons
        );
        (result, stats)
    }

    /// Creates a new InnermostRewriter from the given rewrite specification.
    pub fn new(spec: &RewriteSpecification) -> InnermostRewriter {
        let apma = SetAutomaton::new(spec, AnnouncementInnermost::new, true);

        InnermostRewriter {
            apma,
            stack: InnermostStack::default(),
            builder: TermStackBuilder::new(),
        }
    }

    /// Rewrites `input_term` to normal form, replacing its free variables by their image under
    /// `sigma`.
    ///
    /// `automaton`, `stack` and `builder` are passed as separate parameters, rather than bundled
    /// behind `&mut self`, so the borrow checker allows each to be borrowed independently.
    ///
    /// Uses a stack of terms and configurations to avoid recursion and to keep track of terms in
    /// normal form without explicit tagging. Each configuration is one of:
    ///     - Return(): Returns the top of the stack.
    ///     - Rewrite(index): Rewrites the top of the term stack and places the result at the given
    ///       index.
    ///     - Construct(symbol, arity, index): Constructs `symbol` applied to the `arity` terms at
    ///       the top of the stack and places the result at the given index.
    ///
    /// Free variables of `input_term` are replaced by their image under `sigma` when they are
    /// reached, which is the point where the substitution is consumed: every term that is
    /// matched, constructed or used to instantiate a condition afterwards is already
    /// substituted.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rewrite_aux<S: RewriteSubstitution>(
        tp: &ThreadTermPool,
        stack: &mut InnermostStack,
        builder: &mut TermStackBuilder,
        stats: &mut RewritingStatistics,
        automaton: &SetAutomaton<AnnouncementInnermost>,
        input_term: &DataExpression,
        sigma: &S,
    ) -> DataExpression {
        stats.recursions += 1;
        {
            let mut write_terms = stack.terms.write();
            let mut write_configs = stack.configs.write();

            // Push the result term to the stack.
            let top_of_stack = write_terms.len();
            write_configs.push(Config::Return());
            write_terms.push(None);
            InnermostStack::add_rewrite(&mut write_configs, &mut write_terms, input_term.copy(), top_of_stack);
        }

        loop {
            debug_trace!("{}", stack);

            let mut write_configs = stack.configs.write();
            if let Some(config) = write_configs.pop() {
                match config {
                    Config::Rewrite(result) => {
                        let mut write_terms = stack.terms.write();
                        let term = write_terms.pop().unwrap().unwrap();

                        // A machine number is a value in normal form; it has no head function
                        // symbol to decompose, so place it directly at the result index.
                        if is_data_machine_number(&term) {
                            // Safety: term is stored in the container on the same line.
                            write_terms[result] = Some(unsafe { write_terms.protect(&term) }.into());
                            drop(write_configs);
                            continue;
                        }

                        if let Some(symbol) = term.try_data_function_symbol() {
                            let arguments = term.data_arguments();

                            // For all the argument we reserve space on the stack.
                            let top_of_stack = write_terms.len();
                            for _ in 0..arguments.len() {
                                write_terms.push(Default::default());
                            }

                            // Safety: symbol is stored in the container on the next line.
                            let symbol = unsafe { write_configs.protect(&symbol) };
                            InnermostStack::add_result(&mut write_configs, symbol.into(), arguments.len(), result);
                            for (offset, arg) in arguments.into_iter().enumerate() {
                                InnermostStack::add_rewrite(
                                    &mut write_configs,
                                    &mut write_terms,
                                    arg,
                                    top_of_stack + offset,
                                );
                            }
                            drop(write_configs);
                        } else {
                            // A variable has no head symbol to match on. It is replaced by its
                            // image under sigma, which is assumed to be in normal form, so it
                            // goes straight into the result slot without being constructed.
                            // Variables outside the domain of sigma are their own normal form.
                            let variable: DataVariableRef<'_> = term.copy().into();
                            let replacement = sigma.get(&variable).unwrap_or_else(|| term.copy());

                            // Safety: replacement is stored in the container on the same line.
                            write_terms[result] = Some(unsafe { write_terms.protect(&replacement) }.into());
                            drop(write_configs);
                        }
                    }
                    Config::Construct(symbol, arity, index) => {
                        // Take the last arity arguments.
                        let mut write_terms = stack.terms.write();
                        let length = write_terms.len();

                        let arguments = &write_terms[length - arity..];

                        let term: DataExpression = if arguments.is_empty() {
                            symbol.protect().into()
                        } else {
                            DataApplication::with_iter(&symbol, arguments.len(), arguments.iter().flatten()).into()
                        };

                        // Remove the arguments from the stack.
                        write_terms.drain(length - arity..);
                        drop(write_terms);
                        drop(write_configs);

                        match InnermostRewriter::find_match(tp, stack, builder, stats, automaton, &term.copy()) {
                            Some(MatchResult::Native(result)) => {
                                debug_trace!("native rewrite {} => {}", term, result);

                                let mut write_terms = stack.terms.write();
                                // Safety: result is stored in the container on the same line.
                                write_terms[index] = Some(unsafe { write_terms.protect(&result) }.into());
                                stats.rewrite_steps += 1;
                            }
                            Some(MatchResult::Rule(_announcement, annotation)) => {
                                debug_trace!(
                                    "rewrite {} => {} using rule {}",
                                    term,
                                    annotation.rhs_stack.evaluate(&term),
                                    _announcement.rule
                                );

                                // Reacquire the write access and add the matching RHSStack.
                                let mut write_terms = stack.terms.write();
                                let mut write_configs = stack.configs.write();
                                InnermostStack::integrate(
                                    &mut write_configs,
                                    &mut write_terms,
                                    &annotation.rhs_stack,
                                    &term.copy(),
                                    index,
                                );
                                stats.rewrite_steps += 1;
                            }
                            None => {
                                // Add the term on the stack.
                                let mut write_terms = stack.terms.write();
                                // Safety: term is stored in the container on the same line.
                                write_terms[index] = Some(unsafe { write_terms.protect(&term) }.into());
                            }
                        }
                    }
                    Config::Term(term, index) => {
                        // A constant carried by a right-hand side (a machine number)
                        // is already in normal form: place it at its index directly.
                        let mut write_terms = stack.terms.write();
                        // Safety: term is stored in the container on the same line.
                        write_terms[index] = Some(unsafe { write_terms.protect(&term) }.into());
                        drop(write_terms);
                        drop(write_configs);
                    }
                    Config::Return() => {
                        let mut write_terms = stack.terms.write();

                        return write_terms
                            .pop()
                            .expect("The result should be the last element on the stack")
                            .expect("The result should be Some")
                            .protect();
                    }
                }

                if cfg!(debug_assertions) {
                    let read_configs = stack.configs.read();
                    for (index, term) in stack.terms.read().iter().enumerate() {
                        if term.is_none() {
                            debug_assert!(
                                read_configs.iter().any(|x| {
                                    match x {
                                        Config::Construct(_, _, result) => index == *result,
                                        Config::Rewrite(result) => index == *result,
                                        Config::Term(_, result) => index == *result,
                                        Config::Return() => true,
                                    }
                                }),
                                "The default term at index {index} is not a result of any operation."
                            );
                        }
                    }
                }
            }
        }
    }

    /// Use the APMA to find a match for the given term: either a rewrite rule,
    /// or — when the term's head symbol is a machine-word operation — the
    /// natively-evaluated result.
    fn find_match<'a>(
        tp: &ThreadTermPool,
        stack: &mut InnermostStack,
        builder: &mut TermStackBuilder,
        stats: &mut RewritingStatistics,
        automaton: &'a SetAutomaton<AnnouncementInnermost>,
        t: &DataExpressionRef<'_>,
    ) -> Option<MatchResult<'a, AnnouncementInnermost>> {
        // Start at the initial state
        let mut state_index = 0;
        loop {
            let state = &automaton.states()[state_index];

            // Get the symbol at the position state.label; a variable there matches no pattern.
            stats.symbol_comparisons += 1;
            let pos = t.get_data_position(state.label());

            // A machine number carries no function symbol, so it is matched under
            // the shared stand-in symbol the automaton reserves for them.
            let operation_id = if is_data_machine_number(&pos) {
                machine_number_symbol().operation_id()
            } else {
                pos.try_data_function_symbol()?.operation_id()
            };

            // Get the transition for the label and check if there is a pattern match
            {
                let transition = automaton.get_transition(state_index, operation_id)?;

                // The very first transition observes the term's own head symbol
                // (state 0's label is always the root position ε). That is the
                // only point at which `native` refers to the term being matched
                // as a whole, rather than to some other subterm the automaton
                // happens to inspect while narrowing down candidate rules.
                if state_index == 0
                    && let Some(op) = transition.native
                {
                    return op.evaluate(t.data_arguments()).map(MatchResult::Native);
                }

                for (announcement, annotation) in &transition.announcements {
                    if check_equivalence_classes(t, &annotation.equivalence_classes)
                        && InnermostRewriter::check_conditions(tp, stack, builder, stats, automaton, annotation, t)
                    {
                        // We found a matching pattern
                        return Some(MatchResult::Rule(announcement, annotation));
                    }
                }

                // If there is no pattern match we check if the transition has a destination state
                if transition.destinations.is_empty() {
                    // If there is no destination state there is no pattern match
                    return None;
                }

                state_index = transition.destinations.first().unwrap().1;
            }
        }
    }

    /// Checks whether the condition holds for given match announcement.
    ///
    /// The condition sides are built from the matched subterms, which are already substituted
    /// normal forms, so the nested normalisations must not apply the substitution a second time.
    fn check_conditions(
        tp: &ThreadTermPool,
        stack: &mut InnermostStack,
        builder: &mut TermStackBuilder,
        stats: &mut RewritingStatistics,
        automaton: &SetAutomaton<AnnouncementInnermost>,
        announcement: &AnnouncementInnermost,
        t: &DataExpressionRef<'_>,
    ) -> bool {
        if let Some(cache) = &announcement.condition_cache {
            // A cached subterm may itself need normalising, which recurses back
            // into this same rewrite loop through the closure below.
            return check_conditions_with_cache(cache, t, builder, &mut |term, builder| {
                InnermostRewriter::rewrite_aux(tp, stack, builder, stats, automaton, term, &EmptySubstitution)
            });
        }

        for c in &announcement.conditions {
            let rhs: DataExpression = c.rhs_term_stack.evaluate_with(t, builder);
            let lhs: DataExpression = c.lhs_term_stack.evaluate_with(t, builder);

            let rhs_normal =
                InnermostRewriter::rewrite_aux(tp, stack, builder, stats, automaton, &rhs, &EmptySubstitution);
            let lhs_normal =
                InnermostRewriter::rewrite_aux(tp, stack, builder, stats, automaton, &lhs, &EmptySubstitution);

            if (lhs_normal != rhs_normal && c.equality) || (lhs_normal == rhs_normal && !c.equality) {
                return false;
            }
        }

        true
    }
}

/// Innermost Adaptive Pattern Matching Automaton (APMA) rewrite engine.
pub struct InnermostRewriter {
    apma: SetAutomaton<AnnouncementInnermost>,
    stack: InnermostStack,
    builder: TermStackBuilder,
}

pub struct AnnouncementInnermost {
    /// Positions in the pattern with the same variable, for non-linear patterns
    pub equivalence_classes: Vec<EquivalenceClass>,

    /// Conditions for the left hand side.
    pub conditions: Vec<EMACondition>,

    /// A cache for the (rare) rules whose conditions share subterms across
    /// each other, checked instead of `conditions` when present; see
    /// [crate::matching::condition_cache].
    pub condition_cache: Option<ConditionCache>,

    /// The innermost stack for the right hand side of the rewrite rule.
    pub rhs_stack: TermStack,
}

impl AnnouncementInnermost {
    pub fn new(rule: &Rule) -> AnnouncementInnermost {
        AnnouncementInnermost {
            conditions: extend_conditions(rule),
            condition_cache: build_condition_cache(rule),
            equivalence_classes: derive_equivalence_classes(rule),
            rhs_stack: TermStack::new(rule),
        }
    }
}
