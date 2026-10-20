use std::collections::BTreeSet;

use merc_aterm::ATermString;
use merc_process::ActionNameMultiset;
use merc_process::CommunicationSet;

use crate::passes::encode::Component;
use crate::passes::split::Composition;
use crate::passes::split::Operator;

/// The multisets of action names that the summands of a process can have.
pub(crate) type Alphabet = BTreeSet<ActionNameMultiset>;

/// The size beyond which [alphabet] gives up; the alphabet of a parallel composition can grow
/// exponentially in the number of operands.
const ALPHABET_LIMIT: usize = 10_000;

/// The alphabet of `composition`, an over-approximation of the name multisets of the summands it
/// linearises to, or `None` when it has more than [ALPHABET_LIMIT] elements.
///
/// This is mCRL2's alphabet `α`, computed on names only: a parallel composition can combine any
/// multi-actions of its operands, and an operator transforms each multi-action as [outcomes] does.
/// Recurses once per operator of the parallel layer.
pub(crate) fn alphabet(composition: &Composition<Component>) -> Option<Alphabet> {
    let result: Alphabet = match composition {
        Composition::Leaf(component) => component
            .process
            .action_summands
            .iter()
            .map(|summand| summand.multi_action.names())
            .collect(),
        Composition::Parallel(operands) => {
            let mut result = Alphabet::new();
            for operand in operands {
                let operand = alphabet(operand)?;
                let mut combined = Alphabet::new();
                for lhs in result.iter().filter(|names| !names.is_terminate()) {
                    for rhs in operand.iter().filter(|names| !names.is_terminate()) {
                        combined.insert(lhs.union(rhs));
                    }
                    if combined.len() > ALPHABET_LIMIT {
                        return None;
                    }
                }
                result.extend(operand);
                result.extend(combined);
            }
            result
        }
        Composition::Operator(operator, operand) => alphabet(operand)?
            .iter()
            .flat_map(|names| outcomes(operator, names))
            .collect(),
    };
    (result.len() <= ALPHABET_LIMIT).then_some(result)
}

/// The name multisets that `operator` can turn a multi-action with the names `names` into; none
/// when it removes the multi-action, and several for the possible communications.
pub(crate) fn outcomes(operator: &Operator, names: &ActionNameMultiset) -> Vec<ActionNameMultiset> {
    match operator {
        Operator::Allow(allow) => {
            if allow.allows(names) {
                vec![names.clone()]
            } else {
                Vec::new()
            }
        }
        Operator::Block(block) => {
            if names.iter().any(|name| block.contains(name)) {
                Vec::new()
            } else {
                vec![names.clone()]
            }
        }
        Operator::Hide(hidden) => vec![ActionNameMultiset::new(
            names.iter().filter(|name| !hidden.contains(name)).cloned(),
        )],
        Operator::Rename(renaming) => vec![ActionNameMultiset::new(
            names.iter().map(|name| renaming.get(name).unwrap_or(name).clone()),
        )],
        Operator::Comm(communications) => {
            let names: Vec<ATermString> = names.iter().cloned().collect();
            let mut result = Vec::new();
            communicate_names(&names, Vec::new(), communications, &mut result);
            result
        }
    }
}

/// Adds to `result` the multisets in which some disjoint left-hand sides of `communications` in
/// `names` are replaced by their targets, together with `kept`.
///
/// Recurses once per name.
fn communicate_names(
    names: &[ATermString],
    kept: Vec<ATermString>,
    communications: &CommunicationSet,
    result: &mut Vec<ActionNameMultiset>,
) {
    let Some((first, rest)) = names.split_first() else {
        result.push(ActionNameMultiset::new(kept));
        return;
    };

    // `first` does not communicate, …
    let mut without = kept.clone();
    without.push(first.clone());
    communicate_names(rest, without, communications, result);

    // … or it takes part in the one communication whose left-hand side contains it.
    for (lhs, target) in communications.iter() {
        if !lhs.iter().any(|name| name == first) {
            continue;
        }
        let mut remaining = rest.to_vec();
        let partners = lhs.iter().skip_while(|name| *name != first).skip(1);
        let others = lhs.iter().take_while(|name| *name != first);
        if others
            .chain(partners)
            .all(|partner| match remaining.iter().position(|name| name == partner) {
                Some(position) => {
                    remaining.remove(position);
                    true
                }
                None => false,
            })
        {
            let mut with = kept.clone();
            with.push(target.clone());
            communicate_names(&remaining, with, communications, result);
        }
    }
}
