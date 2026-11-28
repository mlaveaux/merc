#![forbid(unsafe_code)]

use std::fmt;
use std::hash::Hash;

use delegate::delegate;
use itertools::Itertools;

use merc_aterm::ATerm;
use merc_aterm::ATermArgs;
use merc_aterm::ATermIndex;
use merc_aterm::ATermList;
use merc_aterm::ATermRef;
use merc_aterm::ATermString;
use merc_aterm::Markable;
use merc_aterm::Symb;
use merc_aterm::SymbolRef;
use merc_aterm::Term;
use merc_aterm::TermIterator;
use merc_aterm::Transmutable;
use merc_aterm::storage::Marker;
use merc_collections::VecBag;
use merc_data::DataExpression;
use merc_data::is_undefined_real;
use merc_data::undefined_real;
use merc_macros::merc_derive_terms;
use merc_macros::merc_term;
use merc_utilities::MercError;

use crate::TransitionLabel;

/// Represents a multi-action, i.e., a multi set of action labels
#[derive(Clone, PartialOrd, Ord, PartialEq, Eq, Hash)]
pub struct LtsMultiAction<L: Ord> {
    actions: VecBag<L>,
}

impl<L: Ord> LtsMultiAction<L> {
    /// Creates a new multi-action with the given set of action labels.
    pub fn new(actions: VecBag<L>) -> Self {
        LtsMultiAction { actions }
    }

    /// Returns true iff the multi-action is a tau action, i.e., it contains no action labels.
    pub fn is_tau_label(&self) -> bool {
        self.actions.is_empty()
    }

    /// Returns the set of action labels contained in the multi-action.
    pub fn actions(&self) -> &VecBag<L> {
        &self.actions
    }

    /// Consumes the multi-action and returns the set of action labels contained in it.
    pub fn into_actions(self) -> VecBag<L> {
        self.actions
    }

    /// Retains only the actions in the multi-action that satisfy the given predicate.
    pub fn retain<F>(&mut self, mut f: F)
    where
        F: FnMut(&L) -> bool,
    {
        self.actions.retain(|action| f(action));
    }
}

/// A single action label with a name and string-valued arguments, used for the
/// mCRL2 textual `.aut` format where argument values are not yet interpreted as
/// data expressions.
#[derive(Clone, PartialOrd, Ord, PartialEq, Eq, Hash, Debug)]
pub struct SimpleAction {
    label: String,
    arguments: Vec<String>,
}

impl SimpleAction {
    /// Creates a new simple action with the given label and arguments.
    pub fn new(label: String, arguments: Vec<String>) -> Self {
        SimpleAction { label, arguments }
    }

    /// Returns the name of the action label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the string arguments of the action.
    pub fn arguments(&self) -> &[String] {
        &self.arguments
    }
}

impl fmt::Display for SimpleAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.arguments.is_empty() {
            write!(f, "{}", self.label)
        } else {
            write!(f, "{}({})", self.label, self.arguments.iter().format(", "))
        }
    }
}

impl TransitionLabel for SimpleAction {
    fn is_tau_label(&self) -> bool {
        false
    }

    fn tau_label() -> Self {
        panic!("SimpleAction does not support tau labels; use LtsMultiAction for that")
    }

    fn matches_label(&self, label: &str) -> bool {
        self.label == label
    }

    fn from_index(i: usize) -> Self {
        SimpleAction::new(
            char::from_digit(i as u32, 36)
                .expect("Radix is less than 37, so should not panic")
                .to_string(),
            Vec::new(),
        )
    }
}

impl LtsMultiAction<SimpleAction> {
    /// Parses a multi-action from the mCRL2 textual `.aut` label format.
    ///
    /// Handles labels of the form `"a(1, 2) | b | c(x)"`: each `|`-separated
    /// part is split into a label name and optional parenthesised argument list.
    ///
    /// # Errors
    ///
    /// Returns an error if a `|`-separated part is empty or has unbalanced parentheses.
    pub fn from_string(input: &str) -> Result<Self, MercError> {
        let mut actions = VecBag::new();

        for part in input.split('|') {
            let part = part.trim();
            if part.is_empty() {
                return Err("Empty action label in multi-action.".into());
            }

            if let Some(open) = part.find('(') {
                if !part.ends_with(')') {
                    return Err(format!("Malformed action with arguments: {part}").into());
                }
                let label = part[..open].trim().to_string();
                let args_str = &part[open + 1..part.len() - 1];
                let arguments = args_str.split(',').map(|s| s.trim().to_string()).collect();
                actions.insert(SimpleAction::new(label, arguments));
            } else {
                actions.insert(SimpleAction::new(part.to_string(), Vec::new()));
            }
        }

        Ok(LtsMultiAction { actions })
    }
}

impl LtsMultiAction<LtsAction> {
    /// Converts the MultiAction into its mCRL2 ATerm representation.
    pub fn to_mcrl2_aterm(&self) -> Result<ATerm, MercError> {
        let action_terms: Vec<MCRL2Action> = self
            .actions
            .iter()
            .map(|action| -> Result<MCRL2Action, MercError> {
                let label_term = MCRL2ActionLabel::new(
                    ATermString::new(&action.label).copy(),
                    ATermList::<DataExpression>::empty(),
                );

                let arguments_term = ATermList::<DataExpression>::from_double_iter(action.arguments.iter().cloned());

                Ok(MCRL2Action::new(label_term.copy(), arguments_term))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let actions_list = ATermList::<MCRL2Action>::from_double_iter(action_terms.into_iter());
        let time_term: DataExpression = undefined_real();
        Ok(MCRL2TimedMultiAction::new(actions_list, time_term.copy()).into())
    }

    /// Constructs a MultiAction from an mCRL2 ATerm representation.
    ///
    /// # Errors
    ///
    /// Returns an error if `term` is not a `TimedMultAct` term, or if it carries an
    /// actual (non-`@undefined_real`) time value, since timed multi-actions are not supported.
    pub fn from_mcrl2_aterm(term: ATerm) -> Result<Self, MercError> {
        if is_mcrl2_timed_multi_action_symbol(&term.get_head_symbol()) {
            let multi_action = MCRL2TimedMultiAction::from(term);

            if !is_undefined_real(&multi_action.time()) {
                return Err("Timed multi-actions are not supported.".into());
            }

            let mut actions = VecBag::new();
            for action in multi_action.actions() {
                let arguments = action.arguments().to_vec();

                // Convert the label to string
                actions.insert(LtsAction {
                    label: action.label().name().to_string(),
                    arguments,
                });
            }

            Ok(LtsMultiAction { actions })
        } else {
            Err(format!("Expected TimedMultAction symbol, got {}.", term).into())
        }
    }
}

#[merc_derive_terms]
mod inner {
    use merc_aterm::ATermStringRef;
    use merc_aterm::Symbol;
    use merc_data::DataExpression;
    use merc_data::DataExpressionRef;
    use merc_macros::merc_ignore;

    use super::*;

    /// Represents a TimedMultiAction in mCRL2, which is a multi-action with an associated time.
    #[merc_term(is_mcrl2_timed_multi_action)]
    pub struct MCRL2TimedMultiAction {
        term: ATerm,
    }

    impl MCRL2TimedMultiAction {
        /// Creates a new TimedMultiAction with the given actions and time.
        #[merc_ignore]
        pub fn new(actions: ATermList<MCRL2Action>, time: DataExpressionRef<'_>) -> Self {
            let args: &[ATermRef<'_>] = &[actions.copy(), time.into()];
            let term = ATerm::with_args(&Symbol::new("TimedMultAct", 2), args);
            MCRL2TimedMultiAction { term: term.protect() }
        }

        /// Returns the actions contained in the multi-action.
        pub fn actions(&self) -> ATermList<MCRL2Action> {
            self.term.arg(0).into()
        }

        /// Returns the time at which the multi-action occurs.
        pub fn time(&self) -> DataExpressionRef<'_> {
            self.term.arg(1).into()
        }
    }

    #[merc_term(is_mcrl2_action)]
    pub struct MCRL2Action {
        term: ATerm,
    }

    impl MCRL2Action {
        /// Creates a new Action with the given label and arguments.
        #[merc_ignore]
        pub fn new(label: MCRL2ActionLabelRef<'_>, arguments: ATermList<DataExpression>) -> Self {
            let args: &[ATermRef<'_>] = &[label.into(), arguments.copy()];
            let term = ATerm::with_args(&Symbol::new("Action", 2), args);
            MCRL2Action { term: term.protect() }
        }

        /// Returns the label of the action.
        pub fn label(&self) -> MCRL2ActionLabelRef<'_> {
            self.term.arg(0).into()
        }

        /// Returns the data arguments of the action.
        pub fn arguments(&self) -> ATermList<DataExpression> {
            self.term.arg(1).into()
        }
    }

    #[merc_term(is_mcrl2_action_label)]
    pub struct MCRL2ActionLabel {
        term: ATerm,
    }

    impl MCRL2ActionLabel {
        /// Constructs a new action label with the given name and arguments.
        #[merc_ignore]
        pub fn new(name: ATermStringRef<'_>, args: ATermList<DataExpression>) -> Self {
            let args: &[ATermRef<'_>] = &[name.into(), args.copy()];
            let term = ATerm::with_args(&Symbol::new("ActId", 2), args);
            MCRL2ActionLabel { term: term.protect() }
        }

        /// Obtain the name of the action label.
        pub fn name(&self) -> ATermStringRef<'_> {
            self.term.arg(0).into()
        }
    }
}

pub use inner::*;

/// Checks if the given term's head symbol represents a TimedMultiAction in mCRL2.
fn is_mcrl2_timed_multi_action<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    is_mcrl2_timed_multi_action_symbol(&term.get_head_symbol())
}

/// Checks if the given term's head symbol represents an Action in mCRL2.
fn is_mcrl2_action<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    is_mcrl2_action_symbol(&term.get_head_symbol())
}

/// Checks if the given term's head symbol represents an ActionLabel in mCRL2.
fn is_mcrl2_action_label<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> bool {
    is_mcrl2_action_label_symbol(&term.get_head_symbol())
}

/// Checks if the given symbol represents a TimedMultiAction in mCRL2.
fn is_mcrl2_timed_multi_action_symbol(symbol: &SymbolRef<'_>) -> bool {
    symbol.name() == "TimedMultAct" && symbol.arity() == 2
}

/// Checks if the given symbol represents an Action in mCRL2.
fn is_mcrl2_action_symbol(symbol: &SymbolRef<'_>) -> bool {
    symbol.name() == "Action" && symbol.arity() == 2
}

/// Checks if the given symbol represents an ActionLabel in mCRL2.
fn is_mcrl2_action_label_symbol(symbol: &SymbolRef<'_>) -> bool {
    symbol.name() == "ActId" && symbol.arity() == 2
}

/// Represents a single action label, with its (data) arguments
#[derive(Clone, Debug, PartialOrd, Ord, PartialEq, Eq, Hash)]
pub struct LtsAction {
    label: String,
    arguments: Vec<DataExpression>,
}

impl LtsAction {
    /// Creates a new action label with the given name and arguments.
    pub fn new(label: String, arguments: Vec<DataExpression>) -> Self {
        LtsAction { label, arguments }
    }

    /// Returns the name of the action label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the data arguments of the action.
    pub fn arguments(&self) -> &[DataExpression] {
        &self.arguments
    }
}

impl TransitionLabel for LtsAction {
    fn is_tau_label(&self) -> bool {
        false
    }

    fn tau_label() -> Self {
        panic!("LtsAction does not support tau labels. Use LtsMultiAction for that.")
    }

    fn matches_label(&self, label: &str) -> bool {
        self.label == label
    }

    fn from_index(i: usize) -> Self {
        LtsAction::new(
            char::from_digit(i as u32, 36)
                .expect("Radix is less than 37, so should not panic")
                .to_string(),
            Vec::new(),
        )
    }
}

impl<L: TransitionLabel> TransitionLabel for LtsMultiAction<L> {
    fn is_tau_label(&self) -> bool {
        self.actions.is_empty()
    }

    fn tau_label() -> Self {
        LtsMultiAction { actions: VecBag::new() }
    }

    fn matches_label(&self, label: &str) -> bool {
        // TODO: Is this correct, now a|b matches a?
        self.actions.iter().any(|action| action.matches_label(label))
    }

    fn from_index(i: usize) -> Self {
        // For now we only generate single actions, but these could become multiactions as well
        LtsMultiAction {
            actions: VecBag::singleton(L::from_index(i)),
        }
    }
}

impl<L: Ord + fmt::Display> fmt::Display for LtsMultiAction<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.actions.is_empty() {
            write!(f, "τ")
        } else {
            write!(f, "{}", self.actions.iter().format("|"))
        }
    }
}

impl fmt::Display for LtsAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.arguments.is_empty() {
            write!(f, "{}", self.label)
        } else {
            write!(f, "{}({})", self.label, self.arguments.iter().format(", "))
        }
    }
}

impl<L: Ord + fmt::Display> fmt::Debug for LtsMultiAction<L> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Use the debug format to print the display format
        write!(f, "{}", self)
    }
}

#[cfg(test)]
mod tests {
    use merc_collections::VecBag;
    use merc_data::is_undefined_real;

    use super::LtsAction;
    use super::LtsMultiAction;

    #[test]
    fn to_aterm_uses_the_mcrl2_undefined_real() {
        let multi_action = LtsMultiAction::new(VecBag::singleton(LtsAction::new("a".to_string(), vec![])));
        let term = multi_action.to_mcrl2_aterm().unwrap();

        // The default time term must be exactly mCRL2's undefined real: we parse it back and
        // check the time slot of the resulting TimedMultAct is that variable.
        let timed: super::MCRL2TimedMultiAction = term.clone().into();
        assert!(is_undefined_real(&timed.time()));
    }
}
