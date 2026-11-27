use std::collections::HashSet;

use merc_syntax::SortExpressionKind;
use merc_syntax::SortId;
use merc_syntax::UntypedDataSpecification;

use crate::argument_sorts;
use crate::target_sort;

/// Returns the set of syntactically non-empty sorts.
///
/// A sort that is the target of at least one constructor can be empty; every
/// other declared sort (an abstract sort or an alias) is unconstrained and
/// assumed non-empty. The set is therefore seeded with those non-constructor
/// sorts and then grown as the least fixpoint of the non-emptiness rule: a
/// constructor sort becomes non-empty as soon as it has a constructor all of
/// whose argument sorts are non-empty. Built-in, container and function argument
/// sorts are treated as non-empty.
pub(crate) fn nonempty_sorts(spec: &UntypedDataSpecification) -> HashSet<SortId> {
    let constructor_sorts: HashSet<SortId> = spec
        .constructor_declarations
        .iter()
        .filter_map(|constructor| match &target_sort(&constructor.sort).node {
            SortExpressionKind::Resolved(_, id) => Some(*id),
            _ => None,
        })
        .collect();

    let mut nonempty: HashSet<SortId> = spec
        .sort_declarations
        .iter()
        .map(|declaration| declaration.id.expect("Name must have been resolved"))
        .filter(|id| !constructor_sorts.contains(id))
        .collect();

    let mut changed = true;
    while changed {
        changed = false;
        for constructor in &spec.constructor_declarations {
            let SortExpressionKind::Resolved(_, target) = &target_sort(&constructor.sort).node else {
                unreachable!("The target sort of a constructor should be a resolved sort");
            };

            if nonempty.contains(target) {
                continue;
            }

            let all_arguments_nonempty =
                argument_sorts(&constructor.sort)
                    .iter()
                    .all(|argument| match &argument.node {
                        SortExpressionKind::Resolved(_, id) => nonempty.contains(id),
                        _ => true,
                    });

            if all_arguments_nonempty {
                nonempty.insert(*target);
                changed = true;
            }
        }
    }

    nonempty
}
