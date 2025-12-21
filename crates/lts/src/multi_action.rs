use std::{fmt, hash::Hash};

use itertools::Itertools;
use merc_utilities::{MercError, VecSet};

/// A common trait for all transition labels. Ensuring that they are orderable, comparable, and hashable.
pub trait TransitionLabel: Ord + Hash + Eq + Clone + fmt::Display + fmt::Debug {
    /// Returns the tau label for this transition label type.
    fn tau_label() -> Self;

    /// Returns true iff this label is the tau label.
    fn is_tau_label(&self) -> bool {
        self == &Self::tau_label()
    }
}


/// Represents a multi-action, i.e., a set of action labels
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
}

/// Represents a single action label, with its (data) arguments
#[derive(PartialOrd, Ord, PartialEq, Eq)]
pub struct Action {
    label: String,
    arguments: Vec<String>,
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

#[cfg(test)]
mod tests {
    use crate::MultiAction;


    #[test]
    fn test_multi_action_parse_string() {
        let action = MultiAction::from_string("a | b(1, 2) | c").unwrap();
    }
}