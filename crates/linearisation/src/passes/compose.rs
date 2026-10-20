use std::collections::HashSet;

use merc_data::DataExpression;
use merc_utilities::BooleanTerm;
use merc_utilities::optimized_and;

use merc_lps::ActionSummand;
use merc_lps::DeadlockSummand;
use merc_lps::LinearProcess;
use merc_process::Action;
use merc_process::ActionLabel;
use merc_process::ActionNameSet;
use merc_process::CommunicationSet;
use merc_process::MultiAction;
use merc_process::RenameMap;

use crate::data::NameGenerator;
use crate::data::Substitution;
use crate::passes::communication::communicate;
use crate::passes::encode::Component;
use crate::passes::gnf::substitute_action;
use crate::passes::split::Operator;
use crate::passes::stage::Restriction;
use crate::passes::stage::SummandStage;

/// The parallel composition of two linear processes, with `stage` applied to every
/// summand.
///
/// Every pair of summands is a candidate synchronisation; [SummandStage::admits] is checked on its
/// name multiset before the summand is built, because most pairs are removed in practice. The
/// termination action only synchronises with itself.
pub(crate) fn parallel<S: SummandStage>(
    lhs: Component,
    rhs: Component,
    stage: &mut S,
    names: &mut NameGenerator,
) -> Component {
    let rhs = rename_apart(&lhs, rhs, names);
    let identity_lhs: Vec<DataExpression> = lhs
        .process
        .parameters
        .iter()
        .cloned()
        .map(DataExpression::from)
        .collect();
    let identity_rhs: Vec<DataExpression> = rhs
        .process
        .parameters
        .iter()
        .cloned()
        .map(DataExpression::from)
        .collect();

    let mut action_summands = Vec::new();
    let interleavings = lhs
        .process
        .action_summands
        .iter()
        .map(|summand| (summand, true))
        .chain(rhs.process.action_summands.iter().map(|summand| (summand, false)));
    for (summand, left) in interleavings {
        if summand.multi_action.is_terminate() || !stage.admits(&summand.multi_action.names()) {
            continue;
        }
        let next_state = if left {
            summand.next_state.iter().chain(&identity_rhs).cloned().collect()
        } else {
            identity_lhs.iter().chain(&summand.next_state).cloned().collect()
        };
        let summand = ActionSummand {
            next_state,
            ..summand.clone()
        };
        action_summands.extend(stage.finish(summand, names));
    }

    for left in &lhs.process.action_summands {
        for right in &rhs.process.action_summands {
            let multi_action = match (left.multi_action.is_terminate(), right.multi_action.is_terminate()) {
                (true, true) => MultiAction::terminate(),
                (false, false) => {
                    if !stage.admits(&left.multi_action.names().union(&right.multi_action.names())) {
                        continue;
                    }
                    left.multi_action.union(&right.multi_action)
                }
                _ => continue,
            };
            let condition = optimized_and(left.condition.clone(), right.condition.clone());
            if condition.is_false() {
                continue;
            }
            let summand = ActionSummand {
                summation_variables: left
                    .summation_variables
                    .iter()
                    .chain(&right.summation_variables)
                    .cloned()
                    .collect(),
                condition,
                multi_action,
                next_state: left.next_state.iter().chain(&right.next_state).cloned().collect(),
            };
            action_summands.extend(stage.finish(summand, names));
        }
    }

    let mut deadlock_summands = lhs.process.deadlock_summands;
    deadlock_summands.extend(rhs.process.deadlock_summands);
    let mut parameters = lhs.process.parameters;
    parameters.extend(rhs.process.parameters);
    let mut initial_state = lhs.initial_state;
    initial_state.extend(rhs.initial_state);
    let mut global_variables = lhs.global_variables;
    global_variables.extend(rhs.global_variables);

    Component {
        process: LinearProcess {
            parameters,
            action_summands,
            deadlock_summands,
        },
        initial_state,
        global_variables,
    }
}

/// Renames the parameters of `rhs` that clash with those of `lhs`, and the summation variables of
/// `rhs` that clash with any variable of `lhs`, so that the two can be combined summand by summand.
fn rename_apart(lhs: &Component, mut rhs: Component, names: &mut NameGenerator) -> Component {
    let mut taken: HashSet<String> = lhs
        .process
        .parameters
        .iter()
        .map(|parameter| parameter.name().to_string())
        .collect();
    taken.extend(
        lhs.process
            .action_summands
            .iter()
            .flat_map(|summand| &summand.summation_variables)
            .map(|variable| variable.name().to_string()),
    );

    let mut renaming = Substitution::default();
    for parameter in &mut rhs.process.parameters {
        if taken.contains(parameter.name()) {
            let fresh = names.variable(parameter);
            renaming.insert(parameter.clone(), DataExpression::from(fresh.clone()));
            *parameter = fresh;
        }
    }
    for name in &taken {
        renaming.avoid(name);
    }

    for summand in &mut rhs.process.action_summands {
        let (sums, sigma) = renaming.bind(&summand.summation_variables, names);
        summand.summation_variables = sums;
        summand.condition = sigma.apply(&summand.condition, names);
        summand.multi_action = substitute_action(&summand.multi_action, &sigma, names);
        summand.next_state = sigma.apply_all(&summand.next_state, names);
    }
    for summand in &mut rhs.process.deadlock_summands {
        let (sums, sigma) = renaming.bind(&summand.summation_variables, names);
        summand.summation_variables = sums;
        summand.condition = sigma.apply(&summand.condition, names);
    }
    rhs
}

/// Applies `operator` to every summand of `component`, and `stage` to the summands that
/// communication produces.
pub(crate) fn apply<S: SummandStage>(
    operator: &Operator,
    mut component: Component,
    stage: &mut S,
    names: &mut NameGenerator,
) -> Component {
    let process = &mut component.process;
    match operator {
        Operator::Hide(hidden) => hide(process, hidden),
        Operator::Rename(renaming) => rename(process, renaming),
        Operator::Allow(allow) => restrict(process, &Restriction::Allow(allow)),
        Operator::Block(block) => restrict(process, &Restriction::Block(block)),
        Operator::Comm(communications) => comm(process, communications, stage, names),
    }
    component
}

fn hide(process: &mut LinearProcess, hidden: &ActionNameSet) {
    for summand in &mut process.action_summands {
        summand.multi_action = summand
            .multi_action
            .map_actions(|action| (!hidden.contains(&action.label().name().protect())).then(|| action.clone()));
    }
}

fn rename(process: &mut LinearProcess, renaming: &RenameMap) {
    for summand in &mut process.action_summands {
        summand.multi_action = summand.multi_action.map_actions(|action| {
            let label = action.label().protect();
            Some(match renaming.get(&label.name().protect()) {
                Some(target) => Action::new(
                    &ActionLabel::new(target.value(), &label.sorts().to_vec()),
                    &action.arguments().to_vec(),
                ),
                None => action.clone(),
            })
        });
    }
}

/// Removes the summands `restriction` does not admit; they become deadlock summands, as in mCRL2.
pub(crate) fn restrict(process: &mut LinearProcess, restriction: &Restriction<'_>) {
    let (kept, removed): (Vec<ActionSummand>, Vec<ActionSummand>) = std::mem::take(&mut process.action_summands)
        .into_iter()
        .partition(|summand| restriction.admits(&summand.multi_action.names()));
    process.action_summands = kept;
    process
        .deadlock_summands
        .extend(removed.into_iter().map(|summand| DeadlockSummand {
            summation_variables: summand.summation_variables,
            condition: summand.condition,
        }));
}

fn comm<S: SummandStage>(
    process: &mut LinearProcess,
    communications: &CommunicationSet,
    stage: &mut S,
    names: &mut NameGenerator,
) {
    let summands = std::mem::take(&mut process.action_summands);
    for summand in summands {
        for (multi_action, condition) in communicate(&summand.multi_action, communications) {
            if !stage.admits(&multi_action.names()) {
                continue;
            }
            let condition = optimized_and(summand.condition.clone(), condition);
            if condition.is_false() {
                continue;
            }
            let summand = ActionSummand {
                summation_variables: summand.summation_variables.clone(),
                condition,
                multi_action,
                next_state: summand.next_state.clone(),
            };
            process.action_summands.extend(stage.finish(summand, names));
        }
    }
}
