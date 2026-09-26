use rand::Rng;
use rand::RngExt;
use rand::seq::IndexedRandom;

use merc_utilities::Span;

use crate::Bound;
use crate::Condition;
use crate::DataExpr;
use crate::Eq;
use crate::FixedPointOperator;
use crate::IdDecl;
use crate::PresEquation;
use crate::PresExpr;
use crate::PresExprBinaryOp;
use crate::PresExprKind;
use crate::PropVarDecl;
use crate::PropVarInst;
use crate::Sort;
use crate::SortDecl;
use crate::SortExpression;
use crate::SortExpressionKind;
use crate::UntypedPres;
use crate::random_data_specification;
use crate::random_integer_data_expression;
use crate::random_value_expression;

const PRED_INTS: &[&str] = &["m", "n"];
const QUANT_INTS: &[&str] = &["t", "u", "v", "w"];

/// Parameters held constant throughout the random PRES generation.
struct PresGenConfig<'a> {
    /// The predicate variables available for instantiation in leaves.
    predicate_vars: &'a [PredVar],

    /// Whether `inf`/`sup`/`sum` binders may be generated.
    use_bounds: bool,

    /// Probability that a leaf is a predicate variable instantiation rather than a `val(...)`
    /// atom.
    propvar_probability: f64,

    /// The declared sorts of a random data specification a predicate variable's rich (non-`Int`)
    /// parameters, if any, are drawn from -- empty for plain [`random_pres`].
    sort_decls: &'a [SortDecl],
}

/// Generates a random PRES, akin to [`crate::random_pbes`]. Unlike a PBES, every `PresExpr`
/// constructor is freely nestable (a PRES is uniformly real-valued, not split between a boolean
/// top level and embedded data), so unlike `random_pbes_expr` this generator carries no
/// polarity bias -- that bias exists only to keep PBES formulas monotone for their fixpoint
/// semantics, not a property type checking cares about.
///
/// `atom_count` and `propvar_count` together control the expression size: their sum determines
/// the recursion depth, and their ratio determines how often a leaf is a predicate variable
/// instantiation versus a `val(...)` atom.
pub fn random_pres<R: Rng>(
    rng: &mut R,
    equation_count: usize,
    atom_count: usize,
    propvar_count: usize,
    use_bounds: bool,
) -> UntypedPres {
    let pred_vars: Vec<PredVar> = (0..equation_count).map(|i| make_pred_var(rng, i, &[])).collect();
    random_pres_from_pred_vars(rng, pred_vars, atom_count, propvar_count, use_bounds, &[])
}

/// Generates a random PRES whose predicate variables additionally carry one parameter typed with
/// a sort from a freshly generated [`crate::random_data_specification`] (a struct, container, or
/// function sort), exercising the type checker on richer parameter sorts than plain
/// [`random_pres`] ever produces. `sort_count`/`max_sort_depth` control the generated data
/// specification, exactly as in `random_data_specification`.
#[allow(clippy::too_many_arguments)] // each parameter tunes an independent generator knob
pub fn random_pres_with_data_specification<R: Rng>(
    rng: &mut R,
    sort_count: usize,
    max_sort_depth: usize,
    equation_count: usize,
    atom_count: usize,
    propvar_count: usize,
    use_bounds: bool,
) -> UntypedPres {
    let data_specification = random_data_specification(rng, sort_count, max_sort_depth);
    let pred_vars: Vec<PredVar> = (0..equation_count)
        .map(|i| make_pred_var(rng, i, &data_specification.sort_declarations))
        .collect();
    let mut pres = random_pres_from_pred_vars(
        rng,
        pred_vars,
        atom_count,
        propvar_count,
        use_bounds,
        &data_specification.sort_declarations,
    );
    pres.data_specification = data_specification;
    pres
}

fn random_pres_from_pred_vars<R: Rng>(
    rng: &mut R,
    pred_vars: Vec<PredVar>,
    atom_count: usize,
    propvar_count: usize,
    use_bounds: bool,
    sort_decls: &[SortDecl],
) -> UntypedPres {
    let total = (atom_count + propvar_count).max(1);
    let propvar_prob = propvar_count as f64 / total as f64;
    let depth = total.ilog2() as usize + 1;

    let config = PresGenConfig {
        predicate_vars: &pred_vars,
        use_bounds,
        propvar_probability: propvar_prob,
        sort_decls,
    };

    let mut equations = Vec::new();
    for pv in &pred_vars {
        let freevars = pv.expr_freevars();
        let formula = random_pres_expr(rng, depth, &freevars, &config);
        let operator = if rng.random_bool(0.5) {
            FixedPointOperator::Least
        } else {
            FixedPointOperator::Greatest
        };
        equations.push(PresEquation {
            operator,
            variable: pv.to_decl(),
            formula,
            span: Span::default(),
        });
    }

    let first = &pred_vars[0];
    let init_args: Vec<DataExpr> = first
        .params
        .iter()
        .map(|(_, sort)| random_param_value(rng, sort, &[], sort_decls))
        .collect();
    let init = PropVarInst::new(first.name.clone(), init_args);

    UntypedPres {
        data_specification: Default::default(),
        global_variables: Vec::new(),
        equations,
        init,
    }
}

fn random_leaf<R: Rng>(rng: &mut R, freevars: &[IdDecl], config: &PresGenConfig) -> PresExpr {
    if !config.predicate_vars.is_empty() && rng.random_bool(config.propvar_probability) {
        let pv = config.predicate_vars.choose(rng).unwrap();
        let args = pv
            .params
            .iter()
            .map(|(_, sort)| random_param_value(rng, sort, freevars, config.sort_decls))
            .collect();
        PresExprKind::PropVarInst(PropVarInst::new(pv.name.clone(), args)).into()
    } else {
        PresExprKind::DataValExpr(random_integer_data_expression(rng, freevars)).into()
    }
}

/// Generates a value for a predicate-variable parameter of the given sort: the plain integer
/// generator for `Int` (every classic [`random_pres`] parameter), [`random_value_expression`] for
/// anything richer -- the same split [`crate::random_pbes`]'s own `random_param_value` uses to
/// avoid `random_value_expression`'s general-purpose recursion for the common case.
fn random_param_value<R: Rng>(
    rng: &mut R,
    sort: &SortExpression,
    freevars: &[IdDecl],
    sort_decls: &[SortDecl],
) -> DataExpr {
    match &sort.node {
        SortExpressionKind::Simple(Sort::Int) => random_integer_data_expression(rng, freevars),
        _ => random_value_expression(rng, sort_decls, sort, freevars, 2),
    }
}

/// Generates a random PRES expression with the given parameters. `depth` controls the maximum
/// depth of the generated expression; `config` controls how likely a leaf is to be a predicate
/// variable instantiation versus a `val(...)` atom, and whether `inf`/`sup`/`sum` binders may be
/// generated.
fn random_pres_expr<R: Rng>(rng: &mut R, depth: usize, freevars: &[IdDecl], config: &PresGenConfig) -> PresExpr {
    if depth == 0 {
        return random_leaf(rng, freevars, config);
    }

    // Binary/Equal/Condition are over-represented relative to the binders, to bias toward
    // non-trivial trees without making every generated formula deeply quantified.
    let op_table: &[u8] = if config.use_bounds {
        &[0, 1, 2, 3, 4, 5, 0, 1, 2, 3, 4, 5, 6, 7]
    } else {
        &[0, 1, 2, 3, 4, 5]
    };
    let op = *op_table.choose(rng).unwrap();

    match op {
        0 => PresExprKind::Negation(Box::new(random_pres_expr(rng, depth - 1, freevars, config))).into(),
        1 => random_binary(rng, PresExprBinaryOp::Conjunction, depth, freevars, config),
        2 => random_binary(rng, PresExprBinaryOp::Disjunction, depth, freevars, config),
        3 => random_binary(rng, PresExprBinaryOp::Implies, depth, freevars, config),
        4 => random_binary(rng, PresExprBinaryOp::Add, depth, freevars, config),
        5 => random_condition_or_equal(rng, depth, freevars, config),
        6 => random_bound(rng, Bound::Inf, depth - 1, freevars, config),
        7 => random_bound(rng, Bound::Sup, depth - 1, freevars, config),
        _ => unreachable!(),
    }
}

fn random_binary<R: Rng>(
    rng: &mut R,
    op: PresExprBinaryOp,
    depth: usize,
    freevars: &[IdDecl],
    config: &PresGenConfig,
) -> PresExpr {
    let lhs = Box::new(random_pres_expr(rng, depth - 1, freevars, config));
    let rhs = Box::new(random_pres_expr(rng, depth - 1, freevars, config));
    PresExprKind::Binary { op, lhs, rhs }.into()
}

/// Generates `eqinf(body)`/`eqninf(body)` or `condsm(lhs, then, else)`/`condeq(lhs, then, else)`,
/// each an equally plausible way to spend one level of depth.
fn random_condition_or_equal<R: Rng>(
    rng: &mut R,
    depth: usize,
    freevars: &[IdDecl],
    config: &PresGenConfig,
) -> PresExpr {
    if rng.random_bool(0.5) {
        let eq = if rng.random_bool(0.5) { Eq::EqInf } else { Eq::EqnInf };
        let body = Box::new(random_pres_expr(rng, depth - 1, freevars, config));
        PresExprKind::Equal { eq, body }.into()
    } else {
        let condition = if rng.random_bool(0.5) {
            Condition::Condsm
        } else {
            Condition::Condeq
        };
        let lhs = Box::new(random_pres_expr(rng, depth - 1, freevars, config));
        let then = Box::new(random_pres_expr(rng, depth - 1, freevars, config));
        let else_ = Box::new(random_pres_expr(rng, depth - 1, freevars, config));
        PresExprKind::Condition {
            condition,
            lhs,
            then,
            else_,
        }
        .into()
    }
}

/// Generates a random `inf`/`sup`/`sum` binder over a fresh `Int`-sorted variable. Unlike
/// `random_pbes`'s quantifiers, the bound variable is not value-bounded: nothing here evaluates
/// the generated formula, so an unbounded domain is still well-typed and the generator itself
/// already terminates via `depth`.
fn random_bound<R: Rng>(rng: &mut R, op: Bound, depth: usize, freevars: &[IdDecl], config: &PresGenConfig) -> PresExpr {
    let available: Vec<&str> = QUANT_INTS
        .iter()
        .filter(|&&q| !freevars.iter().any(|fv| fv.identifier.node == q))
        .copied()
        .collect();

    if available.is_empty() {
        return random_leaf(rng, freevars, config);
    }

    let var_name = (*available.choose(rng).expect("available is non-empty")).to_string();
    let var_decl = IdDecl::new(
        var_name.clone(),
        SortExpressionKind::Simple(Sort::Int).into(),
        Span::default(),
    );

    let mut new_freevars = freevars.to_vec();
    new_freevars.push(as_expr_decl(&var_name));

    let body = random_pres_expr(rng, depth, &new_freevars, config);

    PresExprKind::Bound {
        op,
        variables: vec![var_decl],
        expr: Box::new(body),
    }
    .into()
}

fn as_expr_decl(name: &str) -> IdDecl {
    IdDecl::new(
        name.to_string(),
        SortExpressionKind::Simple(Sort::Int).into(),
        Span::default(),
    )
}

struct PredVar {
    name: String,
    params: Vec<(String, SortExpression)>,
}

impl PredVar {
    fn to_decl(&self) -> PropVarDecl {
        let params = self
            .params
            .iter()
            .map(|(name, sort)| IdDecl::new(name.clone(), sort.clone(), Span::default()))
            .collect();
        PropVarDecl::new(self.name.clone(), params)
    }

    fn expr_freevars(&self) -> Vec<IdDecl> {
        self.params
            .iter()
            .map(|(name, sort)| IdDecl::new(name.clone(), sort.clone(), Span::default()))
            .collect()
    }
}

/// Generates a predicate variable's parameter list: 0-2 classic `Int` parameters (as
/// [`random_pres`] always has), plus, when `sort_decls` is non-empty, one additional parameter
/// typed with a random declared sort -- exercising the type checker on struct/container/function
/// parameter sorts that plain [`random_pres`] never generates.
fn make_pred_var<R: Rng>(rng: &mut R, index: usize, sort_decls: &[SortDecl]) -> PredVar {
    let size = rng.random_range(0..=2usize);
    let mut pool: Vec<&str> = PRED_INTS.to_vec();
    let mut params = Vec::new();
    for _ in 0..size {
        if pool.is_empty() {
            break;
        }
        let idx = rng.random_range(0..pool.len());
        let name = pool.remove(idx).to_string();
        params.push((name, SortExpressionKind::Simple(Sort::Int).into()));
    }

    if let Some(decl) = sort_decls.choose(rng) {
        let sort_ref = SortExpressionKind::Reference(decl.identifier.clone()).into();
        params.push((format!("rich{index}"), sort_ref));
    }

    PredVar {
        name: format!("X{index}"),
        params,
    }
}
