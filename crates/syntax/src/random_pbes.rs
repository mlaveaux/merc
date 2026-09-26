use rand::Rng;
use rand::RngExt;
use rand::seq::IndexedRandom;

use merc_utilities::Span;

use crate::DataExpr;
use crate::DataExprBinaryOp;
use crate::DataExprKind;
use crate::FixedPointOperator;
use crate::IdDecl;
use crate::PbesEquation;
use crate::PbesExpr;
use crate::PbesExprBinaryOp;
use crate::PbesExprKind;
use crate::PropVarDecl;
use crate::PropVarInst;
use crate::Quantifier;
use crate::Sort;
use crate::SortDecl;
use crate::SortExpression;
use crate::SortExpressionKind;
use crate::UntypedPbes;
use crate::random_boolean_data_expression;
use crate::random_data_specification;
use crate::random_integer_data_expression;
use crate::random_value_expression;

const PRED_INTS: &[&str] = &["m", "n"];
const PRED_BOOLS: &[&str] = &["b", "c"];
const QUANT_INTS: &[&str] = &["t", "u", "v", "w"];

/// Parameters held constant throughout the random PBES generation.
struct PbesGenConfig<'a> {
    /// The predicate variables available for instantiation in leaves.
    predicate_vars: &'a [PredVar],

    /// Whether quantifiers may be generated.
    use_quantifiers: bool,

    /// Probability that a leaf is a predicate variable instantiation rather than
    /// a `val(...)` atom.
    propvar_probability: f64,

    /// The declared sorts of a random data specification a predicate variable's rich (non-Bool,
    /// non-integer) parameters, if any, are drawn from -- empty for plain [`random_pbes`].
    sort_decls: &'a [SortDecl],
}

/// Generates a random PBES.
///
/// `atom_count` and `propvar_count` together control the expression size: their sum determines the
/// recursion depth, and their ratio determines how often a leaf is a predicate variable instantiation
/// versus a `val(...)` atom.
pub fn random_pbes<R: Rng>(
    rng: &mut R,
    equation_count: usize,
    atom_count: usize,
    propvar_count: usize,
    use_quantifiers: bool,
    use_integers: bool,
) -> UntypedPbes {
    let pred_vars: Vec<PredVar> = (0..equation_count)
        .map(|i| make_pred_var(rng, i, use_integers, &[]))
        .collect();
    random_pbes_from_pred_vars(rng, pred_vars, atom_count, propvar_count, use_quantifiers, &[])
}

/// Generates a random PBES whose predicate variables additionally carry one parameter typed with
/// a sort from a freshly generated [`crate::random_data_specification`] (a struct, container, or
/// function sort), exercising the type checker on richer parameter sorts than plain [`random_pbes`]
/// ever produces. `sort_count`/`max_sort_depth` control the generated data specification, exactly
/// as in `random_data_specification`.
#[allow(clippy::too_many_arguments)] // each parameter tunes an independent generator knob
pub fn random_pbes_with_data_specification<R: Rng>(
    rng: &mut R,
    sort_count: usize,
    max_sort_depth: usize,
    equation_count: usize,
    atom_count: usize,
    propvar_count: usize,
    use_quantifiers: bool,
    use_integers: bool,
) -> UntypedPbes {
    let data_specification = random_data_specification(rng, sort_count, max_sort_depth);
    let pred_vars: Vec<PredVar> = (0..equation_count)
        .map(|i| make_pred_var(rng, i, use_integers, &data_specification.sort_declarations))
        .collect();
    let mut pbes = random_pbes_from_pred_vars(
        rng,
        pred_vars,
        atom_count,
        propvar_count,
        use_quantifiers,
        &data_specification.sort_declarations,
    );
    pbes.data_specification = data_specification;
    pbes
}

fn random_pbes_from_pred_vars<R: Rng>(
    rng: &mut R,
    pred_vars: Vec<PredVar>,
    atom_count: usize,
    propvar_count: usize,
    use_quantifiers: bool,
    sort_decls: &[SortDecl],
) -> UntypedPbes {
    let total = (atom_count + propvar_count).max(1);
    let propvar_prob = propvar_count as f64 / total as f64;
    let depth = total.ilog2() as usize + 1;

    let config = PbesGenConfig {
        predicate_vars: &pred_vars,
        use_quantifiers,
        propvar_probability: propvar_prob,
        sort_decls,
    };

    let mut equations = Vec::new();
    for pv in &pred_vars {
        let freevars = pv.expr_freevars();
        let formula = random_pbes_expr(rng, depth, &freevars, &config, false);
        let operator = if rng.random_bool(0.5) {
            FixedPointOperator::Least
        } else {
            FixedPointOperator::Greatest
        };
        equations.push(PbesEquation::new(operator, pv.to_decl(), formula));
    }

    let first = &pred_vars[0];
    let init_args: Vec<DataExpr> = first
        .params
        .iter()
        .map(|(_, sort)| random_value_expression(rng, sort_decls, sort, &[], 2))
        .collect();
    let init = PropVarInst::new(first.name.clone(), init_args);

    UntypedPbes {
        data_specification: Default::default(),
        global_variables: Vec::new(),
        equations,
        init,
    }
}

fn random_leaf<R: Rng>(rng: &mut R, freevars: &[IdDecl], config: &PbesGenConfig, negated: bool) -> PbesExpr {
    if !config.predicate_vars.is_empty() && rng.random_bool(config.propvar_probability) {
        let pv = config.predicate_vars.choose(rng).unwrap();
        let args = pv
            .params
            .iter()
            .map(|(_, sort)| random_param_value(rng, sort, freevars, config.sort_decls))
            .collect();
        let inst = PropVarInst::new(pv.name.clone(), args);
        if negated {
            PbesExprKind::Negation(Box::new(PbesExprKind::PropVarInst(inst).into())).into()
        } else {
            PbesExprKind::PropVarInst(inst).into()
        }
    } else {
        PbesExprKind::DataValExpr(random_boolean_data_expression(rng, freevars)).into()
    }
}

/// Generates a value for a predicate-variable parameter of the given sort: the existing
/// Bool/integer generators (which pick among comparisons/arithmetic over already-typed free
/// variables) for those sorts, [`random_value_expression`] for anything richer.
fn random_param_value<R: Rng>(
    rng: &mut R,
    sort: &SortExpression,
    freevars: &[IdDecl],
    sort_decls: &[SortDecl],
) -> DataExpr {
    match &sort.node {
        SortExpressionKind::Simple(Sort::Bool) => random_boolean_data_expression(rng, freevars),
        SortExpressionKind::Simple(Sort::Pos | Sort::Nat | Sort::Int) => random_integer_data_expression(rng, freevars),
        _ => random_value_expression(rng, sort_decls, sort, freevars, 2),
    }
}

/// Generates a random PBES expression with the given parameters.
///
/// `depth` controls the maximum depth of the generated expression. `config` controls how likely a
/// leaf is to be a predicate variable instantiation versus a `val(...)` atom, and whether
/// quantifiers may be generated. If `negated` is true, the top-level polarity is negative, which
/// biases the generator to produce more negations and implications, which flip polarity.
fn random_pbes_expr<R: Rng>(
    rng: &mut R,
    depth: usize,
    freevars: &[IdDecl],
    config: &PbesGenConfig,
    negated: bool,
) -> PbesExpr {
    if depth == 0 {
        return random_leaf(rng, freevars, config, negated);
    }

    // Binary operators are over-represented to bias toward non-trivial trees.
    let op_table: &[u8] = if config.use_quantifiers {
        &[0, 1, 2, 3, 0, 1, 2, 3, 4, 5]
    } else {
        &[0, 1, 2, 3]
    };
    let op = *op_table.choose(rng).unwrap();

    match op {
        0 => {
            let inner = random_pbes_expr(rng, depth - 1, freevars, config, !negated);
            PbesExprKind::Negation(Box::new(inner)).into()
        }
        1 => {
            let l = random_pbes_expr(rng, depth - 1, freevars, config, negated);
            let r = random_pbes_expr(rng, depth - 1, freevars, config, negated);
            PbesExprKind::Binary {
                op: PbesExprBinaryOp::Conjunction,
                lhs: Box::new(l),
                rhs: Box::new(r),
            }
            .into()
        }
        2 => {
            let l = random_pbes_expr(rng, depth - 1, freevars, config, negated);
            let r = random_pbes_expr(rng, depth - 1, freevars, config, negated);
            PbesExprKind::Binary {
                op: PbesExprBinaryOp::Disjunction,
                lhs: Box::new(l),
                rhs: Box::new(r),
            }
            .into()
        }
        3 => {
            // Antecedent flips polarity for monotonicity.
            let l = random_pbes_expr(rng, depth - 1, freevars, config, !negated);
            let r = random_pbes_expr(rng, depth - 1, freevars, config, negated);
            PbesExprKind::Binary {
                op: PbesExprBinaryOp::Implies,
                lhs: Box::new(l),
                rhs: Box::new(r),
            }
            .into()
        }
        4 => random_quantifier(rng, Quantifier::Forall, depth - 1, freevars, config, negated),
        5 => random_quantifier(rng, Quantifier::Exists, depth - 1, freevars, config, negated),
        _ => unreachable!(),
    }
}

/// Generates a random quantifier expression. The quantifier variable is always of type Nat and is
/// artificially bounded (e.g. `forall t. t < 3 => body`) to ensure termination of the generator,
/// and is added to `freevars` when generating the body. If `negated` is true, the quantifier is
/// generated with negative polarity, which biases the generator to produce more negations and
/// implications, which flip polarity.
fn random_quantifier<R: Rng>(
    rng: &mut R,
    quantifier: Quantifier,
    depth: usize,
    freevars: &[IdDecl],
    config: &PbesGenConfig,
    negated: bool,
) -> PbesExpr {
    let available: Vec<&str> = QUANT_INTS
        .iter()
        .filter(|&&q| !freevars.iter().any(|fv| fv.identifier.node == q))
        .copied()
        .collect();

    if available.is_empty() {
        return random_leaf(rng, freevars, config, negated);
    }

    let var_name = (*available.choose(rng).expect("available is non-empty")).to_string();
    let var_decl = IdDecl::new(
        var_name.clone(),
        SortExpressionKind::Simple(Sort::Nat).into(),
        Span::default(),
    );

    let mut new_freevars = freevars.to_vec();
    new_freevars.push(as_expr_decl(&var_name, &SortExpressionKind::Simple(Sort::Nat).into()));

    let body = random_pbes_expr(rng, depth, &new_freevars, config, negated);

    // Bound the quantifier variable to ensure termination: forall t. t < 3 => body  /  exists t. t < 3 && body
    let bound = PbesExprKind::DataValExpr(
        DataExprKind::Binary {
            op: DataExprBinaryOp::LessThan,
            lhs: Box::new(DataExprKind::Id(var_name).into()),
            rhs: Box::new(DataExprKind::Number("3".to_string()).into()),
        }
        .into(),
    )
    .into();
    let bounded_body = match quantifier {
        Quantifier::Forall => PbesExprKind::Binary {
            op: PbesExprBinaryOp::Implies,
            lhs: Box::new(bound),
            rhs: Box::new(body),
        }
        .into(),
        Quantifier::Exists => PbesExprKind::Binary {
            op: PbesExprBinaryOp::Conjunction,
            lhs: Box::new(bound),
            rhs: Box::new(body),
        }
        .into(),
    };

    PbesExprKind::Quantifier {
        quantifier,
        variables: vec![var_decl],
        body: Box::new(bounded_body),
    }
    .into()
}

fn is_bool_var(name: &str) -> bool {
    PRED_BOOLS.contains(&name)
}

fn as_expr_decl(name: &str, sort: &SortExpression) -> IdDecl {
    IdDecl::new(name.to_string(), sort.clone(), Span::default())
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
            .map(|(name, sort)| as_expr_decl(name, sort))
            .collect()
    }
}

/// Generates a predicate variable's parameter list: 0-2 classic Bool/Nat parameters (as
/// [`random_pbes`] always has), plus, when `sort_decls` is non-empty, one additional parameter
/// typed with a random declared sort -- exercising the type checker on struct/container/function
/// parameter sorts that plain [`random_pbes`] never generates.
fn make_pred_var<R: Rng>(rng: &mut R, index: usize, use_integers: bool, sort_decls: &[SortDecl]) -> PredVar {
    let size = rng.random_range(0..=2usize);
    let mut pool: Vec<&str> = if use_integers {
        PRED_INTS.iter().chain(PRED_BOOLS.iter()).copied().collect()
    } else {
        PRED_BOOLS.to_vec()
    };
    let mut params = Vec::new();
    for _ in 0..size {
        if pool.is_empty() {
            break;
        }
        let idx = rng.random_range(0..pool.len());
        let name = pool.remove(idx).to_string();
        let sort = if is_bool_var(&name) { Sort::Bool } else { Sort::Nat };
        params.push((name, SortExpressionKind::Simple(sort).into()));
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
