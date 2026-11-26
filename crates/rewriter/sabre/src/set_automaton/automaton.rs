use std::collections::VecDeque;
use std::convert::Infallible;
use std::fmt::Debug;
use std::ops::ControlFlow;
use std::time::Instant;

use ahash::HashMap;
use ahash::HashSet;
use log::debug;
use log::info;
use log::log_enabled;
use log::trace;
use log::warn;
use merc_aterm::Term;
use merc_data::BinderType;
use merc_data::DataAbstractionRef;
use merc_data::DataApplicationRef;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_data::DataExpressionVisitor;
use merc_data::DataFunctionSymbol;
use merc_data::DataFunctionSymbolRef;
use merc_data::DataVariable;
use merc_data::MachineNumberRef;
use merc_data::MachineWordOp;
use merc_data::is_data_application;
use merc_data::is_data_binder;
use merc_data::is_data_function_symbol;
use merc_data::is_data_variable;
use merc_data::is_data_where_clause;
use merc_utilities::Step;
use merc_utilities::Visit;
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use smallvec::smallvec;

use crate::rewrite_specification::RewriteSpecification;
use crate::rewrite_specification::Rule;
use crate::set_automaton::free_variables;
use crate::utilities::DataPosition;

use super::DotFormatter;
use super::MatchGoal;

/// The Set Automaton used to find all matching patterns in a term.
pub struct SetAutomaton<T> {
    states: Vec<State>,
    transitions: FxHashMap<(usize, usize), Transition<T>>,
}

/// A match announcement contains the rule that can be announced as a match at
/// the given position.
///
/// `symbols_seen` is internally used to keep track of how many symbols have
/// been observed so far. Since these symbols have a unique number this can be
/// used to speed up certain operations.
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MatchAnnouncement {
    pub rule: Rule,
    pub position: DataPosition,
    pub symbols_seen: usize,
}

/// Represents a transition in the [SetAutomaton].
#[derive(Clone)]
pub struct Transition<T> {
    pub symbol: DataFunctionSymbol,
    pub announcements: SmallVec<[(MatchAnnouncement, T); 1]>,
    pub destinations: SmallVec<[(DataPosition, usize); 1]>,

    /// Set when `symbol` is a machine-word operation, which is evaluated
    /// natively instead of being governed by rewrite rules.
    pub native: Option<MachineWordOp>,
}

/// The result of walking the automaton to find a match for a term, shared by
/// every rewrite engine's `find_match`.
pub enum MatchResult<'a, T> {
    /// An ordinary rewrite rule matched at the term's root.
    Rule(&'a MatchAnnouncement, &'a T),
    /// The term's head symbol is a machine-word operation, and it evaluated
    /// (all its arguments were concrete) to this result.
    Native(DataExpression),
}

/// Represents a match obligation in the [SetAutomaton].
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MatchObligation {
    pub pattern: DataExpression,
    pub position: DataPosition,
}

impl MatchObligation {
    /// Creates a new match obligation for `pattern` at `position`.
    pub fn new(pattern: DataExpression, position: DataPosition) -> Self {
        MatchObligation { pattern, position }
    }
}

/// Represents either the initial state or a set of match goals in the
/// [SetAutomaton].
///
/// This is only used during construction to avoid creating all the goals for
/// the initial state.
#[derive(Debug)]
enum GoalsOrInitial {
    InitialState,
    Goals(Vec<MatchGoal>),
}

impl<M> SetAutomaton<M> {
    /// Creates a new SetAutomaton from the given rewrite specification. If
    /// `apma` is true an Adaptive Pattern Matching Automaton is created,
    /// meaning that it only finds matches at the root position.
    ///
    /// The `annotate` function is used to create the annotation for each match
    /// announcement, accommodating different annotation types across rewrite engines.
    pub fn new<F>(spec: &RewriteSpecification, annotate: F, apma: bool) -> SetAutomaton<M>
    where
        F: Fn(&Rule) -> M,
    {
        Self::build(spec, annotate, apma, false)
    }

    /// Like [SetAutomaton::new] for an APMA, but also keeps the rules with `exists`/`forall`
    /// binders in their right-hand side, for a rewriter that evaluates
    /// [Config::Binder](crate::utilities::Config::Binder).
    pub fn with_binders<F>(spec: &RewriteSpecification, annotate: F) -> SetAutomaton<M>
    where
        F: Fn(&Rule) -> M,
    {
        Self::build(spec, annotate, true, true)
    }

    fn build<F>(spec: &RewriteSpecification, annotate: F, apma: bool, binders: bool) -> SetAutomaton<M>
    where
        F: Fn(&Rule) -> M,
    {
        let start = Instant::now();

        // States are labelled s0, s1, s2, etcetera. state_counter keeps track of count.
        let mut state_counter: usize = 1;

        // Remove rules that we cannot deal with
        let supported_rules: Vec<Rule> = spec
            .rewrite_rules()
            .iter()
            .filter(|rule| is_supported_rule_with(rule, binders))
            .map(Rule::clone)
            .collect();

        // Find the indices of all the function symbols.
        let symbols = {
            let mut symbols = HashMap::default();

            for rule in &supported_rules {
                find_symbols(&rule.lhs.copy(), &mut symbols);
                find_symbols(&rule.rhs.copy(), &mut symbols);

                for cond in &rule.conditions {
                    find_symbols(&cond.lhs.copy(), &mut symbols);
                    find_symbols(&cond.rhs.copy(), &mut symbols);
                }
            }

            // The term being rewritten may contain machine numbers even when no
            // rule mentions one, so the stand-in symbol always needs a transition
            // for matching to be able to step over such a position.
            add_symbol(machine_number_symbol(), 0, &mut symbols);

            // Machine-word operations have no rules of their own — they are
            // `defined_by_code` — so without this they would have no transition
            // at all and could never be recognised as native ops.
            for (symbol, op) in spec.native_symbols() {
                add_symbol(symbol.clone(), op.arity(), &mut symbols);
            }

            symbols
        };

        // Resolves a symbol to its native operation by identity (its hash-consed
        // pool index), not by re-parsing its name. `spec.native_symbols()`
        // already paired each symbol with its operation exactly once; this
        // table lets the state × symbol loop below look that up for every
        // state without ever touching a string again.
        let native_ops: FxHashMap<usize, MachineWordOp> = spec
            .native_symbols()
            .iter()
            .map(|(symbol, op)| (symbol.operation_id(), *op))
            .collect();

        for (index, (symbol, arity)) in symbols.iter().enumerate() {
            trace!("{index}: {symbol} {arity}");
        }

        // The initial state has a match goals for each pattern. For each pattern l there is a match goal
        // with one obligation l@ε and announcement l@ε.
        let mut initial_match_goals = Vec::<MatchGoal>::new();
        for rr in &supported_rules {
            initial_match_goals.push(MatchGoal::new(
                MatchAnnouncement {
                    rule: (*rr).clone(),
                    position: DataPosition::empty(),
                    symbols_seen: 0,
                },
                vec![MatchObligation::new(rr.lhs.clone(), DataPosition::empty())],
            ));
        }

        // Match goals need to be sorted so that we can easily check whether a state with a certain
        // set of match goals already exists.
        initial_match_goals.sort();

        // Create the initial state
        let initial_state = State {
            label: DataPosition::empty(),
            match_goals: initial_match_goals.clone(),
        };

        // HashMap from goals to state number
        let mut map_goals_state = HashMap::default();

        // Queue of states that still need to be explored
        let mut queue = VecDeque::new();
        queue.push_back(0);

        map_goals_state.insert(initial_match_goals, 0);

        let mut states = vec![initial_state];
        let mut transitions = FxHashMap::default();

        // Pick a state to explore
        while let Some(s_index) = queue.pop_front() {
            for (symbol, arity) in &symbols {
                let (mut announcements, pos_to_goals) = states
                    .get(s_index)
                    .expect("The state index refers to an existing state")
                    .derive_transition(symbol, *arity, &supported_rules, apma);

                announcements.sort_by(|ma1, ma2| ma1.position.cmp(&ma2.position));

                // For the destinations we convert the match goal destinations to states
                let mut destinations = smallvec![];

                // Loop over the hypertransitions
                for (pos, goals_or_initial) in pos_to_goals {
                    // Match goals need to be sorted so that we can easily check whether a state with a certain
                    // set of match goals already exists.
                    if let GoalsOrInitial::Goals(goals) = goals_or_initial {
                        // This code cannot be replaced by the entry since contains_key takes a reference.
                        #[allow(clippy::map_entry)]
                        if map_goals_state.contains_key(&goals) {
                            // The destination state already exists
                            destinations.push((
                                pos,
                                *map_goals_state
                                    .get(&goals)
                                    .expect("The destination state exists, checked above"),
                            ))
                        } else if !goals.is_empty() {
                            // The destination state does not yet exist, create it
                            let new_state = State::new(goals.clone());
                            states.push(new_state);
                            destinations.push((pos, state_counter));
                            map_goals_state.insert(goals, state_counter);
                            queue.push_back(state_counter);
                            state_counter += 1;
                        }
                    } else {
                        // The transition is to the initial state
                        destinations.push((pos, 0));
                    }
                }

                // A transition that announces nothing and leads nowhere matches no pattern, as a
                // missing one does. Most pairs of a state and a symbol are such a dead end, so
                // storing them would dominate the automaton's size.
                let native = native_ops.get(&symbol.operation_id()).copied();
                if announcements.is_empty() && destinations.is_empty() && native.is_none() {
                    continue;
                }

                // Add the annotation for every match announcement.
                let announcements = announcements
                    .into_iter()
                    .map(|ma| {
                        let annotation = annotate(&ma.rule);
                        (ma, annotation)
                    })
                    .collect();

                // Add the resulting outgoing transition to the state.
                debug_assert!(
                    !&transitions.contains_key(&(s_index, symbol.operation_id())),
                    "Set automaton should not contain duplicated transitions"
                );
                transitions.insert(
                    (s_index, symbol.operation_id()),
                    Transition {
                        symbol: symbol.clone(),
                        announcements,
                        destinations,
                        native,
                    },
                );
            }

            debug!(
                "Queue size {}, currently {} states and {} transitions",
                queue.len(),
                states.len(),
                transitions.len()
            );
        }

        // Clear the match goals since they are only for debugging purposes.
        if !log_enabled!(log::Level::Debug) {
            for state in &mut states {
                state.match_goals.clear();
            }
        }
        info!(
            "Created set automaton (states: {}, transitions: {}, apma: {}) in {} ms",
            states.len(),
            transitions.len(),
            apma,
            (Instant::now() - start).as_millis()
        );

        let result = SetAutomaton { states, transitions };
        debug!("{result:?}");

        result
    }

    /// Returns the number of states
    pub fn num_of_states(&self) -> usize {
        self.states.len()
    }

    /// Returns the number of transitions
    pub fn num_of_transitions(&self) -> usize {
        self.transitions.len()
    }

    /// Returns the states of the automaton
    pub fn states(&self) -> &[State] {
        &self.states
    }

    /// Returns the transition for the given state and function symbol operation
    /// id, if one exists.
    pub fn get_transition(&self, state: usize, operation_id: usize) -> Option<&Transition<M>> {
        self.transitions.get(&(state, operation_id))
    }

    /// Iterates over all transitions as `(state, operation_id, transition)` tuples.
    pub fn iter_transitions(&self) -> impl Iterator<Item = (usize, usize, &Transition<M>)> {
        self.transitions
            .iter()
            .map(|(&(state, operation_id), transition)| (state, operation_id, transition))
    }

    /// Provides a formatter for the .dot file format
    pub fn to_dot_graph(&self, show_backtransitions: bool, show_final: bool) -> DotFormatter<'_, M> {
        DotFormatter {
            automaton: self,
            show_backtransitions,
            show_final,
        }
    }
}

#[derive(Debug)]
pub struct Derivative {
    pub completed: Vec<MatchGoal>,
    pub unchanged: Vec<MatchGoal>,
    pub reduced: Vec<MatchGoal>,
}

/// Classification of a match goal during derivative computation.
#[derive(Debug)]
enum MatchGoalClassification {
    Completed,
    Discarded,
    Unchanged,
    Reducible,
}

pub struct State {
    label: DataPosition,
    match_goals: Vec<MatchGoal>,
}

impl State {
    /// Derives the outgoing transition for `symbol`: the match announcements it completes, and
    /// its destinations as sets of match goals rather than full states, since it is not yet known
    /// whether a destination state already exists.
    fn derive_transition(
        &self,
        symbol: &DataFunctionSymbol,
        arity: usize,
        rewrite_rules: &[Rule],
        apma: bool,
    ) -> (Vec<MatchAnnouncement>, Vec<(DataPosition, GoalsOrInitial)>) {
        // Computes the derivative containing the goals that are completed, unchanged and reduced
        let mut derivative = self.compute_derivative(symbol, arity);

        // The outputs/matching patterns of the transitions are those who are completed
        let outputs = derivative.completed.into_iter().map(|x| x.announcement).collect();

        // The new match goals are the unchanged and reduced match goals.
        let mut new_match_goals = derivative.unchanged;
        new_match_goals.append(&mut derivative.reduced);

        let mut destinations = vec![];
        // If we are building an APMA we do not deepen the position or create a hypertransitions
        // with multiple endpoints
        if apma {
            if !new_match_goals.is_empty() {
                destinations.push((DataPosition::empty(), GoalsOrInitial::Goals(new_match_goals)));
            }
        } else {
            Self::build_set_automaton_destinations(
                &self.label,
                arity,
                rewrite_rules,
                new_match_goals,
                &mut destinations,
            );
        }

        // Sort the destination such that transitions which do not deepen the position are listed first
        destinations.sort_unstable_by(|(pos1, _), (pos2, _)| pos1.cmp(pos2));
        (outputs, destinations)
    }

    /// Partitions match goals, computes greatest-common-prefix destinations, and
    /// adds fresh match goals for each argument position not covered by an
    /// existing partition.
    fn build_set_automaton_destinations(
        label: &DataPosition,
        arity: usize,
        rewrite_rules: &[Rule],
        new_match_goals: Vec<MatchGoal>,
        destinations: &mut Vec<(DataPosition, GoalsOrInitial)>,
    ) {
        let partitioned = MatchGoal::partition(new_match_goals);

        // Get the greatest common prefix and shorten the positions
        let mut obligations_per_partition = vec![];
        let mut gcp_length_per_partition = vec![];
        for p in partitioned {
            let mut obligation_positions = vec![];
            for goal in &p {
                for obligation in &goal.obligations {
                    obligation_positions.push(obligation.position.clone());
                }
            }
            obligations_per_partition.push(obligation_positions);

            let gcp = MatchGoal::greatest_common_prefix(&p);
            let gcp_length = gcp.len();
            gcp_length_per_partition.push(gcp_length);
            let mut goals = MatchGoal::remove_prefix(p, gcp_length);
            goals.sort_unstable();
            destinations.push((gcp, GoalsOrInitial::Goals(goals)));
        }

        Self::add_fresh_match_goals(
            label,
            arity,
            rewrite_rules,
            &obligations_per_partition,
            &gcp_length_per_partition,
            destinations,
        );
    }

    /// Adds fresh match goals for argument positions `1..=arity` that are not
    /// covered by an existing partition.
    fn add_fresh_match_goals(
        label: &DataPosition,
        arity: usize,
        rewrite_rules: &[Rule],
        obligations_per_partition: &[Vec<DataPosition>],
        gcp_length_per_partition: &[usize],
        destinations: &mut Vec<(DataPosition, GoalsOrInitial)>,
    ) {
        // Handle fresh match goals, they are the positions Label(state).i
        // where i is between 1 and the arity of the function symbol of
        // the transition. Position 1 is the first argument.
        for i in 1..arity + 1 {
            let mut pos = label.clone();
            pos.push(i);

            // Obligation positions, not announcement positions: the latter are
            // empty for root-anchored goals and an empty position is a prefix
            // of every position, so every fresh subtree would be merged and
            // construction would not terminate in practice.
            //
            // Every obligation lies at or below its announcement position, so
            // this only ever splits off fresh positions that the announcement
            // test would have merged. Root-anchored goals still determine the
            // state label, and the partition's prefix sorts before the split-off
            // position, so root matches are still attempted first. What changes
            // is the order in which sibling subterms are explored once the root
            // goals fail.
            let partition_key = obligations_per_partition
                .iter()
                .enumerate()
                .find(|(_, obligation_positions)| {
                    obligation_positions
                        .iter()
                        .any(|op| MatchGoal::pos_comparable(&pos, op))
                })
                .map(|(k, _)| k);

            if let Some(key) = partition_key {
                // If the fresh goals fall in an existing partition
                let gcp_length = gcp_length_per_partition[key];
                debug_assert!(
                    gcp_length <= pos.len(),
                    "greatest common prefix cannot be deeper than the fresh position"
                );
                let pos = DataPosition::new(&pos.indices()[gcp_length..]);

                // Add the fresh goals to the partition
                if let GoalsOrInitial::Goals(goals) = &mut destinations[key].1 {
                    for rr in rewrite_rules {
                        goals.push(MatchGoal {
                            obligations: vec![MatchObligation::new(rr.lhs.clone(), pos.clone())],
                            announcement: MatchAnnouncement {
                                rule: (*rr).clone(),
                                position: pos.clone(),
                                symbols_seen: 0,
                            },
                        });
                    }
                }
            } else {
                // The transition is simply to the initial state
                // GoalsOrInitial::InitialState avoids unnecessary work of creating all these fresh goals
                destinations.push((pos, GoalsOrInitial::InitialState));
            }
        }
    }

    /// Classifies a match goal against the current symbol and label, returning
    /// which bucket it belongs to.
    fn classify_match_goal(
        mg: &MatchGoal,
        symbol: &DataFunctionSymbol,
        label: &DataPosition,
    ) -> MatchGoalClassification {
        debug_assert!(
            !mg.obligations.is_empty(),
            "The obligations should never be empty, should be completed then"
        );

        // Completed match goals
        if mg.obligations.len() == 1
            && mg.obligations.iter().any(|mo| {
                mo.position == *label
                    && mo.pattern.data_function_symbol() == symbol.copy()
                    && mo.pattern.data_arguments().all(|x| is_data_variable(&x))
            })
        {
            return MatchGoalClassification::Completed;
        }

        // Discarded: head symbol does not match
        if mg
            .obligations
            .iter()
            .any(|mo| mo.position == *label && mo.pattern.data_function_symbol() != symbol.copy())
        {
            return MatchGoalClassification::Discarded;
        }

        // Unchanged match goals
        if mg.obligations.iter().all(|mo| mo.position != *label) {
            return MatchGoalClassification::Unchanged;
        }

        MatchGoalClassification::Reducible
    }

    /// For a transition 'symbol' of state 'self' this function computes which match goals are
    /// completed, unchanged and reduced.
    fn compute_derivative(&self, symbol: &DataFunctionSymbol, arity: usize) -> Derivative {
        let mut result = Derivative {
            completed: vec![],
            unchanged: vec![],
            reduced: vec![],
        };

        for mg in &self.match_goals {
            match Self::classify_match_goal(mg, symbol, &self.label) {
                MatchGoalClassification::Completed => {
                    result.completed.push(mg.clone());
                }
                MatchGoalClassification::Discarded => {
                    // Match goal is discarded since head symbol does not match.
                }
                MatchGoalClassification::Unchanged => {
                    let mut mg = mg.clone();
                    if mg.announcement.rule.lhs
                        != mg
                            .obligations
                            .first()
                            .expect("A match goal has at least one obligation")
                            .pattern
                    {
                        mg.announcement.symbols_seen += 1;
                    }
                    result.unchanged.push(mg.clone());
                }
                MatchGoalClassification::Reducible => {
                    let mut mg = mg.clone();
                    let mut new_obligations = vec![];

                    for mo in mg.obligations {
                        if mo.pattern.data_function_symbol() == symbol.copy() && mo.position == self.label {
                            // Reduced match obligation
                            for (index, t) in mo.pattern.data_arguments().enumerate() {
                                assert!(
                                    index < arity,
                                    "This pattern associates function symbol {:?} with different arities {} and {}",
                                    symbol,
                                    index + 1,
                                    arity
                                );

                                if !is_data_variable(&t) {
                                    let mut new_pos = mo.position.clone();
                                    new_pos.push(index + 1);
                                    new_obligations.push(MatchObligation {
                                        pattern: t.protect(),
                                        position: new_pos,
                                    });
                                }
                            }
                        } else {
                            // remains unchanged
                            new_obligations.push(mo.clone());
                        }
                    }

                    new_obligations.sort_unstable_by_key(|mo1| mo1.position.len());
                    mg.obligations = new_obligations;
                    mg.announcement.symbols_seen += 1;

                    result.reduced.push(mg);
                }
            }
        }

        trace!(
            "=== compute_derivative(symbol = {}, label = {}) ===",
            symbol, self.label
        );
        trace!("Match goals: {{");
        for mg in &self.match_goals {
            trace!("\t {mg:?}");
        }

        trace!("}}");
        trace!("Completed: {{");
        for mg in &result.completed {
            trace!("\t {mg:?}");
        }

        trace!("}}");
        trace!("Unchanged: {{");
        for mg in &result.unchanged {
            trace!("\t {mg:?}");
        }

        trace!("}}");
        trace!("Reduced: {{");
        for mg in &result.reduced {
            trace!("\t {mg:?}");
        }
        trace!("}}");

        result
    }

    /// Create a state from a set of match goals
    fn new(goals: Vec<MatchGoal>) -> State {
        // The label of the state is taken from a match obligation of a root match goal.
        let mut label: Option<DataPosition> = None;

        // Go through all match goals until a root match goal is found
        for goal in &goals {
            if goal.announcement.position.is_empty() {
                // Find the shortest match obligation position.
                // This design decision was taken as it presumably has two advantages.
                // 1. Patterns that overlap will be more quickly distinguished, potentially decreasing
                // the size of the automaton.
                // 2. The average lookahead may be shorter.
                if label.is_none() {
                    label = Some(
                        goal.obligations
                            .first()
                            .expect("A match goal has at least one obligation")
                            .position
                            .clone(),
                    );
                }

                for obligation in &goal.obligations {
                    if let Some(l) = &label
                        && &obligation.position < l
                    {
                        label = Some(obligation.position.clone());
                    }
                }
            }
        }

        State {
            label: label.expect("At least one match goal determines the label"),
            match_goals: goals,
        }
    }

    /// Returns the label of the state
    pub fn label(&self) -> &DataPosition {
        &self.label
    }

    /// Returns the match goals of the state
    pub fn match_goals(&self) -> &[MatchGoal] {
        &self.match_goals
    }
}

/// The stand-in function symbol under which every machine number is matched.
///
/// The automaton keys its transitions by function symbol, but a machine number
/// is a value that carries none. Without a symbol to branch on, a position
/// holding a machine number could not be distinguished from one holding a
/// function symbol a pattern expects there, and matching would have to give up
/// at that position — losing every rule that has a variable there, such as
/// `@most_significant_digitNat(w1) + @most_significant_digitNat(w2)`.
///
/// All machine numbers share this one symbol, so the automaton only
/// discriminates *that* a position holds a machine number, not *which* one. The
/// bundled specifications never put a machine-number literal in a left-hand
/// side (they use `@zero_word` and friends, which are ordinary function
/// symbols), so nothing currently relies on telling two values apart here.
pub(crate) fn machine_number_symbol() -> DataFunctionSymbol {
    DataFunctionSymbol::new("@machine_number@")
}

/// Adds the given function symbol to the indexed symbols. Errors when a
/// function symbol is overloaded with different arities.
fn add_symbol(function_symbol: DataFunctionSymbol, arity: usize, symbols: &mut HashMap<DataFunctionSymbol, usize>) {
    if let Some(x) = symbols.get(&function_symbol) {
        assert_eq!(
            *x, arity,
            "Function symbol {function_symbol} occurs with different arities",
        );
    } else {
        symbols.insert(function_symbol, arity);
    }
}

/// Returns false iff this is a higher order term, of the shape t(t_0, ..., t_n), contains a
/// where clause, a binder (unless `binders` allows an `exists`/`forall`), or is an unknown term.
fn is_supported_term(t: &DataExpression, binders: bool) -> bool {
    for subterm in t.iter() {
        if is_data_application(&subterm) && !is_data_function_symbol(&subterm.arg(0)) {
            warn!("{} is higher order", subterm);
            return false;
        }

        // A lambda or comprehension could only be rebuilt, since their reduction is not
        // implemented, so only the binders that the rewriter can decide are kept.
        if is_data_binder(&subterm)
            && !(binders
                && matches!(
                    DataAbstractionRef::from(subterm.copy()).binder_type(),
                    BinderType::Exists | BinderType::Forall
                ))
        {
            warn!("{} contains an unsupported binder", subterm);
            return false;
        }

        if is_data_where_clause(&subterm) {
            warn!("{} contains a where clause", subterm);
            return false;
        }
    }

    true
}

/// Checks whether the set automaton can use this rule: no higher order rules or binders, and
/// every variable of its condition or right-hand side occurs in its left-hand side.
pub fn is_supported_rule(rule: &Rule) -> bool {
    is_supported_rule_with(rule, false)
}

/// Like [is_supported_rule], but `binders` allows `exists`/`forall` binders in the right-hand
/// side; see [SetAutomaton::with_binders].
fn is_supported_rule_with(rule: &Rule, binders: bool) -> bool {
    // There should be no terms of the shape t(t0,...,t_n)
    if !is_supported_term(&rule.rhs, binders) || !is_supported_term(&rule.lhs, false) {
        return false;
    }

    for cond in &rule.conditions {
        if !is_supported_term(&cond.rhs, false) || !is_supported_term(&cond.lhs, false) {
            return false;
        }
    }

    free_variables_occur_in_rhs(rule)
}

/// Returns false iff `expr` has a free variable outside `lhs_variables`, warning about `rule`
/// when it does.
fn free_variables_occur_in(expr: &DataExpression, lhs_variables: &HashSet<DataVariable>, rule: &Rule) -> bool {
    for variable in free_variables(expr) {
        if !lhs_variables.contains(&variable) {
            warn!(
                "the equation '{rule}' uses the variable '{variable}' in its condition or right-hand side, \
                 but it does not occur in the left-hand side; dropping the equation"
            );
            return false;
        }
    }
    true
}

/// Returns true iff every free variable of the rule's right-hand side and conditions also occurs
/// in its left-hand side, since a match only assigns values to the left-hand side's variables.
/// Variables bound by a binder or where clause get their value when that is evaluated instead.
fn free_variables_occur_in_rhs(rule: &Rule) -> bool {
    let lhs_variables: HashSet<DataVariable> = free_variables(&rule.lhs).collect();

    if !free_variables_occur_in(&rule.rhs, &lhs_variables, rule) {
        return false;
    }

    for cond in &rule.conditions {
        if !free_variables_occur_in(&cond.lhs, &lhs_variables, rule)
            || !free_variables_occur_in(&cond.rhs, &lhs_variables, rule)
        {
            return false;
        }
    }
    true
}

/// Finds all data symbols in the term and adds them to the symbol index.
fn find_symbols(t: &DataExpressionRef<'_>, symbols: &mut HashMap<DataFunctionSymbol, usize>) {
    /// Collects the head symbol of every node, with the arity it occurs with.
    struct FindSymbols<'a> {
        symbols: &'a mut HashMap<DataFunctionSymbol, usize>,
    }

    impl<'a> DataExpressionVisitor<'a> for FindSymbols<'_> {
        type Context = ();
        type Break = Infallible;
        type Error = Infallible;

        fn visit_function_symbol(
            &mut self,
            function_symbol: &DataFunctionSymbolRef<'a>,
            context: (),
        ) -> Visit<Infallible, (), Infallible, Infallible> {
            add_symbol(function_symbol.protect(), 0, self.symbols);
            Ok(ControlFlow::Continue(Step::Into(context)))
        }

        fn visit_application(
            &mut self,
            application: &DataApplicationRef<'a>,
            context: (),
        ) -> Visit<Infallible, (), Infallible, Infallible> {
            // REC specifications should never contain this so it can be a debug error.
            assert!(
                is_data_function_symbol(&application.data_function_symbol()),
                "Error in term {application}, higher order term rewrite systems are not supported"
            );

            add_symbol(
                application.data_function_symbol().protect(),
                application.data_arguments().len(),
                self.symbols,
            );
            Ok(ControlFlow::Continue(Step::Into(context)))
        }

        fn visit_machine_number(
            &mut self,
            _number: &MachineNumberRef<'a>,
            context: (),
        ) -> Visit<Infallible, (), Infallible, Infallible> {
            // A machine number has no function symbol of its own, so it is
            // represented by the shared stand-in symbol; see [machine_number_symbol].
            add_symbol(machine_number_symbol(), 0, self.symbols);
            Ok(ControlFlow::Continue(Step::Into(context)))
        }

        // Variables carry no symbol.
    }

    FindSymbols { symbols }.visit(t, ());
}
