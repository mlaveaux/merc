use std::rc::Rc;

use merc_data::DataExpression;
use merc_data::DataVariable;
use merc_data::split_and;
use merc_data::split_equality;
use merc_utilities::BooleanTerm;
use merc_utilities::optimized_conjunction;

use merc_lps::ActionSummand;
use merc_process::ActionNameMultiset;
use merc_process::ActionNameSet;
use merc_process::AllowSet;
use merc_process::CommunicationSet;

use crate::data::NameGenerator;
use crate::data::Substitution;
use crate::data::free_variables;
use crate::passes::gnf::substitute_action;

/// A stage that parallel composition and communication apply to every summand they generate.
///
/// mCRL2 applies `allow`, `block` and sum elimination while summands are generated rather than
/// afterwards: in a parallel composition most combinations of summands are removed by an
/// enclosing `allow`, and a summand that is built and then removed costs as much as one that is
/// kept. Stages compose as pairs, so that every pipeline is a monomorphised type.
pub(crate) trait SummandStage {
    /// Whether a summand with the action names `names` can be kept. Checked before the summand is
    /// built, so it must only depend on the names.
    fn admits(&self, _names: &ActionNameMultiset) -> bool {
        true
    }

    /// Transforms a summand that was admitted and built, or removes it.
    fn finish(&mut self, summand: ActionSummand, _names: &mut NameGenerator) -> Option<ActionSummand> {
        Some(summand)
    }
}

impl<A: SummandStage, B: SummandStage> SummandStage for (A, B) {
    fn admits(&self, names: &ActionNameMultiset) -> bool {
        self.0.admits(names) && self.1.admits(names)
    }

    fn finish(&mut self, summand: ActionSummand, names: &mut NameGenerator) -> Option<ActionSummand> {
        let summand = self.0.finish(summand, names)?;
        self.1.finish(summand, names)
    }
}

/// A restriction of the multi-actions of summands by name.
#[derive(Clone, Debug)]
pub(crate) enum Restriction<'a> {
    None,
    Allow(&'a AllowSet),
    /// An allow set computed from an alphabet, see [crate::passes::alphabet].
    Allowed(Rc<AllowSet>),
    Block(&'a ActionNameSet),
    /// Only sub-multisets of these multi-actions, or `tau`. This is the restriction an `allow`
    /// imposes on an operand whose multi-actions are later combined with others, `A^⊆` in mCRL2's
    /// alphabet reduction: a multi-action only grows when combined, so one that is not a
    /// sub-multiset of an allowed one never becomes allowed.
    Within(Rc<[ActionNameMultiset]>),
}

/// The bound on the number of multi-actions [Restriction::through_comm] computes before it gives up
/// restricting; inverting a communication multiplies the number of multi-actions.
const INVERSE_LIMIT: usize = 10_000;

impl<'a> Restriction<'a> {
    /// The restriction for the operands of a parallel composition, whose multi-actions are still
    /// combined with those of the other operands.
    pub(crate) fn for_operands(&self) -> Restriction<'a> {
        match self {
            Restriction::Allow(allow) => Restriction::Within(allow.iter().cloned().collect()),
            Restriction::Allowed(allow) => Restriction::Within(allow.iter().cloned().collect()),
            restriction => restriction.clone(),
        }
    }

    /// The restriction for the operand of `comm(C, …)`: the multi-actions that communicate into
    /// one this restriction admits, `C⁻¹(A)`.
    pub(crate) fn through_comm(&self, communications: &CommunicationSet) -> Restriction<'a> {
        let allowed: Vec<ActionNameMultiset> = match self {
            Restriction::Allow(allow) => allow.iter().cloned().collect(),
            Restriction::Allowed(allow) => allow.iter().cloned().collect(),
            Restriction::Within(allowed) => allowed.to_vec(),
            Restriction::None | Restriction::Block(_) => return Restriction::None,
        };

        let mut result = Vec::new();
        for multiset in &allowed {
            // Every name is either kept or the result of a communication.
            let mut candidates = vec![ActionNameMultiset::default()];
            for name in multiset.iter() {
                let mut next = Vec::new();
                for candidate in &candidates {
                    next.push(candidate.with(name.clone()));
                    for (lhs, target) in communications.iter() {
                        if target == name {
                            next.push(candidate.union(lhs));
                        }
                    }
                }
                if next.len() + result.len() > INVERSE_LIMIT {
                    return Restriction::None;
                }
                candidates = next;
            }
            result.extend(candidates);
        }
        Restriction::Within(result.into())
    }
}

impl SummandStage for Restriction<'_> {
    fn admits(&self, names: &ActionNameMultiset) -> bool {
        match self {
            Restriction::None => true,
            Restriction::Allow(allow) => allow.allows(names),
            Restriction::Allowed(allow) => allow.allows(names),
            Restriction::Block(block) => !names.iter().any(|name| block.contains(name)),
            Restriction::Within(allowed) => {
                names.is_empty() || names.is_terminate() || allowed.iter().any(|multiset| multiset.contains(names))
            }
        }
    }
}

/// Sum elimination: removes a summation variable `d` from a summand whose condition has
/// a conjunct `d == e` or `e == d` where `e` does not mention `d`, by substituting `e` for `d`.
/// Communication produces such conjuncts for every synchronisation with data. Summands whose
/// condition becomes `false` are removed.
pub(crate) struct SumElimination;

impl SummandStage for SumElimination {
    fn finish(&mut self, summand: ActionSummand, names: &mut NameGenerator) -> Option<ActionSummand> {
        let summand = eliminate_sums(summand, names);
        (!summand.condition.is_false()).then_some(summand)
    }
}

fn eliminate_sums(mut summand: ActionSummand, names: &mut NameGenerator) -> ActionSummand {
    loop {
        let conjuncts = split_and(&summand.condition);
        let Some((index, variable, value)) = conjuncts.iter().enumerate().find_map(|(index, conjunct)| {
            let (lhs, rhs) = split_equality(conjunct)?;
            [(&lhs, &rhs), (&rhs, &lhs)].into_iter().find_map(|(side, value)| {
                let variable = summation_variable(&summand, side)?;
                (!free_variables(value).contains(&variable)).then(|| (index, variable, value.clone()))
            })
        }) else {
            return summand;
        };

        let sigma = Substitution::new([(variable.clone(), value)]);
        summand.summation_variables.retain(|sum| sum != &variable);
        summand.condition = optimized_conjunction(
            conjuncts
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .map(|(_, conjunct)| sigma.apply(conjunct, names)),
        );
        summand.multi_action = substitute_action(&summand.multi_action, &sigma, names);
        summand.next_state = sigma.apply_all(&summand.next_state, names);
    }
}

/// The summation variable of `summand` that `expr` is, if it is one.
fn summation_variable(summand: &ActionSummand, expr: &DataExpression) -> Option<DataVariable> {
    summand
        .summation_variables
        .iter()
        .find(|variable| DataExpression::from((*variable).clone()) == *expr)
        .cloned()
}
