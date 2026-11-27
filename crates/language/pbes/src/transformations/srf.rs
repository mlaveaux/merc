use std::collections::HashMap;
use std::collections::VecDeque;

use merc_data::DataVariable;
use merc_utilities::FreshNameGenerator;
use merc_utilities::MercError;
use merc_utilities::optimized_and;
use merc_utilities::optimized_conjunction;
use merc_utilities::optimized_disjunction;
use merc_utilities::optimized_not;
use merc_utilities::optimized_or;

use crate::FixpointSymbol;
use crate::Pbes;
use crate::PbesEquation;
use crate::PbesExpression;
use crate::PbesExpressionRef;
use crate::PbesFold;
use crate::PbesKind;
use crate::PropositionalVariable;
use crate::PropositionalVariableInstantiation;
use crate::SrfEquation;
use crate::SrfPbes;
use crate::SrfSummand;
use crate::distribute_not_over_and;
use crate::fold_pbes_expr;
use crate::is_simple_expression;
use crate::split_and;
use crate::split_or;
use crate::transformations::simple_expression::SimpleExpressionCache;

/// Converts a PBES into standard recursive form.
///
/// `pbes` must already be normalised (no `!`/`=>` left at the PBES level), and must be well
/// typed: a [crate::PbesWellTypedError] is returned otherwise.
pub fn pbes_to_srf(pbes: &Pbes, merge_simple_expressions: bool) -> Result<SrfPbes, MercError> {
    let srf = pbes_to_pre_srf_impl(pbes, false, merge_simple_expressions)?;
    Ok(simplify_srf_pbes(&srf))
}

/// Converts a PBES into *pre*-SRF: like [pbes_to_srf], but counter-example propositional variables
/// may remain as simple expressions in summand conditions instead of forcing a new equation.
///
/// `pbes` must already be normalised (no `!`/`=>` left at the PBES level), and must be well
/// typed: a [crate::PbesWellTypedError] is returned otherwise.
pub fn pbes_to_pre_srf(pbes: &Pbes, merge_simple_expressions: bool) -> Result<SrfPbes, MercError> {
    pbes_to_pre_srf_impl(pbes, true, merge_simple_expressions)
}

/// Simplifies an SRF PBES by removing summands whose condition simplifies to `true` or `false`: a
/// vacuous conjunct or dead disjunct is dropped, and an unconditionally true/false summand
/// collapses the whole equation to a single summand recursing into `X_true`/`X_false`, which
/// avoids spurious dependencies on the other, now irrelevant, summands.
pub fn simplify_srf_pbes(pbes: &SrfPbes) -> SrfPbes {
    let x_false = pbes.false_equation.summands[0].variable.clone();
    let x_true = pbes.true_equation.summands[0].variable.clone();

    // Shared by all guards, which typically have many conditions in common.
    let mut simplified = HashMap::new();
    let mut simplify_equation = |equation: &SrfEquation| {
        let conjunctive = equation.conjunctive;
        let mut summands = Vec::new();
        let mut equation_is_trivial = false;

        for summand in &equation.summands {
            // For a conjunctive equation, to_pbes_expression() builds
            // `distribute_not_over_and(condition) || variable`; mirror that here to tell a
            // vacuously true or unsatisfiable guard apart from a real one.
            let guard = if conjunctive {
                simplify_boolean(
                    &distribute_not_over_and(&summand.condition.copy()).copy(),
                    &mut simplified,
                )
            } else {
                summand.condition.clone()
            };
            let is_true = matches!(guard.kind(), PbesKind::True);
            let is_false = matches!(guard.kind(), PbesKind::False);
            let targets_false = summand.parameters.is_empty() && summand.variable.name() == x_false.name();
            let targets_true = summand.parameters.is_empty() && summand.variable.name() == x_true.name();

            if conjunctive {
                if is_true {
                    continue; // forall params . true || variable always holds.
                }
                if is_false && targets_false {
                    equation_is_trivial = true;
                    break; // forall <> . false || X_false == X_false.
                }
            } else {
                if is_false {
                    continue; // exists params . false && variable never holds.
                }
                if is_true && targets_true {
                    equation_is_trivial = true;
                    break; // exists <> . true && X_true == X_true.
                }
            }

            summands.push(summand.clone());
        }

        if equation_is_trivial {
            let target = if conjunctive { x_false.clone() } else { x_true.clone() };
            summands = vec![SrfSummand::new(Vec::new(), PbesExpression::true_constant(), target)];
        } else if summands.is_empty() {
            let target = if conjunctive { x_true.clone() } else { x_false.clone() };
            summands = vec![SrfSummand::new(Vec::new(), PbesExpression::true_constant(), target)];
        }

        SrfEquation::new(equation.symbol, equation.variable.clone(), summands, conjunctive)
    };

    SrfPbes::new(
        pbes.data_specification.clone(),
        pbes.global_variables.clone(),
        pbes.equations.iter().map(&mut simplify_equation).collect(),
        simplify_equation(&pbes.false_equation),
        simplify_equation(&pbes.true_equation),
        pbes.initial_state.clone(),
    )
}

/// Returns true iff `pbes` is already in standard recursive form, i.e. converting it does not
/// introduce any equation beyond the two generated `X_false`/`X_true` ones.
pub fn is_srf(pbes: &Pbes) -> Result<bool, MercError> {
    let srf = pbes_to_srf(pbes, true)?;
    Ok(srf.equations.len() == pbes.equations.len())
}

/// Determines whether a normalised PBES equation's right-hand side `phi` should become a
/// conjunctive (`forall`-based) or disjunctive (`exists`-based) SRF equation.
pub fn is_conjunctive(phi: &PbesExpressionRef<'_>, allow_ce: bool) -> Result<bool, MercError> {
    is_conjunctive_with(phi, &mut |expr| is_simple_expression(expr, allow_ce))
}

fn pbes_to_pre_srf_impl(pbes: &Pbes, allow_ce: bool, merge_simple_expressions: bool) -> Result<SrfPbes, MercError> {
    pbes.check_well_typed()?;

    // Fresh propositional variable names, distinct from every equation of the input.
    let mut id_generator = FreshNameGenerator::new(
        pbes.equations
            .iter()
            .map(|equation| equation.variable.name().value().to_string()),
    );
    let x_false = id_generator.generate("X_false");
    let x_true = id_generator.generate("X_true");

    // mu X_false = false || X_false; nu X_true = X_true: the escape equations every other
    // equation can route to.
    let false_variable = PropositionalVariable::new(&x_false, &[]);
    let true_variable = PropositionalVariable::new(&x_true, &[]);
    let false_formula = PbesExpression::or(
        &PbesExpression::false_constant(),
        &PropositionalVariableInstantiation::identity(&false_variable).into(),
    );
    let true_formula = PropositionalVariableInstantiation::identity(&true_variable).into();
    let eqn_false = PbesEquation::new(FixpointSymbol::Mu, false_variable, false_formula);
    let eqn_true = PbesEquation::new(FixpointSymbol::Nu, true_variable, true_formula);

    let mut builder = SrfBuilder {
        pending: pbes.equations.iter().cloned().collect(),
        id_generator,
        x_true,
        x_false,
        merge_simple_expressions,
        allow_ce,
        simple: SimpleExpressionCache::default(),
    };

    let mut equations = Vec::new();
    while let Some(eqn) = builder.pending.pop_front() {
        equations.push(builder.build_equation(&eqn)?);
    }

    // Both escape equations are already in SRF, so they spawn nothing.
    let false_equation = builder.build_equation(&eqn_false)?;
    let true_equation = builder.build_equation(&eqn_true)?;
    debug_assert!(builder.pending.is_empty());

    Ok(SrfPbes::new(
        pbes.data_specification.clone(),
        pbes.global_variables.clone(),
        equations,
        false_equation,
        true_equation,
        pbes.initial_state.clone(),
    ))
}

/// Builds the SRF equations and summands for one input PBES.
struct SrfBuilder {
    /// Equations still to be processed: equations spawned while processing one equation are
    /// inserted at the front, so they are handled immediately after it.
    pending: VecDeque<PbesEquation>,
    id_generator: FreshNameGenerator,
    x_true: String,
    x_false: String,
    merge_simple_expressions: bool,
    allow_ce: bool,
    simple: SimpleExpressionCache,
}

impl SrfBuilder {
    fn escape_target(&self, conjunctive: bool) -> PropositionalVariableInstantiation {
        let name = if conjunctive { &self.x_false } else { &self.x_true };
        PropositionalVariableInstantiation::new(name, &[])
    }

    /// Defers `phi` (the raw, unprocessed expression) to a freshly named equation with
    /// `variables` as its parameters, and returns the summand that invokes it.
    fn spawn_equation(
        &mut self,
        eqn: &PbesEquation,
        variables: &[DataVariable],
        phi: &PbesExpressionRef<'_>,
    ) -> SrfSummand {
        let name = self.id_generator.generate(eqn.variable.name().value());
        let variable = PropositionalVariable::new(&name, variables);
        let instantiation = PropositionalVariableInstantiation::identity(&variable);
        self.pending
            .push_front(PbesEquation::new(eqn.symbol, variable, phi.protect()));

        SrfSummand::new(Vec::new(), PbesExpression::true_constant(), instantiation)
    }

    /// Turns `eqn` into an SRF equation; equations it spawns are queued in `pending`.
    fn build_equation(&mut self, eqn: &PbesEquation) -> Result<SrfEquation, MercError> {
        let formula = eqn.formula.copy();
        let allow_ce = self.allow_ce;
        let conjunctive = is_conjunctive_with(&formula, &mut |expr| self.simple.is_simple(expr, allow_ce))?;
        let variables = eqn.variable.parameters().to_vec();
        let summands = self.collect_summands(conjunctive, &formula, eqn, &variables)?;
        Ok(SrfEquation::new(
            eqn.symbol,
            eqn.variable.clone(),
            summands,
            conjunctive,
        ))
    }

    /// Collects the summands for `phi`, the (sub)formula of equation `eqn` currently being turned
    /// into SRF; `conjunctive` is the shape of `eqn`, and `variables` are the parameters in scope
    /// (those of the equation plus every quantifier bound on the way down).
    fn collect_summands(
        &mut self,
        conjunctive: bool,
        phi: &PbesExpressionRef<'_>,
        eqn: &PbesEquation,
        variables: &[DataVariable],
    ) -> Result<Vec<SrfSummand>, MercError> {
        // Every node is tried against the generic "free of propositional variables" case before
        // any connective or quantifier specific handler, so a quantifier with a simple body is
        // caught here.
        if self.simple.is_simple(phi, self.allow_ce) {
            return Ok(vec![SrfSummand::new(
                Vec::new(),
                leaf_condition(conjunctive, phi),
                self.escape_target(conjunctive),
            )]);
        }

        match phi.kind() {
            PbesKind::Instantiation(inst) => Ok(vec![SrfSummand::new(
                Vec::new(),
                PbesExpression::true_constant(),
                inst.protect(),
            )]),
            PbesKind::And(..) if conjunctive => self.collect_own_connective(conjunctive, phi, eqn, variables),
            PbesKind::Or(..) if !conjunctive => self.collect_own_connective(conjunctive, phi, eqn, variables),
            PbesKind::And(lhs, rhs) | PbesKind::Or(lhs, rhs) => {
                self.collect_other_connective(conjunctive, phi, &lhs, &rhs, eqn, variables)
            }
            PbesKind::Forall(bound, body) if conjunctive => {
                self.collect_quantifier(conjunctive, &bound.to_vec(), &body, eqn, variables)
            }
            PbesKind::Exists(bound, body) if !conjunctive => {
                self.collect_quantifier(conjunctive, &bound.to_vec(), &body, eqn, variables)
            }
            PbesKind::Forall(..) | PbesKind::Exists(..) => Ok(vec![self.spawn_equation(eqn, variables, phi)]),
            _ => Err(format!("collect_summands: PBES is not normalised, unexpected expression {phi}").into()),
        }
    }

    /// The quantifier matches the equation's shape, so its variables become summand parameters.
    ///
    /// The summands of `body` end up under `bound`, next to conditions that come from outside the
    /// quantifier (see [strengthen]). This cannot capture any variable, because a well-typed PBES
    /// has no quantifier that shadows a parameter, a global variable, or an enclosing quantifier;
    /// see [Pbes::check_well_typed].
    fn collect_quantifier(
        &mut self,
        conjunctive: bool,
        bound: &[DataVariable],
        body: &PbesExpressionRef<'_>,
        eqn: &PbesEquation,
        variables: &[DataVariable],
    ) -> Result<Vec<SrfSummand>, MercError> {
        let mut extended = variables.to_vec();
        extended.extend(bound.iter().cloned());
        let mut summands = self.collect_summands(conjunctive, body, eqn, &extended)?;
        for summand in &mut summands {
            summand.add_variables(bound);
        }
        Ok(summands)
    }

    /// `phi`'s top-level connective matches `conjunctive` (an `&&` chain in a conjunctive
    /// equation, or an `||` chain in a disjunctive one): flattens the whole chain and recurses into
    /// every clause, optionally pulling out clauses free of propositional variables to strengthen
    /// the summands coming from the rest.
    fn collect_own_connective(
        &mut self,
        conjunctive: bool,
        phi: &PbesExpressionRef<'_>,
        eqn: &PbesEquation,
        variables: &[DataVariable],
    ) -> Result<Vec<SrfSummand>, MercError> {
        let clauses = split_own(conjunctive, phi);
        let (simple, complex): (Vec<_>, Vec<_>) = if self.merge_simple_expressions {
            // Always checked without counter-example variables, regardless of `allow_ce`: the
            // strengthening condition built from `simple` is spliced into other summands'
            // conditions as-is, so it must not depend on any propositional variable.
            clauses
                .into_iter()
                .partition(|clause| self.simple.is_simple(clause, false))
        } else {
            (Vec::new(), clauses)
        };

        let mut summands = Vec::new();
        for clause in &simple {
            summands.extend(self.collect_summands(conjunctive, clause, eqn, variables)?);
        }

        if simple.is_empty() {
            for clause in &complex {
                summands.extend(self.collect_summands(conjunctive, clause, eqn, variables)?);
            }
            return Ok(summands);
        }

        let combined = combine_simple(conjunctive, simple.iter().map(|clause| clause.protect()));
        let condition = strengthen(conjunctive, &combined.copy());
        for clause in &complex {
            let start = summands.len();
            summands.extend(self.collect_summands(conjunctive, clause, eqn, variables)?);
            for summand in &mut summands[start..] {
                summand.add_condition(&condition);
            }
        }
        Ok(summands)
    }

    /// `phi`'s top-level connective is the *other* one (an `||` found while building a conjunctive
    /// equation, or an `&&` while building a disjunctive one). Unlike
    /// [Self::collect_own_connective] this only looks at the two immediate operands: if exactly
    /// one of them is free of propositional variables, it strengthens the summands coming from the
    /// other side; otherwise the whole `phi` is deferred to a freshly spawned equation, which
    /// flattens it once reprocessed as its own top-level connective.
    fn collect_other_connective(
        &mut self,
        conjunctive: bool,
        phi: &PbesExpressionRef<'_>,
        lhs: &PbesExpressionRef<'_>,
        rhs: &PbesExpressionRef<'_>,
        eqn: &PbesEquation,
        variables: &[DataVariable],
    ) -> Result<Vec<SrfSummand>, MercError> {
        // A simple side next to an instantiation is always pulled out, even without merging: that
        // is the shape by which [is_conjunctive] classified the equation, so spawning `phi` instead
        // would produce an equation identical to this one, and so on forever.
        let is_instantiation = |expr: &PbesExpressionRef<'_>| matches!(expr.kind(), PbesKind::Instantiation(_));
        let one_sided = if self.simple.is_simple(lhs, self.allow_ce)
            && (self.merge_simple_expressions || is_instantiation(rhs))
        {
            Some((lhs, rhs))
        } else if self.simple.is_simple(rhs, self.allow_ce) && (self.merge_simple_expressions || is_instantiation(lhs))
        {
            Some((rhs, lhs))
        } else {
            None
        };

        if let Some((simple_side, complex_side)) = one_sided {
            let mut summands = self.collect_summands(conjunctive, complex_side, eqn, variables)?;
            // The other connective is the own connective of the opposite shape, so the condition
            // has the polarity that shape would use.
            let condition = strengthen(!conjunctive, simple_side);
            for summand in &mut summands {
                summand.add_condition(&condition);
            }
            return Ok(summands);
        }

        Ok(vec![self.spawn_equation(eqn, variables, phi)])
    }
}

/// See [is_conjunctive]; `is_simple` decides whether an expression is simple, so that a caller can
/// answer from a cache.
fn is_conjunctive_with<F>(phi: &PbesExpressionRef<'_>, is_simple: &mut F) -> Result<bool, MercError>
where
    F: FnMut(&PbesExpressionRef<'_>) -> bool,
{
    if is_simple(phi) {
        return Ok(false);
    }

    match phi.kind() {
        PbesKind::Instantiation(_) => Ok(false),
        PbesKind::Or(lhs, rhs) => Ok(one_side_is_simple_instantiation(&lhs, &rhs, is_simple)),
        PbesKind::And(lhs, rhs) => Ok(!one_side_is_simple_instantiation(&lhs, &rhs, is_simple)),
        PbesKind::Exists(..) => Ok(false),
        PbesKind::Forall(..) => Ok(true),
        _ => Err(format!("is_conjunctive: unexpected case {phi}").into()),
    }
}

fn one_side_is_simple_instantiation<F>(
    lhs: &PbesExpressionRef<'_>,
    rhs: &PbesExpressionRef<'_>,
    is_simple: &mut F,
) -> bool
where
    F: FnMut(&PbesExpressionRef<'_>) -> bool,
{
    let is_instantiation = |expr: &PbesExpressionRef<'_>| matches!(expr.kind(), PbesKind::Instantiation(_));
    (is_simple(lhs) && is_instantiation(rhs)) || (is_simple(rhs) && is_instantiation(lhs))
}

fn split_own<'a>(conjunctive: bool, expr: &PbesExpressionRef<'a>) -> Vec<PbesExpressionRef<'a>> {
    if conjunctive { split_and(expr) } else { split_or(expr) }
}

/// The condition of the summand produced for a leaf (non-recursive) expression: negated for a
/// conjunctive equation, since its summands are joined by `||` rather than `&&`.
fn leaf_condition(conjunctive: bool, expr: &PbesExpressionRef<'_>) -> PbesExpression {
    if conjunctive {
        optimized_not(expr.protect())
    } else {
        expr.protect()
    }
}

/// The extra conjunct added to a summand's condition to account for a sibling simple clause
/// pulled out of the same `&&`/`||` chain: negated for a disjunctive equation, dually to
/// [leaf_condition].
fn strengthen(conjunctive: bool, condition: &PbesExpressionRef<'_>) -> PbesExpression {
    if conjunctive {
        condition.protect()
    } else {
        optimized_not(condition.protect())
    }
}

/// Merges several simple clauses of a chain into the one condition passed to [strengthen].
fn combine_simple<I>(conjunctive: bool, clauses: I) -> PbesExpression
where
    I: IntoIterator<Item = PbesExpression>,
{
    if conjunctive {
        optimized_conjunction(clauses)
    } else {
        optimized_disjunction(clauses)
    }
}

/// A minimal bottom-up `true`/`false` absorption over `!`, `&&` and `||` (see [optimized_and]),
/// scoped to what [simplify_srf_pbes] needs to decide whether a guard is a literal `true`/`false`;
/// not a general PBES rewriter. The other connectives and quantifiers are rebuilt around their
/// simplified operands, which never makes them a literal.
fn simplify_boolean(
    expr: &PbesExpressionRef<'_>,
    cache: &mut HashMap<PbesExpression, PbesExpression>,
) -> PbesExpression {
    fold_pbes_expr(expr, cache, |expr, node| match node {
        PbesFold::Not(inner) => optimized_not(inner),
        PbesFold::And(lhs, rhs) => optimized_and(lhs, rhs),
        PbesFold::Or(lhs, rhs) => optimized_or(lhs, rhs),
        PbesFold::Imp(lhs, rhs) => PbesExpression::imp(&lhs, &rhs),
        PbesFold::Forall(variables, body) => PbesExpression::forall(&variables.to_vec(), &body),
        PbesFold::Exists(variables, body) => PbesExpression::exists(&variables.to_vec(), &body),
        PbesFold::True | PbesFold::False | PbesFold::Data(_) | PbesFold::Instantiation(_) => expr.protect(),
    })
}

#[cfg(test)]
mod tests {
    //! [simplify_srf_pbes] on SRF PBESs with given constant summand conditions; building them
    //! directly needs the crate-private constructors.

    use merc_data::DataExpression;
    use merc_data::DataVariable;
    use merc_data::Mcrl2DataSpecification;
    use merc_data::SortExpression;

    use crate::FixpointSymbol;
    use crate::PbesExpression;
    use crate::PropositionalVariable;
    use crate::PropositionalVariableInstantiation;
    use crate::SrfEquation;
    use crate::SrfPbes;
    use crate::SrfSummand;
    use crate::simplify_srf_pbes;

    /// The targets of the summands of `X` after [simplify_srf_pbes], where `X` is the equation with
    /// the given summands, each to an equation without parameters given by name. The PBES also has
    /// the escape equations, each with its single self-referencing summand.
    fn simplified_targets(
        conjunctive: bool,
        symbol: FixpointSymbol,
        summands: &[(PbesExpression, &str)],
    ) -> Vec<String> {
        let summand = |condition: &PbesExpression, target: &str| {
            SrfSummand::new(
                Vec::new(),
                condition.clone(),
                PropositionalVariableInstantiation::new(target, &[]),
            )
        };
        let escape = |symbol, name: &str| {
            SrfEquation::new(
                symbol,
                PropositionalVariable::new(name, &[]),
                vec![summand(&PbesExpression::true_constant(), name)],
                false,
            )
        };
        let x = SrfEquation::new(
            symbol,
            PropositionalVariable::new("X", &[]),
            summands
                .iter()
                .map(|(condition, target)| summand(condition, target))
                .collect(),
            conjunctive,
        );
        let srf = SrfPbes::new(
            Mcrl2DataSpecification::default(),
            Vec::new(),
            vec![x],
            escape(FixpointSymbol::Mu, "X_false"),
            escape(FixpointSymbol::Nu, "X_true"),
            PropositionalVariableInstantiation::new("X_true", &[]),
        );

        let result = simplify_srf_pbes(&srf);
        let x = result.equations().next().expect("X is the first equation");
        assert_eq!(x.variable().name().value(), "X");
        x.summands()
            .iter()
            .map(|summand| summand.variable().name().value().to_string())
            .collect()
    }

    #[test]
    fn test_simplify_srf_pbes_drops_dead_disjunct() {
        // exists <> . false && Y() never holds and is dropped; exists <> . true && Z() survives.
        let summands = [
            (PbesExpression::false_constant(), "Y"),
            (PbesExpression::true_constant(), "Z"),
        ];
        assert_eq!(simplified_targets(false, FixpointSymbol::Mu, &summands), ["Z"]);
    }

    #[test]
    fn test_simplify_srf_pbes_drops_vacuous_conjunct() {
        // The guard of a conjunctive summand is negated, so a stored condition `false` makes
        // `true || Y()` vacuously true and the conjunct is dropped; `false || Z()` survives.
        let summands = [
            (PbesExpression::false_constant(), "Y"),
            (PbesExpression::true_constant(), "Z"),
        ];
        assert_eq!(simplified_targets(true, FixpointSymbol::Mu, &summands), ["Z"]);
    }

    #[test]
    fn test_simplify_srf_pbes_disjunctive_equation_collapses_to_true() {
        // An unconditional summand to X_true makes the whole disjunctive equation true.
        let summands = [
            (PbesExpression::true_constant(), "Y"),
            (PbesExpression::true_constant(), "X_true"),
        ];
        assert_eq!(simplified_targets(false, FixpointSymbol::Mu, &summands), ["X_true"]);
    }

    #[test]
    fn test_simplify_srf_pbes_conjunctive_equation_collapses_to_false() {
        // Dually, a summand whose negated guard is `false`, to X_false, makes the whole
        // conjunctive equation false.
        let summands = [
            (PbesExpression::true_constant(), "Y"),
            (PbesExpression::true_constant(), "X_false"),
        ];
        assert_eq!(simplified_targets(true, FixpointSymbol::Nu, &summands), ["X_false"]);
    }

    #[test]
    fn test_simplify_srf_pbes_all_disjuncts_dead_becomes_false() {
        // An empty disjunction is false, represented by a single summand to X_false.
        let summands = [
            (PbesExpression::false_constant(), "Y"),
            (PbesExpression::false_constant(), "Z"),
        ];
        assert_eq!(simplified_targets(false, FixpointSymbol::Mu, &summands), ["X_false"]);
    }

    #[test]
    fn test_simplify_srf_pbes_all_conjuncts_vacuous_becomes_true() {
        // Dually, an empty conjunction is true, represented by a single summand to X_true.
        let summands = [
            (PbesExpression::false_constant(), "Y"),
            (PbesExpression::false_constant(), "Z"),
        ];
        assert_eq!(simplified_targets(true, FixpointSymbol::Nu, &summands), ["X_true"]);
    }

    #[test]
    fn test_simplify_srf_pbes_leaves_ordinary_equations_unchanged() {
        let b1: PbesExpression =
            DataExpression::from(DataVariable::with_sort("b1", SortExpression::bool().copy())).into();
        let b2: PbesExpression =
            DataExpression::from(DataVariable::with_sort("b2", SortExpression::bool().copy())).into();
        let summands = [(b1, "Y"), (b2, "Z")];
        assert_eq!(simplified_targets(true, FixpointSymbol::Nu, &summands), ["Y", "Z"]);
    }
}
