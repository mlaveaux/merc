use std::collections::VecDeque;
use std::fmt::Debug;
use std::time::Instant;

use ahash::HashMap;
use log::debug;
use log::info;
use log::log_enabled;
use log::trace;
use log::warn;
use merc_aterm::Term;
use merc_data::DataExpression;
use merc_data::DataExpressionRef;
use merc_data::DataFunctionSymbol;
use merc_data::is_data_application;
use merc_data::is_data_function_symbol;
use merc_data::is_data_machine_number;
use merc_data::is_data_variable;
use smallvec::SmallVec;
use smallvec::smallvec;

use crate::rewrite_specification::RewriteSpecification;
use crate::rewrite_specification::Rule;
use crate::utilities::DataPosition;

use super::DotFormatter;
use super::MatchGoal;

/// The Set Automaton used to find all matching patterns in a term.
pub struct SetAutomaton<T> {
    states: Vec<State>,
    transitions: HashMap<(usize, usize), Transition<T>>,
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
}

/// Represents a match obligation in the [SetAutomaton].
#[derive(Clone, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MatchObligation {
    pub pattern: DataExpression,
    pub position: DataPosition,
}

impl MatchObligation {
    /// Returns the pattern of the match obligation
    pub fn new(pattern: DataExpression, position: DataPosition) -> Self {
        MatchObligation { pattern, position }
    }
}

/// Represents either the initial state or a set of match goals in the
/// [SetAutomaton].
///
/// This is only used during construction to avoid craating all the goals for
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
    /// announcement. This is used to accomondate different types of annotations
    /// for the different rewrite engines.
    pub fn new(spec: &RewriteSpecification, annotate: impl Fn(&Rule) -> M, apma: bool) -> SetAutomaton<M> {
        let start = Instant::now();

        // States are labelled s0, s1, s2, etcetera. state_counter keeps track of count.
        let mut state_counter: usize = 1;

        // Remove rules that we cannot deal with
        let supported_rules: Vec<Rule> = spec
            .rewrite_rules()
            .iter()
            .filter(|rule| is_supported_rule(rule))
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

            symbols
        };

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
        let mut transitions = HashMap::default();

        // Pick a state to explore
        while let Some(s_index) = queue.pop_front() {
            for (symbol, arity) in &symbols {
                let (mut announcements, pos_to_goals) =
                    states
                        .get(s_index)
                        .unwrap()
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
                            destinations.push((pos, *map_goals_state.get(&goals).unwrap()))
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
    pub fn states(&self) -> &Vec<State> {
        &self.states
    }

    /// Returns the transitions of the automaton
    pub fn transitions(&self) -> &HashMap<(usize, usize), Transition<M>> {
        &self.transitions
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

pub struct State {
    label: DataPosition,
    match_goals: Vec<MatchGoal>,
}

impl State {
    /// Derive transitions from a state given a head symbol. The resulting transition is returned as a tuple
    /// The tuple consists of a vector of outputs and a set of destinations (which are sets of match goals).
    /// We don't use the struct Transition as it requires that the destination is a full state, with name.
    /// Since we don't yet know whether the state already exists we just return a set of match goals as 'state'.
    ///
    /// Parameter symbol is the symbol for which the transition is computed
    fn derive_transition(
        &self,
        symbol: &DataFunctionSymbol,
        arity: usize,
        rewrite_rules: &Vec<Rule>,
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
            // In case we are building a set automaton we partition the match goals
            let partitioned = MatchGoal::partition(new_match_goals);

            // Get the greatest common prefix and shorten the positions
            let mut positions_per_partition = vec![];
            let mut gcp_length_per_partition = vec![];
            for (p, pos) in partitioned {
                positions_per_partition.push(pos);
                let gcp = MatchGoal::greatest_common_prefix(&p);
                let gcp_length = gcp.len();
                gcp_length_per_partition.push(gcp_length);
                let mut goals = MatchGoal::remove_prefix(p, gcp_length);
                goals.sort_unstable();
                destinations.push((gcp, GoalsOrInitial::Goals(goals)));
            }

            // Handle fresh match goals, they are the positions Label(state).i
            // where i is between 1 and the arity of the function symbol of
            // the transition. Position 1 is the first argument.
            for i in 1..arity + 1 {
                let mut pos = self.label.clone();
                pos.push(i);

                // Check if the fresh goals are related to one of the existing partitions
                let mut partition_key = None;
                'outer: for (i, part_pos) in positions_per_partition.iter().enumerate() {
                    for p in part_pos {
                        if MatchGoal::pos_comparable(p, &pos) {
                            partition_key = Some(i);
                            break 'outer;
                        }
                    }
                }

                if let Some(key) = partition_key {
                    // If the fresh goals fall in an existing partition
                    let gcp_length = gcp_length_per_partition[key];
                    let pos = DataPosition::new(&pos.indices()[gcp_length..]);

                    // Add the fresh goals to the partition
                    for rr in rewrite_rules {
                        if let GoalsOrInitial::Goals(goals) = &mut destinations[key].1 {
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

        // Sort the destination such that transitions which do not deepen the position are listed first
        destinations.sort_unstable_by(|(pos1, _), (pos2, _)| pos1.cmp(pos2));
        (outputs, destinations)
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
            debug_assert!(
                !mg.obligations.is_empty(),
                "The obligations should never be empty, should be completed then"
            );

            // Completed match goals
            if mg.obligations.len() == 1
                && mg.obligations.iter().any(|mo| {
                    mo.position == self.label
                        && mo.pattern.data_function_symbol() == symbol.copy()
                        && mo.pattern.data_arguments().all(|x| is_data_variable(&x))
                    // Again skip the function symbol
                })
            {
                result.completed.push(mg.clone());
            } else if mg
                .obligations
                .iter()
                .any(|mo| mo.position == self.label && mo.pattern.data_function_symbol() != symbol.copy())
            {
                // Match goal is discarded since head symbol does not match.
            } else if mg.obligations.iter().all(|mo| mo.position != self.label) {
                // Unchanged match goals
                let mut mg = mg.clone();
                if mg.announcement.rule.lhs != mg.obligations.first().unwrap().pattern {
                    mg.announcement.symbols_seen += 1;
                }

                result.unchanged.push(mg.clone());
            } else {
                // Reduce match obligations
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

                new_obligations.sort_unstable_by(|mo1, mo2| mo1.position.len().cmp(&mo2.position.len()));
                mg.obligations = new_obligations;
                mg.announcement.symbols_seen += 1;

                result.reduced.push(mg);
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
                    label = Some(goal.obligations.first().unwrap().position.clone());
                }

                for obligation in &goal.obligations {
                    if let Some(l) = &label {
                        if &obligation.position < l {
                            label = Some(obligation.position.clone());
                        }
                    }
                }
            }
        }

        State {
            label: label.unwrap(),
            match_goals: goals,
        }
    }

    /// Returns the label of the state
    pub fn label(&self) -> &DataPosition {
        &self.label
    }

    /// Returns the match goals of the state
    pub fn match_goals(&self) -> &Vec<MatchGoal> {
        &self.match_goals
    }
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

/// Returns false iff this is a higher order term, of the shape t(t_0, ..., t_n), or an unknown term.
fn is_supported_term(t: &DataExpression) -> bool {
    for subterm in t.iter() {
        if is_data_application(&subterm) && !is_data_function_symbol(&subterm.arg(0)) {
            warn!("{} is higher order", &subterm);
            return false;
        }
    }

    true
}

/// Checks whether the set automaton can use this rule, no higher order rules or binders.
pub fn is_supported_rule(rule: &Rule) -> bool {
    // There should be no terms of the shape t(t0,...,t_n)
    if !is_supported_term(&rule.rhs) || !is_supported_term(&rule.lhs) {
        return false;
    }

    for cond in &rule.conditions {
        if !is_supported_term(&cond.rhs) || !is_supported_term(&cond.lhs) {
            return false;
        }
    }

    true
}

/// Finds all data symbols in the term and adds them to the symbol index.
fn find_symbols(t: &DataExpressionRef<'_>, symbols: &mut HashMap<DataFunctionSymbol, usize>) {
    if is_data_function_symbol(t) {
        add_symbol(t.protect().into(), 0, symbols);
    } else if is_data_application(t) {
        // REC specifications should never contain this so it can be a debug error.
        assert!(
            is_data_function_symbol(&t.data_function_symbol()),
            "Error in term {t}, higher order term rewrite systems are not supported"
        );

        add_symbol(t.data_function_symbol().protect(), t.data_arguments().len(), symbols);
        for arg in t.data_arguments() {
            find_symbols(&arg, symbols);
        }
    } else if is_data_machine_number(t) {
        // Ignore machine numbers during matching?
    } else if !is_data_variable(t) {
        panic!("Unexpected term {t:?}");
    }
}
