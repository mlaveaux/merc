use std::rc::Rc;

use merc_data::DataVariable;
use merc_data::Mcrl2DataSpecification;
use merc_lps::LinearProcessSpecification;
use merc_process::ActionLabel;
use merc_process::AllowSet;
use merc_process::ProcessSpecification;
use merc_sabre::InnermostRewriter;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;
use merc_syntax::Sort;
use merc_typecheck::NumberEncoding;

use crate::LinearisationError;
use crate::data::NameGenerator;
use crate::passes::alphabet::alphabet;
use crate::passes::alphabet::outcomes;
use crate::passes::check::check;
use crate::passes::compose::apply;
use crate::passes::compose::parallel;
use crate::passes::compose::restrict;
use crate::passes::encode::Component;
use crate::passes::encode::Representatives;
use crate::passes::encode::encode;
use crate::passes::gnf::Gnf;
use crate::passes::regular::KeyNormaliser;
use crate::passes::regular::Rewriting;
use crate::passes::regular::regular_form;
use crate::passes::simplify::simplify;
use crate::passes::split::Composition;
use crate::passes::split::Operator;
use crate::passes::split::split;
use crate::passes::stage::Restriction;
use crate::passes::stage::SumElimination;
use crate::passes::stage::SummandStage;
use crate::passes::termination::Termination;
use crate::process::Call;
use crate::process::ProcessTable;
use crate::process::variable_names;

/// Options of [linearise].
#[derive(Clone, Debug)]
pub struct LinearisationOptions {
    /// The number of sequence processes the regular method may introduce before it gives up,
    /// reporting that the process is probably not regular.
    pub sequence_limit: usize,
}

impl Default for LinearisationOptions {
    fn default() -> Self {
        LinearisationOptions { sequence_limit: 10_000 }
    }
}

/// A rewriter for `data_specification`, which [linearise] can use.
///
/// This is an innermost rewriter rather than a `SabreRewriter`: building the latter's set automaton
/// for the hundreds of system-defined equations of even a small specification does not finish in
/// reasonable time, while linearisation only rewrites a few terms.
pub fn rewriter(data_specification: &Mcrl2DataSpecification) -> InnermostRewriter {
    InnermostRewriter::new(&RewriteSpecification::from_data_specification(data_specification))
}

/// Linearises `spec` with mCRL2's regular method.
///
/// The passes, each in its own module, are:
///
/// 1. check: reject the constructs that are not supported;
/// 2. termination: determine which equations can terminate;
/// 3. split: separate the parallel layer of `init` from the sequential processes below it, and make
///    their termination observable;
/// 4. for every sequential process: Greibach normal form, regular form (sequences of calls become
///    single calls) and encoding as a linear process with a control parameter;
/// 5. composition of these linear processes along the parallel layer, applying `allow`, `block`,
///    communication and sum elimination while the summands are generated;
/// 6. sum elimination on the result, and removal of the deadlock summands.
///
/// `make_rewriter` builds the rewriter that normalises the arguments of call sequences in the
/// regular form, from the data specification of `spec`, for instance [rewriter]. It is only called
/// when such a sequence occurs.
pub fn linearise<R, F>(
    spec: &ProcessSpecification,
    make_rewriter: F,
    options: &LinearisationOptions,
) -> Result<LinearProcessSpecification, LinearisationError>
where
    R: RewriteEngine,
    F: FnOnce(&Mcrl2DataSpecification) -> R,
{
    check(spec)?;
    let init = spec.init.as_ref().ok_or(LinearisationError::MissingInit)?;

    let mut names = NameGenerator::new(used_names(spec));
    let mut table = ProcessTable::new(&spec.equations);
    let mut termination = Termination::analyse(&table);
    let composition = split(&mut table, &mut termination, &mut names, init, &spec.global_variables)?;

    let data_specification = &spec.data_specification;
    let encoding = NumberEncoding::of(data_specification);
    let representatives = Representatives::new(data_specification.constructors(), encoding);
    let mut normaliser = Rewriting::new(data_specification, make_rewriter);
    let mut composer = Composer {
        table: &mut table,
        termination: &mut termination,
        names: &mut names,
        normaliser: &mut normaliser,
        representatives: &representatives,
        options,
    };
    let leaves = composer.leaves(&composition)?;
    let mut component = composer.compose(leaves, Restriction::None);
    simplify(&mut component.process, &mut names);

    let mut action_labels = spec.action_labels.clone();
    if component
        .process
        .action_summands
        .iter()
        .any(|summand| summand.multi_action.is_terminate())
    {
        action_labels.push(ActionLabel::terminate());
    }
    let mut global_variables: Vec<DataVariable> = spec.global_variables.clone();
    global_variables.extend(component.global_variables);

    Ok(LinearProcessSpecification::new(
        data_specification.clone(),
        action_labels,
        global_variables,
        component.process,
        component.initial_state,
        encoding.literal(1, Sort::Real),
    ))
}

/// The names that fresh process and variable names must not clash with: those of the processes,
/// actions, constructors, mappings and variables of `spec`.
fn used_names(spec: &ProcessSpecification) -> impl Iterator<Item = String> + '_ {
    let data = &spec.data_specification;
    let functions = data
        .constructors()
        .iter()
        .chain(data.mappings())
        .map(|function| function.name().to_string());
    let actions = spec.action_labels.iter().map(|label| label.name().value().to_string());
    let globals = spec.global_variables.iter().map(|variable| variable.name().to_string());
    let equations = spec.equations.iter().flat_map(|equation| {
        let identifier = &equation.identifier;
        std::iter::once(identifier.name().value().to_string())
            .chain(
                identifier
                    .parameters()
                    .iter()
                    .map(|variable| variable.name().to_string()),
            )
            .chain(variable_names(&equation.body))
    });
    let init = spec.init.iter().flat_map(variable_names);
    functions.chain(actions).chain(globals).chain(equations).chain(init)
}

/// The context of linearising and composing the leaves of the parallel layer.
struct Composer<'a, N> {
    table: &'a mut ProcessTable,
    termination: &'a mut Termination,
    names: &'a mut NameGenerator,
    normaliser: &'a mut N,
    representatives: &'a Representatives<'a>,
    options: &'a LinearisationOptions,
}

impl<N: KeyNormaliser> Composer<'_, N> {
    /// Linearises every leaf of `composition`. Recurses once per operator of the parallel layer.
    fn leaves(&mut self, composition: &Composition<Call>) -> Result<Composition<Component>, LinearisationError> {
        Ok(match composition {
            Composition::Leaf(root) => {
                let mut gnf = Gnf::new(self.table, self.termination, self.names);
                let processes = regular_form(&mut gnf, &root.process, self.normaliser, self.options.sequence_limit)?;
                Composition::Leaf(encode(
                    processes,
                    root.arguments.clone(),
                    self.names,
                    self.representatives,
                ))
            }
            Composition::Parallel(operands) => Composition::Parallel(
                operands
                    .iter()
                    .map(|operand| self.leaves(operand))
                    .collect::<Result<_, _>>()?,
            ),
            Composition::Operator(operator, operand) => {
                Composition::Operator(operator.clone(), Box::new(self.leaves(operand)?))
            }
        })
    }

    /// Composes the linear processes of `composition` bottom-up, keeping only the summands that
    /// `restriction` admits.
    ///
    /// The restrictions of enclosing operators are applied while summands are generated, as
    /// mCRL2's alphabet reduction does: exactly at the outermost merge of a parallel composition,
    /// as sub-multisets at the merges below it, and through `comm`, `hide` and `rename` by the
    /// alphabet of their operand. Recurses once per operator of the parallel layer.
    fn compose(&mut self, composition: Composition<Component>, restriction: Restriction<'_>) -> Component {
        match composition {
            Composition::Leaf(mut component) => {
                restrict(&mut component.process, &restriction);
                component
            }
            Composition::Parallel(operands) => {
                let mut operand_restriction = restriction.for_operands();
                let mut operands = operands.into_iter();
                let last = operands.next_back().expect("a parallel composition has operands");
                let mut result: Option<Component> = None;
                for operand in operands {
                    let component = self.compose(operand, operand_restriction.clone());
                    result = Some(match result {
                        None => component,
                        Some(lhs) => parallel(lhs, component, &mut operand_restriction, self.names),
                    });
                }
                let component = self.compose(last, operand_restriction);
                let mut restriction = restriction;
                match result {
                    None => component,
                    Some(lhs) => parallel(lhs, component, &mut restriction, self.names),
                }
            }
            Composition::Operator(operator, operand) => {
                let inner = match &operator {
                    Operator::Allow(allow) => Restriction::Allow(allow),
                    Operator::Block(block) => Restriction::Block(block),
                    _ if matches!(restriction, Restriction::None) => Restriction::None,
                    _ => match alphabet(&operand) {
                        Some(alphabet) => {
                            Restriction::Allowed(Rc::new(AllowSet::new(alphabet.into_iter().filter(|names| {
                                outcomes(&operator, names)
                                    .iter()
                                    .any(|outcome| restriction.admits(outcome))
                            }))))
                        }
                        None => match &operator {
                            Operator::Comm(communications) => restriction.through_comm(communications),
                            _ => Restriction::None,
                        },
                    },
                };
                let component = self.compose(*operand, inner);
                let mut stage = (restriction, SumElimination);
                apply(&operator, component, &mut stage, self.names)
            }
        }
    }
}
