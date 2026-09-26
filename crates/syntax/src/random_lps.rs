use rand::Rng;
use rand::RngExt;
use rand::seq::IndexedRandom;

use merc_utilities::Span;

use crate::ActDecl;
use crate::ActionName;
use crate::Assignment;
use crate::CommExpr;
use crate::DataExpr;
use crate::DataExprBinaryOp;
use crate::DataExprKind;
use crate::IdDecl;
use crate::MultiActionLabel;
use crate::ProcDecl;
use crate::ProcExprBinaryOp;
use crate::ProcessExpr;
use crate::ProcessExprKind;
use crate::Rename;
use crate::Sort;
use crate::SortDecl;
use crate::SortExpressionKind;
use crate::UntypedDataSpecification;
use crate::UntypedProcessSpecification;
use crate::random_boolean_data_expression;
use crate::random_data_specification;
use crate::random_value_expression;
use crate::respan;

/// Generates a random linear process specification as an AST.
///
/// Models a finite-state machine: `num_states` states (encoded as a `Nat` parameter `s`),
/// `num_actions` label-only actions (a0, a1, …), each (state, action) pair enabled with
/// probability `transition_prob`, and the target state chosen uniformly. The returned
/// specification is accepted by `txt2lps` when printed (no linearization step required).
pub fn random_lps<R: Rng>(
    rng: &mut R,
    num_states: usize,
    num_actions: usize,
    transition_prob: f64,
) -> UntypedProcessSpecification {
    assert!(num_states >= 1 && num_actions >= 1);

    let action_names: Vec<String> = (0..num_actions).map(|i| format!("a{i}")).collect();

    let action_declarations = action_names
        .iter()
        .map(|name| ActDecl {
            identifier: synthetic_name(name.clone()),
            args: Vec::new(),
            span: Span::default(),
        })
        .collect();

    let s_param = IdDecl::new(
        "s".to_string(),
        SortExpressionKind::Simple(Sort::Nat).into(),
        Span::default(),
    );

    let mut summands: Vec<ProcessExpr> = Vec::new();
    for from in 0..num_states {
        for act in &action_names {
            if rng.random_bool(transition_prob) {
                let to = rng.random_range(0..num_states);
                let condition = DataExprKind::Binary {
                    op: DataExprBinaryOp::Equal,
                    lhs: Box::new(DataExprKind::Id("s".to_string()).into()),
                    rhs: Box::new(DataExprKind::Number(from.to_string()).into()),
                }
                .into();
                let seq = ProcessExprKind::Binary {
                    op: ProcExprBinaryOp::Sequence,
                    lhs: Box::new(ProcessExprKind::Action(respan(Span::default(), act.clone()), Vec::new()).into()),
                    rhs: Box::new(
                        ProcessExprKind::Id(
                            respan(Span::default(), "P".to_string()),
                            vec![Assignment::new(
                                "s".to_string(),
                                DataExprKind::Number(to.to_string()).into(),
                            )],
                        )
                        .into(),
                    ),
                }
                .into();
                summands.push(
                    ProcessExprKind::Condition {
                        condition,
                        then: Box::new(seq),
                        else_: None,
                    }
                    .into(),
                );
            }
        }
    }

    // Keep the spec syntactically valid when no transitions are generated.
    if summands.is_empty() {
        summands.push(ProcessExprKind::Delta.into());
    }

    let body = summands
        .into_iter()
        .reduce(|acc, s| {
            ProcessExprKind::Binary {
                op: ProcExprBinaryOp::Choice,
                lhs: Box::new(acc),
                rhs: Box::new(s),
            }
            .into()
        })
        .expect("summands is non-empty");

    let process_declarations = vec![ProcDecl {
        identifier: respan(Span::default(), "P".to_string()),
        params: vec![s_param],
        body,
        span: Span::default(),
    }];

    let init_state = rng.random_range(0..num_states);
    let init = ProcessExprKind::Id(
        respan(Span::default(), "P".to_string()),
        vec![Assignment::new(
            "s".to_string(),
            DataExprKind::Number(init_state.to_string()).into(),
        )],
    )
    .into();

    UntypedProcessSpecification {
        data_specification: UntypedDataSpecification::default(),
        global_variables: Vec::new(),
        action_declarations,
        process_declarations,
        init: Some(init),
    }
}

const ACTIONS: &[&str] = &["a", "b", "c", "d"];
const PROC_NAMES: &[&str] = &["P", "Q", "R"];

// Sum-variable names are kept distinct from process parameter names (b, c, m, n).
const SUM_VARS: &[&str] = &["s1", "s2", "s3"];

fn id_decl(name: &str, sort: Sort) -> IdDecl {
    IdDecl::new(
        name.to_string(),
        SortExpressionKind::Simple(sort).into(),
        Span::default(),
    )
}

struct ProcVar {
    name: String,
    params: Vec<IdDecl>,
}

/// Generates a process variable's parameter list: 0-2 classic `Bool`/`Nat` parameters (as
/// [`make_process_specification`] always has), plus, when `sort_decls` is non-empty, one
/// additional parameter typed with a random declared sort -- exercising the type checker on
/// struct/container/function parameter sorts that plain [`make_process_specification`] never
/// generates.
fn random_proc_var<R: Rng>(rng: &mut R, index: usize, use_integers: bool, sort_decls: &[SortDecl]) -> ProcVar {
    let size = rng.random_range(0..=2usize);
    let mut pool: Vec<IdDecl> = vec![id_decl("b", Sort::Bool), id_decl("c", Sort::Bool)];
    if use_integers {
        pool.push(id_decl("m", Sort::Nat));
        pool.push(id_decl("n", Sort::Nat));
    }
    let mut params = Vec::new();
    for _ in 0..size {
        if pool.is_empty() {
            break;
        }
        let idx = rng.random_range(0..pool.len());
        params.push(pool.remove(idx));
    }

    if let Some(decl) = sort_decls.choose(rng) {
        let sort_ref = SortExpressionKind::Reference(decl.identifier.clone()).into();
        params.push(IdDecl::new(format!("rich{index}"), sort_ref, Span::default()));
    }

    ProcVar {
        name: PROC_NAMES[index].to_string(),
        params,
    }
}

/// Generates a value for a process-variable parameter of the given declared sort. Delegates to
/// [`random_value_expression`] uniformly: unlike calling `random_integer_data_expression` directly
/// for a `Nat` parameter, it keeps the value well-sorted (that generator's `Subtract` can go
/// negative, which is not a valid `Nat` value on its own).
fn random_param_value<R: Rng>(rng: &mut R, param: &IdDecl, freevars: &[IdDecl], sort_decls: &[SortDecl]) -> DataExpr {
    random_value_expression(rng, sort_decls, &param.sort, freevars, 2)
}

fn random_process_instance<R: Rng>(
    rng: &mut R,
    pv: &ProcVar,
    freevars: &[IdDecl],
    sort_decls: &[SortDecl],
) -> ProcessExpr {
    let assignments = pv
        .params
        .iter()
        .map(|p| {
            Assignment::new(
                p.identifier.node.clone(),
                random_param_value(rng, p, freevars, sort_decls),
            )
        })
        .collect();
    ProcessExprKind::Id(respan(Span::default(), pv.name.clone()), assignments).into()
}

fn random_leaf<R: Rng>(
    rng: &mut R,
    freevars: &[IdDecl],
    actions: &[&str],
    proc_vars: &[ProcVar],
    sort_decls: &[SortDecl],
    is_guarded: bool,
) -> ProcessExpr {
    // Build a weighted table: Action has high weight, Delta/Tau low, ProcessInstance medium.
    let mut table: Vec<usize> = vec![0, 1]; // Delta=0, Tau=1
    if !actions.is_empty() {
        table.extend(std::iter::repeat_n(2, 8));
    }

    if !proc_vars.is_empty() && !is_guarded {
        table.extend(std::iter::repeat_n(3, 2)); // ProcessInstance=3
    }

    match *table.choose(rng).expect("table always contains at least Delta and Tau") {
        0 => ProcessExprKind::Delta.into(),
        1 => ProcessExprKind::Tau.into(),
        2 => ProcessExprKind::Action(
            respan(
                Span::default(),
                (*actions.choose(rng).expect("actions is non-empty")).to_string(),
            ),
            Vec::new(),
        )
        .into(),
        3 => {
            let pv = proc_vars.choose(rng).expect("proc_vars is non-empty");
            random_process_instance(rng, pv, freevars, sort_decls)
        }
        _ => unreachable!(),
    }
}

fn random_process_expr<R: Rng>(
    rng: &mut R,
    depth: usize,
    freevars: &[IdDecl],
    actions: &[&str],
    proc_vars: &[ProcVar],
    sort_decls: &[SortDecl],
    is_guarded: bool,
) -> ProcessExpr {
    if depth == 0 {
        return random_leaf(rng, freevars, actions, proc_vars, sort_decls, is_guarded);
    }

    // op: 0=leaf, 1=sum, 2=if-then, 3=if-then-else, 4=choice, 5=seq
    // seq is over-represented to bias toward action-guarded continuations.
    let op_table: &[usize] = &[0, 0, 1, 2, 3, 4, 4, 5, 5, 5];
    match *op_table.choose(rng).expect("op_table is a non-empty constant") {
        0 => random_leaf(rng, freevars, actions, proc_vars, sort_decls, is_guarded),
        1 => {
            // Sum: bind a fresh variable chosen from SUM_VARS to avoid capture.
            let var_name = SUM_VARS
                .iter()
                .find(|&&n| !freevars.iter().any(|v| v.identifier.node == n))
                .copied();
            match var_name {
                None => random_leaf(rng, freevars, actions, proc_vars, sort_decls, is_guarded),
                Some(name) => {
                    let sort = if rng.random_bool(0.5) { Sort::Bool } else { Sort::Nat };
                    let var = id_decl(name, sort);
                    let mut new_vars = freevars.to_vec();
                    new_vars.push(var.clone());
                    let body =
                        random_process_expr(rng, depth - 1, &new_vars, actions, proc_vars, sort_decls, is_guarded);
                    ProcessExprKind::Sum {
                        variables: vec![var],
                        operand: Box::new(body),
                    }
                    .into()
                }
            }
        }
        2 => {
            // IfThen: condition -> body
            let cond = random_boolean_data_expression(rng, freevars);
            let body = random_process_expr(rng, depth - 1, freevars, actions, proc_vars, sort_decls, is_guarded);
            ProcessExprKind::Condition {
                condition: cond,
                then: Box::new(body),
                else_: None,
            }
            .into()
        }
        3 => {
            // IfThenElse: condition -> x <> y
            let cond = random_boolean_data_expression(rng, freevars);
            let then = random_process_expr(rng, depth - 1, freevars, actions, proc_vars, sort_decls, is_guarded);
            let else_ = random_process_expr(rng, depth - 1, freevars, actions, proc_vars, sort_decls, is_guarded);
            ProcessExprKind::Condition {
                condition: cond,
                then: Box::new(then),
                else_: Some(Box::new(else_)),
            }
            .into()
        }
        4 => {
            // Choice: lhs + rhs (each branch must independently satisfy guardedness)
            let lhs = random_process_expr(rng, depth - 1, freevars, actions, proc_vars, sort_decls, is_guarded);
            let rhs = random_process_expr(rng, depth - 1, freevars, actions, proc_vars, sort_decls, is_guarded);
            ProcessExprKind::Binary {
                op: ProcExprBinaryOp::Choice,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            }
            .into()
        }
        5 => {
            // Seq: emit an action first, then an (unguarded) continuation.
            // The explicit action satisfies the guard, so the rhs may reference proc instances.
            let action = (*actions.choose(rng).expect("actions is non-empty")).to_string();
            let lhs = ProcessExprKind::Action(respan(Span::default(), action), Vec::new()).into();
            let rhs = random_process_expr(rng, depth - 1, freevars, actions, proc_vars, sort_decls, false);
            ProcessExprKind::Binary {
                op: ProcExprBinaryOp::Sequence,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            }
            .into()
        }
        _ => unreachable!(),
    }
}

/// Wraps a synthetically-generated action name in a default (empty) [Span].
fn synthetic_name(name: String) -> ActionName {
    respan(Span::default(), name)
}

fn apply_wrapper<R: Rng>(rng: &mut R, actions: &[&str], expr: ProcessExpr) -> ProcessExpr {
    match rng.random_range(0..5usize) {
        0 => {
            let a = (*actions.choose(rng).expect("actions is non-empty")).to_string();
            ProcessExprKind::Hide {
                actions: vec![synthetic_name(a)],
                operand: Box::new(expr),
            }
            .into()
        }
        1 => {
            let a = (*actions.choose(rng).expect("actions is non-empty")).to_string();
            ProcessExprKind::Block {
                actions: vec![synthetic_name(a)],
                operand: Box::new(expr),
            }
            .into()
        }
        2 if actions.len() >= 2 => {
            let mut pool = actions.to_vec();
            let ai = rng.random_range(0..pool.len());
            let from = pool.remove(ai).to_string();
            let to = (*pool
                .choose(rng)
                .expect("pool has at least one element after removing from"))
            .to_string();
            ProcessExprKind::Rename {
                renames: vec![Rename {
                    from: synthetic_name(from),
                    to: synthetic_name(to),
                }],
                operand: Box::new(expr),
            }
            .into()
        }
        3 if actions.len() >= 3 => {
            // Comm: a | b -> c  (mCRL2 synchronisation mapping)
            let mut pool: Vec<String> = actions.iter().map(|&s| s.to_string()).collect();
            let ai = rng.random_range(0..pool.len());
            let a = pool.remove(ai);
            let bi = rng.random_range(0..pool.len());
            let b = pool.remove(bi);
            let c = pool
                .choose(rng)
                .expect("pool has at least one element after removing a and b")
                .clone();
            ProcessExprKind::Comm {
                comm: vec![CommExpr::new(
                    MultiActionLabel::new(vec![synthetic_name(a), synthetic_name(b)]),
                    synthetic_name(c),
                )],
                operand: Box::new(expr),
            }
            .into()
        }
        4 => {
            let mut labels: Vec<MultiActionLabel> = (0..5)
                .map(|_| {
                    let size = rng.random_range(1..=2usize);
                    let mut acts: Vec<ActionName> = (0..size)
                        .map(|_| synthetic_name((*actions.choose(rng).expect("actions is non-empty")).to_string()))
                        .collect();
                    acts.sort();
                    acts.dedup();
                    MultiActionLabel::new(acts)
                })
                .collect();
            labels.sort();
            labels.dedup();
            ProcessExprKind::Allow {
                actions: labels,
                operand: Box::new(expr),
            }
            .into()
        }
        _ => expr, // guard failures from arms 2/3 fall here
    }
}

fn random_parallel_init<R: Rng>(
    rng: &mut R,
    actions: &[&str],
    mut procs: Vec<ProcessExpr>,
    wrapper_count: usize,
) -> ProcessExpr {
    // Fold all instances into a single parallel composition tree.
    while procs.len() > 1 {
        let n = procs.len();
        let j = rng.random_range(1..n);
        let p = procs.remove(j);
        let q = procs.remove(0);
        procs.push(
            ProcessExprKind::Binary {
                op: ProcExprBinaryOp::Parallel,
                lhs: Box::new(q),
                rhs: Box::new(p),
            }
            .into(),
        );
    }
    let mut result = procs.remove(0);
    for _ in 0..wrapper_count {
        result = apply_wrapper(rng, actions, result);
    }
    result
}

/// Generates a random mCRL2 process specification.
///
/// `equation_count` controls how many process equations are produced (capped at 3).
/// `depth` controls the maximum nesting depth of each process body.
/// `use_integers` adds `Nat`-typed parameters alongside `Bool` ones.
pub fn make_process_specification<R: Rng>(
    rng: &mut R,
    equation_count: usize,
    depth: usize,
    use_integers: bool,
) -> UntypedProcessSpecification {
    let proc_vars: Vec<ProcVar> = (0..equation_count.min(PROC_NAMES.len()))
        .map(|i| random_proc_var(rng, i, use_integers, &[]))
        .collect();
    make_process_specification_from_proc_vars(rng, proc_vars, depth, &[])
}

/// As [`make_process_specification`], but process variables additionally carry one parameter typed
/// with a sort from a freshly generated [`crate::random_data_specification`] (a struct, container,
/// or function sort), exercising the type checker on richer parameter sorts than plain
/// [`make_process_specification`] ever produces. `sort_count`/`max_sort_depth` control the
/// generated data specification, exactly as in `random_data_specification`.
pub fn make_process_specification_with_data_specification<R: Rng>(
    rng: &mut R,
    sort_count: usize,
    max_sort_depth: usize,
    equation_count: usize,
    depth: usize,
    use_integers: bool,
) -> UntypedProcessSpecification {
    let data_specification = random_data_specification(rng, sort_count, max_sort_depth);
    let count = equation_count.min(PROC_NAMES.len());
    let proc_vars: Vec<ProcVar> = (0..count)
        .map(|i| random_proc_var(rng, i, use_integers, &data_specification.sort_declarations))
        .collect();
    let mut spec =
        make_process_specification_from_proc_vars(rng, proc_vars, depth, &data_specification.sort_declarations);
    spec.data_specification = data_specification;
    spec
}

fn make_process_specification_from_proc_vars<R: Rng>(
    rng: &mut R,
    proc_vars: Vec<ProcVar>,
    depth: usize,
    sort_decls: &[SortDecl],
) -> UntypedProcessSpecification {
    let action_declarations = ACTIONS
        .iter()
        .map(|&name| ActDecl {
            identifier: synthetic_name(name.to_string()),
            args: Vec::new(),
            span: Span::default(),
        })
        .collect();

    let process_declarations: Vec<ProcDecl> = proc_vars
        .iter()
        .map(|pv| {
            let body = random_process_expr(rng, depth, &pv.params, ACTIONS, &proc_vars, sort_decls, true);
            ProcDecl {
                identifier: respan(Span::default(), pv.name.clone()),
                params: pv.params.clone(),
                body,
                span: Span::default(),
            }
        })
        .collect();

    let instances: Vec<ProcessExpr> = proc_vars
        .iter()
        .map(|pv| {
            let assignments = pv
                .params
                .iter()
                .map(|p| Assignment::new(p.identifier.node.clone(), random_param_value(rng, p, &[], sort_decls)))
                .collect();
            ProcessExprKind::Id(respan(Span::default(), pv.name.clone()), assignments).into()
        })
        .collect();

    let init = if instances.is_empty() {
        ProcessExprKind::Delta.into()
    } else {
        let wrapper_count = rng.random_range(0..=5usize);
        random_parallel_init(rng, ACTIONS, instances, wrapper_count)
    };

    UntypedProcessSpecification {
        data_specification: UntypedDataSpecification::default(),
        global_variables: Vec::new(),
        action_declarations,
        process_declarations,
        init: Some(init),
    }
}
