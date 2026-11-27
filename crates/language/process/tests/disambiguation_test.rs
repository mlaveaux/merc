//! Disambiguation of process conditions whose data expression absorbed process operators.

use merc_process::disambiguate_process_specification;
use merc_syntax::DataExprKind;
use merc_syntax::ProcExprBinaryOp;
use merc_syntax::ProcessExpr;
use merc_syntax::ProcessExprKind;
use merc_syntax::UntypedProcessSpecification;

fn disambiguated_body(text: &str) -> ProcessExpr {
    let mut spec = UntypedProcessSpecification::parse(text).expect("the fixture should parse");
    disambiguate_process_specification(&mut spec);
    spec.init.take().expect("the fixture always has an init")
}

fn disambiguated_proc_body(text: &str) -> ProcessExpr {
    let mut spec = UntypedProcessSpecification::parse(text).expect("the fixture should parse");
    disambiguate_process_specification(&mut spec);
    let decl = spec.process_declarations.pop().expect("the fixture always has a proc");
    decl.body
}

/// A single leading action step swallowed into the condition via `.` — the classic
/// `act(args) . cond -> P <> Q` shape.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn dot_swallowed_action_prefix_is_recovered() {
    let body = disambiguated_body("act a: Nat; init a(1) . true -> delta;");
    assert_eq!(body.to_string(), "(a(1) . ((true) -> (delta)))");
    let ProcessExprKind::Binary {
        op: ProcExprBinaryOp::Sequence,
        lhs,
        rhs,
    } = &body.node
    else {
        panic!("expected a Sequence, got {body:?}");
    };
    assert!(matches!(&lhs.node, ProcessExprKind::Action(name, _) if name.node == "a"));
    assert!(matches!(&rhs.node, ProcessExprKind::Condition { .. }));
}

/// A `+`-separated chain of guarded actions, each swallowed into the previous clause's
/// condition.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn choice_swallowed_guarded_actions_are_recovered() {
    let body = disambiguated_body("act a: Nat; b: Nat; init (true) -> a(1) + (false) -> b(2);");
    assert_eq!(body.to_string(), "(((true) -> (a(1))) + ((false) -> (b(2))))");
    let ProcessExprKind::Binary {
        op: ProcExprBinaryOp::Choice,
        lhs,
        rhs,
    } = &body.node
    else {
        panic!("expected a Choice, got {body:?}");
    };
    let ProcessExprKind::Condition { then: lhs_then, .. } = &lhs.node else {
        panic!("expected a Condition, got {lhs:?}");
    };
    assert!(matches!(&lhs_then.node, ProcessExprKind::Action(name, _) if name.node == "a"));
    let ProcessExprKind::Condition { then: rhs_then, .. } = &rhs.node else {
        panic!("expected a Condition, got {rhs:?}");
    };
    assert!(matches!(&rhs_then.node, ProcessExprKind::Action(name, _) if name.node == "b"));
}

/// The same shape with `||` (`Parallel`) in place of `+` (`Choice`): `P || (true) -> a` parses
/// `P || (true)` as one `DataExprDisj` chain rather than a parallel-composition operand
/// followed by the real condition.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn parallel_swallowed_condition_is_recovered() {
    let body = disambiguated_body("act a; proc P = delta; init P || (true) -> a;");
    assert_eq!(body.to_string(), "(P || ((true) -> (a)))");
    let ProcessExprKind::Binary {
        op: ProcExprBinaryOp::Parallel,
        lhs,
        rhs,
    } = &body.node
    else {
        panic!("expected a Parallel, got {body:?}");
    };
    assert!(matches!(&lhs.node, ProcessExprKind::Action(name, _) if name.node == "P"));
    let ProcessExprKind::Condition { condition, then, .. } = &rhs.node else {
        panic!("expected a Condition, got {rhs:?}");
    };
    assert!(matches!(&condition.node, DataExprKind::Bool(true)));
    assert!(matches!(&then.node, ProcessExprKind::Action(name, _) if name.node == "a"));
}

/// A three-way `+`-chain, each clause's action itself preceded by a `.`-sequence — combines
/// both swallow shapes at once.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn three_way_choice_with_sequenced_actions_is_recovered() {
    let body = disambiguated_body(
        "act a: Nat; b: Nat; c: Nat; init (true) -> a(1) . b(1) + (false) -> a(2) . b(2) + (true) -> c(3);",
    );
    assert_eq!(
        body.to_string(),
        "(((true) -> ((a(1) . b(1)))) + (((false) -> ((a(2) . b(2)))) + ((true) -> (c(3)))))"
    );
    let ProcessExprKind::Binary {
        op: ProcExprBinaryOp::Choice,
        lhs,
        rhs,
    } = &body.node
    else {
        panic!("expected a Choice, got {body:?}");
    };
    let ProcessExprKind::Condition { then: first_then, .. } = &lhs.node else {
        panic!("expected a Condition, got {lhs:?}");
    };
    assert!(matches!(
        &first_then.node,
        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Sequence,
            ..
        }
    ));

    let ProcessExprKind::Binary {
        op: ProcExprBinaryOp::Choice,
        lhs: second,
        rhs: third,
    } = &rhs.node
    else {
        panic!("expected a nested Choice, got {rhs:?}");
    };
    let ProcessExprKind::Condition { then: second_then, .. } = &second.node else {
        panic!("expected a Condition, got {second:?}");
    };
    assert!(matches!(
        &second_then.node,
        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Sequence,
            ..
        }
    ));
    let ProcessExprKind::Condition { then: third_then, .. } = &third.node else {
        panic!("expected a Condition, got {third:?}");
    };
    assert!(matches!(&third_then.node, ProcessExprKind::Action(name, _) if name.node == "c"));
}

/// A `+`-separated chain in `else_` position — `cond -> then <> a + cond2 -> then2 <> else2`,
/// mirroring `knights.mcrl2`'s `(...) -> jump . X(...) <> delta + (f==finalBoard) -> ready .
/// delta <> delta`. A bare `Binary { op: Choice, .. }` sitting directly in `then`/`else_`
/// position is indistinguishable from a genuine, deliberate `a + b` the user wrote (`cond ->
/// (a + b)` and `cond -> a + b` parse identically), so it is deliberately left alone.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn choice_directly_in_else_position_is_left_alone() {
    let body = disambiguated_proc_body(
        "act jump, ready; proc X(f: Bool) = (f) -> jump . X(f) <> delta + (!f) -> ready . delta <> delta; init X(true);",
    );
    assert_eq!(
        body.to_string(),
        "((f) -> ((jump . X(f))) <> ((delta + (((! f)) -> ((ready . delta)) <> (delta)))))"
    );
    let ProcessExprKind::Condition { then, else_, .. } = &body.node else {
        panic!("expected a Condition, got {body:?}");
    };
    assert!(matches!(
        &then.node,
        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Sequence,
            ..
        }
    ));
    let Some(else_) = else_ else {
        panic!("expected an else_ branch, got None");
    };
    assert!(matches!(
        &else_.node,
        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Choice,
            ..
        }
    ));
}

/// Minimal form of the same shape: `then` hiding an unrelated (`Add`-shaped) swallow must never
/// cost the current clause its own `<>`. `(ps==1) -> a(1) + (ps==2) -> a(2) <> b(9)` puts the
/// swallow (`a(1) + (ps==2) -> ...`) one level down from the outermost condition, and the
/// grammar attaches `<>`'s `b(9)` to the *innermost* reached `->` (`(ps==2)`, not `(ps==1)`).
/// `else_` must not be dropped, wherever the grammar attaches it.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn else_branch_survives_a_swallow_one_level_down() {
    let body = disambiguated_body("act a: Nat; b: Nat; init (true) -> a(1) + (false) -> a(2) <> b(9);");
    assert_eq!(body.to_string(), "(((true) -> (a(1))) + ((false) -> (a(2)) <> (b(9))))");
    let ProcessExprKind::Binary {
        op: ProcExprBinaryOp::Choice,
        lhs: first,
        rhs: second,
    } = &body.node
    else {
        panic!("expected a Choice, got {body:?}");
    };
    let ProcessExprKind::Condition { else_: first_else, .. } = &first.node else {
        panic!("expected a Condition, got {first:?}");
    };
    assert!(first_else.is_none(), "the swallow consumes the only `<>` in this input");
    let ProcessExprKind::Condition { else_: second_else, .. } = &second.node else {
        panic!("expected a Condition, got {second:?}");
    };
    let Some(second_else) = second_else else {
        panic!("expected `<> b(9)` to survive on the clause the grammar actually attaches it to");
    };
    assert!(matches!(&second_else.node, ProcessExprKind::Action(name, _) if name.node == "b"));
}

/// The minimal shape of the same class as [`choice_directly_in_else_position_is_left_alone`],
/// with `then` (not `else_`) directly `a + b`: `cond -> a + b <> c` must keep `b` unconditional
/// (not folded into `cond`'s own guard) *and* keep `c` as the `else_`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn choice_directly_in_then_position_keeps_its_own_else() {
    let body = disambiguated_body("act a: Nat; b: Nat; c: Nat; init (true) -> a(1) + b(2) <> c(3);");
    assert_eq!(body.to_string(), "((true) -> ((a(1) + b(2))) <> (c(3)))");
    let ProcessExprKind::Condition { condition, then, else_ } = &body.node else {
        panic!("expected a Condition, got {body:?}");
    };
    assert!(matches!(&condition.node, DataExprKind::Bool(true)));
    assert!(
        matches!(
            &then.node,
            ProcessExprKind::Binary {
                op: ProcExprBinaryOp::Choice,
                ..
            }
        ),
        "a + b must stay intact, got {then:?}"
    );
    let Some(else_) = else_ else {
        panic!("expected `<> c(3)` to survive, got None");
    };
    assert!(matches!(&else_.node, ProcessExprKind::Action(name, _) if name.node == "c"));
}

/// Leading, wholly *unconditional* `+`-branches (no `->` of their own at all) swallowed
/// straight into the one and only `Condition`'s `condition` field — the shape produced when a
/// process body has just one `->` in it, so the *whole* run up to that point is one legal
/// `DataExpr`. Mirrors `mutex.mcrl2`'s `Turn`: `r_assign_turnA.Turn(A) + r_assign_turnB.Turn(B)
/// + (t==A) -> s_read_turnA|label(true).Turn(A)`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn unconditional_leading_branches_are_recovered() {
    let body = disambiguated_proc_body("act a, b, c; proc P = a.P + b.P + (true) -> c.P; init P;");
    assert_eq!(body.to_string(), "(((a . P) + (b . P)) + ((true) -> ((c . P))))");
    // The two unconditional branches (`a.P`, `b.P`) peel off together, in one `Add`-chain
    // piece, as a nested `Choice` of their own; the real, guarded clause (`c.P`) is the
    // right-hand side.
    let ProcessExprKind::Binary {
        op: ProcExprBinaryOp::Choice,
        lhs: branches,
        rhs: guarded,
    } = &body.node
    else {
        panic!("expected a Choice, got {body:?}");
    };
    let ProcessExprKind::Binary {
        op: ProcExprBinaryOp::Choice,
        lhs: first,
        rhs: second,
    } = &branches.node
    else {
        panic!("expected a nested Choice, got {branches:?}");
    };
    assert!(matches!(
        &first.node,
        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Sequence,
            ..
        }
    ));
    assert!(matches!(
        &second.node,
        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Sequence,
            ..
        }
    ));
    let ProcessExprKind::Condition { condition, then, else_ } = &guarded.node else {
        panic!("expected a Condition, got {guarded:?}");
    };
    assert!(matches!(&condition.node, DataExprKind::Bool(true)));
    assert!(matches!(
        &then.node,
        ProcessExprKind::Binary {
            op: ProcExprBinaryOp::Sequence,
            ..
        }
    ));
    assert!(else_.is_none());
}

/// A genuine data expression that merely happens to use `.`/`+` — nothing declared as an
/// action or process appears in it — must be left untouched.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn a_genuine_condition_using_at_and_plus_is_left_alone() {
    let body = disambiguated_body("sort L = List(Nat); init (([1,2,3] . 0) + 1 == 2) -> delta;");
    assert_eq!(body.to_string(), "((((([1, 2, 3] . 0) + 1) == 2)) -> (delta))");
    assert!(matches!(&body.node, ProcessExprKind::Condition { .. }));
}

/// Nothing to fix at all: a plain, already-well-formed condition is unchanged.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn an_ordinary_condition_is_left_alone() {
    let body = disambiguated_body("init true -> delta;");
    assert_eq!(body.to_string(), "((true) -> (delta))");
    let ProcessExprKind::Condition { condition, then, else_ } = &body.node else {
        panic!("expected a Condition, got {body:?}");
    };
    assert!(matches!(&condition.node, DataExprKind::Bool(true)));
    assert!(matches!(&then.node, ProcessExprKind::Delta));
    assert!(else_.is_none());
}

/// `hide({a}, P) + (true) -> b` swallows the same way `+` alone does: `hide(...)` is
/// recognized as a process operator, so the whole run peels as an unconditional leading branch,
/// and `hide`'s own `{a}` set survives as `Hide`'s `actions`.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn hide_swallowed_guarded_choice_is_recovered() {
    let body = disambiguated_body("act a, b; proc P = delta; init hide({a}, P) + (true) -> b;");
    assert_eq!(body.to_string(), "(hide({a}, P) + ((true) -> (b)))");
    let ProcessExprKind::Binary {
        op: ProcExprBinaryOp::Choice,
        lhs,
        rhs,
    } = &body.node
    else {
        panic!("expected a Choice, got {body:?}");
    };
    let ProcessExprKind::Hide { actions, operand } = &lhs.node else {
        panic!("expected a Hide, got {lhs:?}");
    };
    assert_eq!(actions.iter().map(|name| name.as_str()).collect::<Vec<_>>(), vec!["a"]);
    assert!(matches!(&operand.node, ProcessExprKind::Action(name, _) if name.node == "P"));
    let ProcessExprKind::Condition { then, .. } = &rhs.node else {
        panic!("expected a Condition, got {rhs:?}");
    };
    assert!(matches!(&then.node, ProcessExprKind::Action(name, _) if name.node == "b"));
}

/// `comm` uses `from -> to` pairs inside its set argument, and `->` isn't a valid `DataExpr`
/// operator, so it can't reach the swallow at all — a hard parse error, not a silent misparse.
/// `rename` shares the same `from -> to` shape but does not match the ambiguous
/// `DataExpr`-typed `condition` position, falling back to its own dedicated grammar rule, so
/// it parses correctly here with nothing to fix.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn comm_fails_to_parse_and_rename_already_parses_correctly() {
    assert!(
        UntypedProcessSpecification::parse("act a, b; proc P = delta; init comm({a -> b}, P) + (true) -> b;").is_err()
    );

    let body = disambiguated_body("act a, b; proc P = delta; init rename({a -> b}, P) + (true) -> b;");
    assert_eq!(body.to_string(), "(rename({a -> b}, P) + ((true) -> (b)))");
    let ProcessExprKind::Binary {
        op: ProcExprBinaryOp::Choice,
        lhs,
        rhs,
    } = &body.node
    else {
        panic!("expected a Choice, got {body:?}");
    };
    assert!(matches!(&lhs.node, ProcessExprKind::Rename { .. }));
    assert!(matches!(&rhs.node, ProcessExprKind::Condition { .. }));
}

/// The fixture [`long_guarded_choice_chain_with_comments_is_recovered`] and
/// [`disambiguation_is_idempotent_for_the_long_guarded_choice_chain`] share: a long `is_x(state) ->
/// (...) + is_y(state) -> (...) + ...` chain of eight guarded clauses, each `then` a
/// parenthesized `.`/`+` mix, with a `%`-comment sitting between two clauses.
const LONG_GUARDED_CHOICE_CHAIN: &str = r#"act _JobWrapper_initialize, _JobWrapper_transferInputSandbox, _JobWrapper_resolveInputData, _JobWrapper_execute, _JobWrapper_processJobOutputs, _JobWrapper_finalize, AppPayload;
proc JobWrapper(cc:List(Nat),state:Nat) =

		is_startWrapper(state)->
		(
		_JobWrapper_initialize(cc,state).
			(
			_JobWrapper_initialize(cc,state).JobWrapper(cc,state) +
			_JobWrapper_initialize(cc,state).JobWrapper(cc,state) +
			JobWrapper(cc,state)
			)
		)
		+
		is_initializeOK(state)->
		(
			_JobWrapper_transferInputSandbox(cc,state).
			(
			_JobWrapper_transferInputSandbox(cc,state).JobWrapper(cc,state) +
			_JobWrapper_transferInputSandbox(cc,state).JobWrapper(cc,state) +
			JobWrapper(cc,state)
			)
		)
		+
		is_inputSandboxOK(state)->
		(
			_JobWrapper_resolveInputData(cc,state).
			(
			_JobWrapper_resolveInputData(cc,state).JobWrapper(cc,state) +
			_JobWrapper_resolveInputData(cc,state).JobWrapper(cc,state) +
			JobWrapper(cc,state)
			)
		)
		+
		is_resolveInputDataOK(state)->
		(
			_JobWrapper_execute(cc,state).
			(
			AppPayload.JobWrapper(cc,state) +
			_JobWrapper_execute(cc,state).JobWrapper(cc,state)
			)

		)
		+
		% creates an execution thread
		% passes the execution thread to a Watchdog instance (TBW)

		is_executeOK(state)->
		(
			_JobWrapper_execute(cc,state).JobWrapper(cc,state) +
		 	_JobWrapper_execute(cc,state).JobWrapper(cc,state) +
		 	_JobWrapper_execute(cc,state).JobWrapper(cc,state)
		)
		+

		is_completedOK(state)->
		(
			_JobWrapper_processJobOutputs(cc,state).
			_JobWrapper_processJobOutputs(cc,state).JobWrapper(cc,state)
			+
			_JobWrapper_processJobOutputs(cc,state).JobWrapper(cc,state)
		)
		+
		is_outputSandboxOK(state)->
		(
			_JobWrapper_processJobOutputs(cc,state).
			_JobWrapper_processJobOutputs(cc,state).JobWrapper(cc,state)
			+
			_JobWrapper_processJobOutputs(cc,state).JobWrapper(cc,state)
		)
		+
		is_outputDataOK(state)->
		(
			_JobWrapper_finalize(cc,state) +
			_JobWrapper_finalize(cc,state) +
			_JobWrapper_finalize(cc,state)
		).JobWrapper(cc,state)
	;
init JobWrapper([],0);
"#;

/// A long `is_x(state) -> (...) +
/// is_y(state) -> (...) + ...` chain of eight guarded clauses, each `then` a parenthesized
/// `.`/`+` mix, with a `%`-comment sitting between two clauses. Every guard must recover as its
/// own `Condition`, in order, not swallow a neighboring clause.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn long_guarded_choice_chain_with_comments_is_recovered() {
    let body = disambiguated_proc_body(LONG_GUARDED_CHOICE_CHAIN);

    // Flattens the top-level `Choice` spine, returning each clause's guard name (the
    // function name of its `Condition`'s `condition`, which is always a plain application
    // here: `is_x(state)`).
    fn guard_names(expr: &ProcessExpr) -> Vec<&str> {
        match &expr.node {
            ProcessExprKind::Binary {
                op: ProcExprBinaryOp::Choice,
                lhs,
                rhs,
            } => {
                let mut names = guard_names(lhs);
                names.extend(guard_names(rhs));
                names
            }
            ProcessExprKind::Condition { condition, .. } => {
                let DataExprKind::Application { function, .. } = &condition.node else {
                    panic!("expected an application guard, got {condition:?}");
                };
                let DataExprKind::Id(name) = &function.node else {
                    panic!("expected an Id-headed guard, got {function:?}");
                };
                vec![name.as_str()]
            }
            _ => panic!("expected a Choice or a Condition, got {expr:?}"),
        }
    }

    assert_eq!(
        guard_names(&body),
        [
            "is_startWrapper",
            "is_initializeOK",
            "is_inputSandboxOK",
            "is_resolveInputDataOK",
            "is_executeOK",
            "is_completedOK",
            "is_outputSandboxOK",
            "is_outputDataOK",
        ],
        "every guard must recover as its own Condition, in source order"
    );
}

/// Disambiguating an already-disambiguated specification must find nothing left to rewrite. Covers every shape the
/// tests above exercise (a dot-prefix swallow, a choice/parallel swallow, a swallow nested one
/// level down inside `then`, `else_` surviving every which way, leading unconditional
/// branches, `hide`/`rename`, and a genuine data condition that must be left alone) in one
/// pass; [`disambiguation_is_idempotent_for_the_long_guarded_choice_chain`] covers the long
/// real-world chain separately, since it needs the whole `spec` (not one extracted body).
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn disambiguation_is_idempotent() {
    let fixtures = [
        "act a: Nat; init a(1) . true -> delta;",
        "act a: Nat; b: Nat; init (true) -> a(1) + (false) -> b(2);",
        "act a; proc P = delta; init P || (true) -> a;",
        "act a: Nat; b: Nat; c: Nat; init (true) -> a(1) . b(1) + (false) -> a(2) . b(2) + (true) -> c(3);",
        "act jump, ready; proc X(f: Bool) = (f) -> jump . X(f) <> delta + (!f) -> ready . delta <> delta; init X(true);",
        "act a: Nat; b: Nat; init (true) -> a(1) + (false) -> a(2) <> b(9);",
        "act a: Nat; b: Nat; c: Nat; init (true) -> a(1) + b(2) <> c(3);",
        "act a, b, c; proc P = a.P + b.P + (true) -> c.P; init P;",
        "sort L = List(Nat); init (([1,2,3] . 0) + 1 == 2) -> delta;",
        "init true -> delta;",
        "act a, b; proc P = delta; init hide({a}, P) + (true) -> b;",
        "act a, b; proc P = delta; init rename({a -> b}, P) + (true) -> b;",
    ];

    for text in fixtures {
        let mut spec = UntypedProcessSpecification::parse(text).expect("the fixture should parse");
        disambiguate_process_specification(&mut spec);
        let once = spec.clone();

        disambiguate_process_specification(&mut spec);
        assert_eq!(
            spec, once,
            "disambiguating an already-disambiguated specification must be a no-op for {text:?}"
        );
    }
}

/// As [`disambiguation_is_idempotent`], for [`LONG_GUARDED_CHOICE_CHAIN`] — the fixture
/// [`long_guarded_choice_chain_with_comments_is_recovered`] also uses — on its own, since it
/// needs the real `proc`/`init` split rather than one extracted body.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn disambiguation_is_idempotent_for_the_long_guarded_choice_chain() {
    let mut spec = UntypedProcessSpecification::parse(LONG_GUARDED_CHOICE_CHAIN).expect("the fixture should parse");
    disambiguate_process_specification(&mut spec);
    let once = spec.clone();

    disambiguate_process_specification(&mut spec);
    assert_eq!(
        spec, once,
        "disambiguating an already-disambiguated specification must be a no-op"
    );
}
