use std::fmt;
use std::hash::Hash;

use itertools::Itertools;
use merc_aterm::ATerm;
use merc_utilities::{MercError, VecSet};

use crate::TransitionLabel;

/// Represents a multi-action, i.e., a set of action labels
#[derive(Clone, PartialOrd, Ord, PartialEq, Eq, Hash)]
pub struct MultiAction {
    actions: VecSet<Action>,
}

impl MultiAction {
    /// Parses a multi-action from a string representation, typically found in the Aldebaran format.
    pub fn from_string(input: &str) -> Result<Self, MercError> {
        let mut actions = VecSet::new();

        for part in input.split('|') {
            let part = part.trim();
            if part.is_empty() {
                return Err("Empty action label in multi-action.".into());
            }

            if let Some(open_paren_index) = part.find('(') {
                if !part.ends_with(')') {
                    return Err(format!("Malformed action with arguments: {}", part).into());
                }

                let label = &part[..open_paren_index].trim();
                let args_str = &part[open_paren_index + 1..part.len() - 1];
                let arguments: Vec<String> = args_str.split(',').map(|s| s.trim().to_string()).collect();
                actions.insert(Action {
                    label: label.to_string(),
                    arguments,
                });
            } else {
                let label = part.trim();
                actions.insert(Action {
                    label: label.to_string(),
                    arguments: Vec::new(),
                });
            }
        }

        Ok(MultiAction { actions })
    }

    /// Constructs a multi-action from an ATerm representation.
    pub fn from_mcrl2_aterm(_term: ATerm) -> Self {
        unimplemented!("Cannot yet translate the mCRL2 terms");
    }
}

/// Represents a single action label, with its (data) arguments
#[derive(Clone, PartialOrd, Ord, PartialEq, Eq, Hash)]
pub struct Action {
    label: String,
    arguments: Vec<String>,
}

impl TransitionLabel for MultiAction {
    fn is_tau_label(&self) -> bool {
        self.actions.is_empty()
    }

    fn tau_label() -> Self {
        MultiAction { actions: VecSet::new() }
    }

    fn matches_label(&self, label: &String) -> bool {
        // TODO: Is this correct, now a|b matches a?
        self.actions.iter().any(|action| &action.label == label)
    }
}

impl fmt::Display for MultiAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.actions.is_empty() {
            write!(f, "τ")
        } else {
            write!(f, "{{{}}}", self.actions.iter().format("|"))
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.arguments.is_empty() {
            write!(f, "{}", self.label)
        } else {
            let args_str = self.arguments.join(", ");
            write!(f, "{}({})", self.label, args_str)
        }
    }
}

impl fmt::Debug for MultiAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Use the debug format to print the display format
        write!(f, "{}", self)
    }
}



#[cfg(test)]
mod tests {
    use crate::MultiAction;


    #[test]
    fn test_multi_action_parse_string() {
        let action = MultiAction::from_string("a | b(1, 2) | c").unwrap();

        assert_eq!(action.actions.len(), 3);
        assert!(action.actions.iter().any(|act| act.label == "a" && act.arguments.is_empty()));
        assert!(action.actions.iter().any(|act| act.label == "b" && act.arguments == vec!["1", "2"]));
        assert!(action.actions.iter().any(|act| act.label == "c" && act.arguments.is_empty()));
    }
}