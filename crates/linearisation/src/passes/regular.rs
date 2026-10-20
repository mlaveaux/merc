use std::collections::HashMap;
use std::collections::HashSet;
use std::convert::Infallible;
use std::ops::ControlFlow;

use merc_data::DataAbstractionRef;
use merc_data::DataExpression;
use merc_data::DataExpressionVisitor;
use merc_data::DataVariable;
use merc_data::DataWhereClauseRef;
use merc_data::Mcrl2DataSpecification;
use merc_data::is_data_application;
use merc_process::MultiAction;
use merc_process::ProcessExpression;
use merc_process::ProcessIdentifier;
use merc_sabre::RewriteEngine;
use merc_utilities::Visit;

use crate::LinearisationError;
use crate::data::occurring;
use crate::passes::gnf::Gnf;
use crate::passes::gnf::Head;
use crate::process::Call;

/// The normalised calls of a sequence and the variables it uses, which identify its process.
type SequenceKey = (Vec<(ProcessIdentifier, Vec<DataExpression>)>, Vec<DataVariable>);

/// Normalises the arguments of a sequence of calls, so that sequences that are equal modulo the
/// data equations are represented by one process.
pub(crate) trait KeyNormaliser {
    /// The normal form of `expr`.
    fn normalise(&mut self, expr: &DataExpression) -> DataExpression;
}

/// Rewrites arguments to normal form with the specification's rewrite rules, as mCRL2 does.
///
/// The rewriter is built on first use: building it can be expensive, and most specifications never
/// produce a sequence of two or more calls. Arguments that are variables or constants are already
/// in normal form and do not need it.
pub(crate) struct Rewriting<'a, R, F> {
    data_specification: &'a Mcrl2DataSpecification,
    rewriter: Option<R>,
    make_rewriter: Option<F>,
}

impl<'a, R, F> Rewriting<'a, R, F>
where
    R: RewriteEngine,
    F: FnOnce(&Mcrl2DataSpecification) -> R,
{
    pub(crate) fn new(data_specification: &'a Mcrl2DataSpecification, make_rewriter: F) -> Self {
        Rewriting {
            data_specification,
            rewriter: None,
            make_rewriter: Some(make_rewriter),
        }
    }
}

impl<R, F> KeyNormaliser for Rewriting<'_, R, F>
where
    R: RewriteEngine,
    F: FnOnce(&Mcrl2DataSpecification) -> R,
{
    fn normalise(&mut self, expr: &DataExpression) -> DataExpression {
        // The rewriter does not support binders and where clauses, and leaves are normal forms.
        if !is_data_application(expr) || BinderSearch.visit(&expr.copy(), ()).is_some() {
            return expr.clone();
        }
        let rewriter = match &mut self.rewriter {
            Some(rewriter) => rewriter,
            None => {
                let make_rewriter = self.make_rewriter.take().expect("the rewriter is made once");
                self.rewriter.insert(make_rewriter(self.data_specification))
            }
        };
        rewriter.rewrite(expr)
    }
}

struct BinderSearch;

impl DataExpressionVisitor for BinderSearch {
    type Context = ();
    type Break = ();
    type Error = Infallible;

    fn visit_binder(
        &mut self,
        _binder: &DataAbstractionRef<'_>,
        _context: (),
    ) -> Visit<Infallible, (), (), Infallible> {
        Ok(ControlFlow::Break(()))
    }

    fn visit_where_clause(
        &mut self,
        _clause: &DataWhereClauseRef<'_>,
        _context: (),
    ) -> Visit<Infallible, (), (), Infallible> {
        Ok(ControlFlow::Break(()))
    }
}

/// A summand of a body in regular form: an action or `δ`, followed by at most one call.
///
/// Invariant: a summand with `action == None` (that is `δ`) has no `next`.
#[derive(Clone, Debug)]
pub(crate) struct RegularSummand {
    pub(crate) sums: Vec<DataVariable>,
    pub(crate) condition: DataExpression,
    pub(crate) action: Option<MultiAction>,
    pub(crate) next: Option<Call>,
}

/// An equation in regular form.
#[derive(Clone, Debug)]
pub(crate) struct RegularProcess {
    pub(crate) id: ProcessIdentifier,
    pub(crate) summands: Vec<RegularSummand>,
}

/// The regular-form pass: every tail of two or more calls is replaced by a call to a
/// process standing for that sequence. Returns the equations reachable from `root`, `root` first.
pub(crate) fn regular_form<N: KeyNormaliser>(
    gnf: &mut Gnf<'_>,
    root: &ProcessIdentifier,
    normaliser: &mut N,
    limit: usize,
) -> Result<Vec<RegularProcess>, LinearisationError> {
    let mut sequences: HashMap<SequenceKey, ProcessIdentifier> = HashMap::new();
    let mut result = Vec::new();
    let mut visited = HashSet::from([root.clone()]);
    let mut worklist = vec![root.clone()];

    while let Some(id) = worklist.pop() {
        let parameters = id.parameters().to_vec();
        let mut summands = Vec::new();
        for summand in gnf.summands(&id)?.iter() {
            let action = match &summand.head {
                Head::Action(action) => action.clone(),
                Head::Delta => {
                    summands.push(RegularSummand {
                        sums: summand.sums.clone(),
                        condition: summand.condition.clone(),
                        action: None,
                        next: None,
                    });
                    continue;
                }
                Head::Call(_) => unreachable!("GNF has no leading calls"),
            };

            // Everything after a call that cannot terminate is unreachable.
            let length = summand
                .tail
                .iter()
                .position(|call| !gnf.terminates(&call.process))
                .map_or(summand.tail.len(), |index| index + 1);
            let tail = &summand.tail[..length];

            let next = match tail {
                [] => None,
                [call] => Some(call.clone()),
                calls => {
                    let mut scope = parameters.clone();
                    scope.extend(summand.sums.iter().cloned());
                    Some(sequence_call(
                        gnf,
                        calls,
                        &scope,
                        normaliser,
                        &mut sequences,
                        root,
                        limit,
                    )?)
                }
            };
            if let Some(call) = &next
                && visited.insert(call.process.clone())
            {
                worklist.push(call.process.clone());
            }
            summands.push(RegularSummand {
                sums: summand.sums.clone(),
                condition: summand.condition.clone(),
                action: Some(action),
                next,
            });
        }
        result.push(RegularProcess { id, summands });
    }

    Ok(result)
}

/// A call to the process `Y(x⃗) = X₁(e⃗₁) · … · Xₙ(e⃗ₙ)`, where `x⃗` are the variables of `scope`
/// that the sequence uses. Sequences with equal normal forms over the same variables share `Y`.
fn sequence_call<N: KeyNormaliser>(
    gnf: &mut Gnf<'_>,
    calls: &[Call],
    scope: &[DataVariable],
    normaliser: &mut N,
    sequences: &mut HashMap<SequenceKey, ProcessIdentifier>,
    root: &ProcessIdentifier,
    limit: usize,
) -> Result<Call, LinearisationError> {
    let parameters = occurring(scope, calls.iter().flat_map(|call| &call.arguments));
    let mut key = Vec::with_capacity(calls.len());
    for call in calls {
        key.push((
            call.process.clone(),
            call.arguments
                .iter()
                .map(|argument| normaliser.normalise(argument))
                .collect(),
        ));
    }
    let key = (key, parameters.clone());

    let process = match sequences.get(&key) {
        Some(process) => process.clone(),
        None => {
            if sequences.len() >= limit {
                return Err(LinearisationError::NonRegular {
                    process: root.name().value().to_string(),
                    limit,
                });
            }
            let body = calls
                .iter()
                .map(Call::expression)
                .reduce(|lhs, rhs| ProcessExpression::seq(&lhs, &rhs))
                .expect("a sequence has at least two calls");
            let process = gnf.add_equation("Y", &parameters, body);
            sequences.insert(key, process.clone());
            process
        }
    };
    Ok(Call {
        process,
        arguments: parameters.into_iter().map(DataExpression::from).collect(),
    })
}
