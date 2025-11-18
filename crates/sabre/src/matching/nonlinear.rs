#![forbid(unsafe_code)]

use std::fmt;

use itertools::Itertools;
use merc_data::DataVariable;
use merc_data::DataVariableRef;
use merc_data::is_data_variable;

use crate::Rule;
use crate::utilities::DataPosition;
use crate::utilities::DataPositionIndexed;
use crate::utilities::DataPositionIterator;

/// An equivalence class is a variable with (multiple) positions. This is
/// necessary for non-linear patterns.
///
/// # Example
/// Suppose we have a pattern f(x,x), where x is a variable. Then it will have
/// one equivalence class storing "x" and the positions 1 and 2. The function
/// equivalences_hold checks whether the term has the same term on those
/// positions. For example, it will returns false on the term f(a, b) and true
/// on the term f(a, a).
#[derive(Hash, Clone, Eq, PartialEq, Ord, PartialOrd, Debug)]
pub struct EquivalenceClass {
    pub variable: DataVariable,
    pub positions: Vec<DataPosition>,
}

/// Derives the positions in a pattern with same variable (for non-linear patters)
pub fn derive_equivalence_classes(rule: &Rule) -> Vec<EquivalenceClass> {
    let mut var_equivalences = vec![];

    for (term, pos) in DataPositionIterator::new(rule.lhs.copy()) {
        if is_data_variable(&term) {
            // Register the position of the variable
            update_equivalences(&mut var_equivalences, &DataVariableRef::from(term), pos);
        }
    }

    // Discard variables that only occur once
    var_equivalences.retain(|x| x.positions.len() > 1);
    var_equivalences
}

/// Checks if the equivalence classes hold for the given term.
pub fn check_equivalence_classes<'a, T, P>(term: &'a P, eqs: &[EquivalenceClass]) -> bool
where
    P: DataPositionIndexed<'a, Target<'a> = T> + 'a,
    T: PartialEq,
{
    eqs.iter().all(|ec| {
        debug_assert!(
            ec.positions.len() >= 2,
            "An equivalence class must contain at least two positions"
        );

        // The term at the first position must be equivalent to all other positions.
        let mut iter_pos = ec.positions.iter();
        let first = iter_pos.next().unwrap();
        iter_pos.all(|other_pos| term.get_data_position(first) == term.get_data_position(other_pos))
    })
}

/// Adds the position of a variable to the equivalence classes
fn update_equivalences(ve: &mut Vec<EquivalenceClass>, variable: &DataVariableRef<'_>, pos: DataPosition) {
    // Check if the variable was seen before
    if ve.iter().any(|ec| ec.variable.copy() == *variable) {
        for ec in ve.iter_mut() {
            // Find the equivalence class and add the position
            if ec.variable.copy() == *variable && !ec.positions.iter().any(|x| x == &pos) {
                ec.positions.push(pos);
                break;
            }
        }
    } else {
        // If the variable was not found at another position add a new equivalence class
        ve.push(EquivalenceClass {
            variable: variable.protect(),
            positions: vec![pos],
        });
    }
}

impl fmt::Display for EquivalenceClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{{ {} }}", self.variable, self.positions.iter().format(", "))
    }
}

#[cfg(test)]
mod tests {
    use merc_data::DataExpression;
    use merc_data::DataVariable;

    use crate::test_utility::create_rewrite_rule;

    use super::*;

    #[test]
    fn test_derive_equivalence_classes() {
        let eq: Vec<EquivalenceClass> =
            derive_equivalence_classes(&create_rewrite_rule("f(x, h(x))", "result", &["x"]).unwrap());

        assert_eq!(
            eq,
            vec![EquivalenceClass {
                variable: DataVariable::new("x").into(),
                positions: vec![DataPosition::new(&[1]), DataPosition::new(&[2, 1])]
            },],
            "The resulting config stack is not as expected"
        );

        // Check the equivalence class for an example
        let expression = DataExpression::from_string("f(a(b), h(a(b)))").unwrap();

        assert!(
            check_equivalence_classes(&expression, &eq),
            "The equivalence classes are not checked correctly, equivalences: {:?} and term {}",
            &eq,
            &expression
        );
    }
}
