use merc_data::DataExpression;
use merc_utilities::BooleanTerm;
use merc_utilities::optimized_and;
use merc_utilities::optimized_not;

use merc_process::Action;
use merc_process::ActionLabel;
use merc_process::ActionNameMultiset;
use merc_process::CommunicationSet;
use merc_process::MultiAction;

use crate::data::equal_vectors;

/// The communication function on a multi-action with open data, `γ̄(m, C, [])` of "Calculation of
/// Communication with Open Terms" (van Weerdenburg), in the variant with an accumulator that mCRL2
/// implements in `linearise_communication.h`.
///
/// Returns the possible outcomes, each with the condition under which it is the result. The
/// conditions are mutually exclusive: an outcome where some actions do not communicate requires
/// that they cannot. Outcomes with condition `false` are omitted.
///
/// The work is exponential in the number of actions of `multi_action` that occur in a left-hand
/// side of `communications`, and the recursion depth is linear in the number of actions.
pub(crate) fn communicate(
    multi_action: &MultiAction,
    communications: &CommunicationSet,
) -> Vec<(MultiAction, DataExpression)> {
    gamma(multi_action.actions(), communications, Vec::new())
        .into_iter()
        .filter(|(_, condition)| !condition.is_false())
        .map(|(actions, condition)| (MultiAction::new(actions), condition))
        .collect()
}

type Outcomes = Vec<(Vec<Action>, DataExpression)>;

/// `γ̄(m, C, r)`: the outcomes of `m`, where `r` are actions already decided not to communicate.
fn gamma(m: &[Action], c: &CommunicationSet, r: Vec<Action>) -> Outcomes {
    let Some((first, rest)) = m.split_first() else {
        let condition = psi(&r, c);
        return vec![(r, condition)];
    };

    // `first` takes part in a communication, …
    let mut result = phi(
        std::slice::from_ref(first),
        &first.arguments().to_vec(),
        &[],
        rest,
        c,
        &r,
    );
    // … or it does not.
    let mut r = r;
    r.push(first.clone());
    result.extend(gamma(rest, c, r));
    result
}

/// `φ(m, d⃗, w, n, C, r)`: the outcomes in which `m`, extended with a part of `n`, communicates
/// with arguments `d⃗`; the actions of `n` that are not used are moved to `w`.
fn phi(m: &[Action], d: &[DataExpression], w: &[Action], n: &[Action], c: &CommunicationSet, r: &[Action]) -> Outcomes {
    let names = names_of(m);
    if !c.is_partial_lhs(&names) {
        return Vec::new();
    }

    let Some((next, rest)) = n.split_first() else {
        let Some(target) = c.target(&names) else {
            return Vec::new();
        };
        let label = ActionLabel::new(target.value(), &m[0].label().sorts().to_vec());
        let action = Action::new(&label, d);
        return gamma(w, c, r.to_vec())
            .into_iter()
            .map(|(mut actions, condition)| {
                actions.push(action.clone());
                (actions, condition)
            })
            .collect();
    };

    // `next` takes part in the communication, which requires equal arguments, …
    let mut result = Vec::new();
    let arguments = next.arguments().to_vec();
    if arguments.len() == d.len() {
        let mut extended = m.to_vec();
        extended.push(next.clone());
        let equal = equal_vectors(&arguments, d);
        result.extend(
            phi(&extended, d, w, rest, c, r)
                .into_iter()
                .map(|(actions, condition)| (actions, optimized_and(condition, equal.clone()))),
        );
    }
    // … or it does not.
    let mut w = w.to_vec();
    w.push(next.clone());
    result.extend(phi(m, d, &w, rest, c, r));
    result
}

/// `ψ(m, C)`: the condition under which the actions of `m` cannot communicate with each other.
fn psi(m: &[Action], c: &CommunicationSet) -> DataExpression {
    let mut result = DataExpression::true_();
    for (index, action) in m.iter().enumerate() {
        result = optimized_and(result, psi_prime(action, &m[index + 1..], c));
    }
    result
}

/// `ψ'(a(d⃗), m, C)`: the condition under which `a(d⃗)` communicates with no action of `m`.
fn psi_prime(action: &Action, m: &[Action], c: &CommunicationSet) -> DataExpression {
    let mut result = DataExpression::true_();
    let arguments = action.arguments().to_vec();
    for (index, other) in m.iter().enumerate() {
        let pair = names_of(&[action.clone(), other.clone()]);
        let other_arguments = other.arguments().to_vec();
        if c.is_partial_lhs(&pair) && arguments.len() == other_arguments.len() && xi(&pair, &m[index + 1..], c) {
            result = optimized_and(result, optimized_not(equal_vectors(&arguments, &other_arguments)));
        }
    }
    result
}

/// `ξ(m, n, C)`: whether `m`, extended with a part of `n`, is a left-hand side of `C`.
fn xi(m: &ActionNameMultiset, n: &[Action], c: &CommunicationSet) -> bool {
    let Some((next, rest)) = n.split_first() else {
        return c.target(m).is_some();
    };
    let extended = m.with(next.label().name().protect());
    if c.target(&extended).is_some() {
        true
    } else if c.extends(&extended) {
        xi(&extended, rest, c) || xi(m, rest, c)
    } else {
        xi(m, rest, c)
    }
}

fn names_of(actions: &[Action]) -> ActionNameMultiset {
    ActionNameMultiset::new(actions.iter().map(|action| action.label().name().protect()))
}
