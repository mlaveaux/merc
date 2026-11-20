use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::process::Command;

use itertools::Itertools;
use log::info;
use merc_collections::VecBag;
use merc_data::DataExpression;
use merc_io::temp_dir;
use merc_io::traced_command;
use merc_lts::LTS;
use merc_lts::LabelIndex;
use merc_lts::LabelledTransitionSystem;
use merc_lts::LtsAction;
use merc_lts::LtsBuilderMem;
use merc_lts::LtsMultiAction;
use merc_lts::MutexLtsBuilder;
use merc_lts::SimpleAction;
use merc_lts::StateIndex;
use merc_lts::TransitionLabel;
use merc_lts::random_lts;
use merc_lts::read_lts;
use merc_lts::write_mcrl2_aut;
use merc_reduction::Equivalence;
use merc_reduction::compare_lts;
use merc_syntax::CommExpr;
use merc_syntax::MultiActionLabel;
use merc_syntax::Span;
use merc_syntax::parse_action_names;
use merc_syntax::parse_allow_action_names;
use merc_syntax::parse_comm_expr_set;
use merc_syntax::respan;
use merc_utilities::Timing;
use merc_utilities::random_test;
use rand::RngExt;
use rand::seq::IndexedRandom;
use rand::seq::IteratorRandom;

use merc_explore::CombineLabel;
use merc_explore::SendAction;
use merc_explore::combine_lts;
use merc_explore::combine_lts_actions_parallel;
use merc_explore::combine_lts_parallel;

/// Returns a random multi-action label with action names sampled from the given list.
fn random_multi_action<R: rand::Rng>(rng: &mut R, actions: &[String], max_size: usize) -> MultiActionLabel {
    let max_size = usize::min(actions.len(), max_size);
    let size = rng.random_range(0..=max_size);
    let selected_actions = actions
        .sample(rng, size)
        .map(|name| respan(Span::default(), name.clone()))
        .collect::<Vec<_>>();
    MultiActionLabel::new(selected_actions)
}

/// An LTS with the given labels after tau (label index 0), in which the transitions are (from, label index, to).
fn make_lts(
    labels: &[&str],
    num_of_states: usize,
    transitions: &[(usize, usize, usize)],
) -> LabelledTransitionSystem<LtsMultiAction<SimpleAction>> {
    let labels = std::iter::once(LtsMultiAction::tau_label())
        .chain(labels.iter().map(|label| LtsMultiAction::from_string(label).unwrap()))
        .collect();
    LabelledTransitionSystem::new(
        StateIndex::new(0),
        Some(num_of_states),
        || {
            transitions
                .iter()
                .map(|&(from, label, to)| (StateIndex::new(from), LabelIndex::new(label), StateIndex::new(to)))
        },
        labels,
    )
}

/// Exactly the actions with equal arguments are compatible for communication, and converting back returns the
/// original actions.
#[test]
fn test_send_action_conversion() {
    let single = |label: &str, argument: &str| {
        LtsMultiAction::new(VecBag::from_vec(vec![LtsAction::new(
            label.to_owned(),
            vec![DataExpression::from_string(argument).unwrap()],
        )]))
    };
    let (a_one, b_one, b_two) = (single("a", "1"), single("b", "1"), single("b", "f(2)"));

    let converted: Vec<LtsMultiAction<SendAction>> = [&a_one, &b_one, &b_two]
        .iter()
        .map(|action| SendAction::from_multi_action(action))
        .collect();
    let action = |i: usize| converted[i].actions().iter().next().unwrap();

    assert!(action(0).comm_args_compatible(action(1)));
    assert!(!action(1).comm_args_compatible(action(2)));
    assert!(action(0).matches_label("a") && action(2).matches_label("b"));
    assert_eq!(SendAction::from_multi_action(&b_two), converted[2]);
    for (original, converted) in [&a_one, &b_one, &b_two].into_iter().zip(&converted) {
        assert_eq!(&SendAction::to_multi_action(converted), original);
    }
}

#[test]
fn test_combine_communication_allow_and_hide() {
    // The first LTS does a and then c or e, the second does b. The combination a|b is reachable from two states of
    // the composition, so its cached label is used twice.
    let ltss = vec![
        make_lts(&["a", "c", "e"], 2, &[(0, 1, 1), (1, 2, 0), (1, 3, 0)]),
        make_lts(&["b"], 1, &[(0, 1, 0)]),
    ];

    // a and b only occur in the communication d; e is not allowed and c is hidden.
    let comm = parse_comm_expr_set("{a|b -> d}").unwrap();
    let allow = parse_allow_action_names("{d, c}").unwrap();
    let hide = parse_action_names("{c}").unwrap();

    let mut builder = LtsBuilderMem::new(Vec::new(), Vec::new());
    let result = combine_lts(&mut builder, ltss, &hide, &allow, &comm, &Timing::new()).unwrap();
    let result = &result;

    let transitions = result
        .iter_states()
        .flat_map(|s| {
            result
                .outgoing_transitions(s)
                .map(move |t| (s.value(), result.labels()[t.label].to_string(), t.to.value()))
        })
        .sorted()
        .collect::<Vec<_>>();
    assert_eq!(result.num_of_states(), 2);
    assert_eq!(
        transitions,
        vec![
            (0, "d".to_string(), 1),
            (1, LtsMultiAction::<SimpleAction>::tau_label().to_string(), 0)
        ]
    );
}

#[test]
fn test_mcrl2_ltscombine() {
    let Ok(mcrl2_path) = std::env::var("MCRL2_PATH") else {
        println!("Skipping test: MCRL2_PATH not set");
        return;
    };

    let mcrl2_ltscombine = Path::new(&mcrl2_path).join("ltscombine");
    let mcrl2_mcrl22lps = Path::new(&mcrl2_path).join("mcrl22lps");
    let mcrl2_ltsconvert = Path::new(&mcrl2_path).join("ltsconvert");

    // Write the random LTS to a temp file for ltsconvert to process.
    let temp_dir = temp_dir("test_mcrl2_ltscombine").unwrap();

    let spec_path = temp_dir.path().join("spec.mcrl2");
    let lps_path = temp_dir.path().join("spec.lps");
    let output_path = temp_dir.path().join("output.lts");

    // Generate a dummy linear process specification to convert the .aut files to .lts format.
    writeln!(&mut File::create(&spec_path).unwrap(), "act a, b, c; init delta;").unwrap();

    let status = traced_command(Command::new(&mcrl2_mcrl22lps).arg(&spec_path).arg(&lps_path))
        .expect("Failed to run ltsconvert");
    assert!(status.success(), "ltsconvert failed with status: {status}");

    random_test(100, |rng| {
        let left_lts = random_lts::<String, _>(rng, 1000, 3)
            .relabel(|label| Ok(LtsMultiAction::new(VecBag::singleton(LtsAction::new(label, vec![])))))
            .unwrap();
        let right_lts = random_lts::<String, _>(rng, 1000, 3)
            .relabel(|label| Ok(LtsMultiAction::new(VecBag::singleton(LtsAction::new(label, vec![])))))
            .unwrap();

        let left_path = temp_dir.path().join("left.aut");
        let right_path = temp_dir.path().join("right.aut");
        write_mcrl2_aut(&mut File::create(&left_path).unwrap(), &left_lts).unwrap();
        write_mcrl2_aut(&mut File::create(&right_path).unwrap(), &right_lts).unwrap();

        // For mCRL2's ltscombine we need to convert the inputs to the mCRL2 LTS format.
        let left_lts_path = temp_dir.path().join("left.lts");
        let right_lts_path = temp_dir.path().join("right.lts");
        let status = traced_command(
            Command::new(&mcrl2_ltsconvert)
                .arg("-enone")
                .arg(&left_path)
                .arg(&left_lts_path)
                .arg("-l")
                .arg(&lps_path),
        )
        .expect("Failed to run ltsconvert");
        assert!(status.success(), "ltsconvert failed with status: {status}");

        let status = traced_command(
            Command::new(&mcrl2_ltsconvert)
                .arg("-enone")
                .arg(&right_path)
                .arg(&right_lts_path)
                .arg("-l")
                .arg(&lps_path),
        )
        .expect("Failed to run ltsconvert");
        assert!(status.success(), "ltsconvert failed with status: {status}");

        // Allow an arbitrary subset of labels
        let labels = left_lts
            .labels()
            .iter()
            .chain(right_lts.labels().iter())
            .map(|l| l.to_string())
            .filter(|label| !label.is_tau_label())
            .collect::<Vec<_>>();

        let num_of_allowed = rng.random_range(0..=labels.len());
        let allow = (0..num_of_allowed)
            .map(|_| random_multi_action(rng, &labels, 3))
            .filter(|a| !a.is_tau_label())
            .collect::<Vec<_>>();

        let num_of_hidden = rng.random_range(0..=labels.len());
        let hide = labels
            .iter()
            .filter(|a| !a.is_tau_label())
            .cloned()
            .sample(rng, num_of_hidden);

        let num_of_comm = rng.random_range(0..=5);
        let mut comm = (0..num_of_comm)
            .map(|_| {
                let size = rng.random_range(2..=3);
                let actions = random_multi_action(rng, &labels, size);
                let to = respan(Span::default(), labels.choose(rng).unwrap().clone());
                CommExpr::new(actions, to)
            })
            .filter(|comm| {
                !comm.from.is_tau_label() && comm.from.actions.contains(&comm.to) && comm.from.actions.len() >= 2
            })
            .collect::<Vec<_>>();

        comm.sort_unstable();
        comm.dedup();

        // Remove communication expressions with overlapping left-hand sides.
        let comm = comm
            .iter()
            .filter(|&comm_i| {
                !comm.iter().any(|comm_j| {
                    comm_i != comm_j && comm_i.from.actions.iter().any(|a| comm_j.from.actions.contains(a))
                })
            })
            .cloned()
            .collect::<Vec<_>>();

        info!("Allow set {{{}}}", allow.iter().format(", "));
        info!("Hide set {{{}}}", hide.iter().format(", "));
        info!("Comm set {{{}}}", comm.iter().format(", "));

        // Use ltscombine to compute the combined LTS, which we will compare against our implementation's result
        let mut command = Command::new(&mcrl2_ltscombine);
        command.arg(&left_lts_path).arg(&right_lts_path);
        if !allow.is_empty() {
            command.arg(format!("--allow={{{}}}", allow.iter().format(", ")));
        }
        if !hide.is_empty() {
            command.arg(format!("--hide={{{}}}", hide.iter().format(", ")));
        }
        if !comm.is_empty() {
            command.arg(format!("--comm={{{}}}", comm.iter().format(", ")));
        }
        command.arg(&output_path);

        let status = traced_command(&mut command).expect("Failed to run ltscombine");
        assert!(status.success(), "ltscombine failed with status: {status}");

        let (expected_lts, _data_spec) = read_lts(&File::open(&output_path).unwrap(), false).unwrap();
        let expected_path = temp_dir.path().join("expected.aut");
        write_mcrl2_aut(&mut File::create(&expected_path).unwrap(), &expected_lts).unwrap();

        // The arguments of the actions are interned to combine them in parallel.
        let parallel_lts = combine_lts_actions_parallel(
            vec![left_lts.clone(), right_lts.clone()],
            &hide,
            &allow,
            &comm,
            4,
            &Timing::new(),
        )
        .unwrap();

        let mut result: LtsBuilderMem<LtsMultiAction<LtsAction>> = LtsBuilderMem::new(Vec::new(), Vec::new());
        let result_lts = combine_lts(
            &mut result,
            vec![left_lts, right_lts],
            &hide,
            &allow,
            &comm,
            &Timing::new(),
        )
        .unwrap();

        let result_path = temp_dir.path().join("result.aut");
        write_mcrl2_aut(&mut File::create(&result_path).unwrap(), &result_lts).unwrap();

        assert!(
            compare_lts(
                Equivalence::StrongBisim,
                expected_lts.clone().with_state_labels(()),
                result_lts,
                false,
                false,
                &Timing::new(),
            )
            .0,
            "The resulting LTSs are not bisimilar."
        );
        assert!(
            compare_lts(
                Equivalence::StrongBisim,
                expected_lts.with_state_labels(()),
                parallel_lts,
                false,
                false,
                &Timing::new(),
            )
            .0,
            "The LTS combined in parallel is not bisimilar."
        );
    });
}

/// The parallel composition computed with several threads is strongly bisimilar to the one computed with one thread,
/// and has the same transitions up to the numbering of the states.
#[test]
#[cfg_attr(miri, ignore)] // Ignored under Miri because of rayon usage
fn test_combine_parallel_matches_sequential() {
    random_test(20, |rng| {
        let random_test_lts = |rng: &mut _| {
            random_lts::<String, _>(rng, 10, 5)
                .relabel(|label| LtsMultiAction::<SimpleAction>::from_string(&label))
                .unwrap()
        };
        let ltss = vec![random_test_lts(rng), random_test_lts(rng), random_test_lts(rng)];
        let comm = parse_comm_expr_set("{a|b -> c}").unwrap();
        let allow = parse_allow_action_names("{a, c, a|c, d}").unwrap();
        let hide = parse_action_names("{a}").unwrap();

        let sequential = combine_lts(
            &mut LtsBuilderMem::new(Vec::new(), Vec::new()),
            ltss.clone(),
            &hide,
            &allow,
            &comm,
            &Timing::new(),
        )
        .unwrap();
        let parallel = combine_lts_parallel(
            &mut MutexLtsBuilder::new(LtsBuilderMem::new(Vec::new(), Vec::new())),
            ltss,
            &hide,
            &allow,
            &comm,
            4,
            &Timing::new(),
        )
        .unwrap();

        assert_eq!(sequential.num_of_transitions(), parallel.num_of_transitions());
        assert!(
            compare_lts(
                Equivalence::StrongBisim,
                sequential,
                parallel,
                false,
                false,
                &Timing::new()
            )
            .0,
            "The sequential and parallel compositions are not bisimilar."
        );
    });
}
