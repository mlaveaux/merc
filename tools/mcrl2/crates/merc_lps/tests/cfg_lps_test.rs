use std::fs::File;
use std::path::Path;
use std::process::Command;

use mcrl2::LinearProcessSpecification;
use mcrl2::PreprocessOptions;
use mcrl2::preprocess;
use mcrl2::read_lps;
use merc_explore::CachingStrategy;
use merc_explore::ExplorationStrategy;
use merc_io::temp_dir;
use merc_io::traced_command;
use merc_lts::LTS;
use merc_lts::LtsBuilderMem;
use merc_lts::StateIndex;
use merc_reduction::Equivalence;
use merc_reduction::compare_lts;
use merc_syntax::random_lps;
use merc_utilities::Timing;
use merc_utilities::random_test;

use merc_lps::Mcrl2MultiActionLabel;
use merc_lps::explore_lps_explicit;

/// Reads the LPS at `lps_path` and preprocesses it exactly as the `merc-lps`
/// tool does, since the explorers themselves take the LPS as given. Each
/// exploration consumes its own LPS, so this is called once per exploration.
fn read_preprocessed_lps(lps_path: &Path) -> LinearProcessSpecification {
    let lps = read_lps(lps_path.to_str().unwrap()).expect("Failed to read LPS");
    preprocess(&lps, &PreprocessOptions::default()).expect("Failed to preprocess LPS")
}

/// Explores `lps_path` both with the plain explicit explorer and with the
/// control-flow-pruning explorer, and asserts the two LTSs have equal
/// state/transition counts and are strongly bisimilar. Pruning summands
/// whose guard cannot hold must never change the explored transition system.
fn assert_cfg_matches_explicit(lps_path: &Path) {
    let mut reference_builder: LtsBuilderMem<Mcrl2MultiActionLabel> = LtsBuilderMem::new(Vec::new(), Vec::new());
    explore_lps_explicit(
        &mut reference_builder,
        read_preprocessed_lps(lps_path),
        CachingStrategy::None,
        ExplorationStrategy::Dfs,
        false,
        &Timing::new(),
    )
    .expect("Explicit exploration failed");
    let reference = reference_builder
        .finish(StateIndex::new(0), false)
        .relabel(|label| Ok(label.to_string()))
        .unwrap();

    let mut cfg_builder: LtsBuilderMem<Mcrl2MultiActionLabel> = LtsBuilderMem::new(Vec::new(), Vec::new());
    explore_lps_explicit(
        &mut cfg_builder,
        read_preprocessed_lps(lps_path),
        CachingStrategy::None,
        ExplorationStrategy::Dfs,
        true,
        &Timing::new(),
    )
    .expect("Control flow exploration failed");
    let cfg = cfg_builder
        .finish(StateIndex::new(0), false)
        .relabel(|label| Ok(label.to_string()))
        .unwrap();

    assert_eq!(
        reference.num_of_states(),
        cfg.num_of_states(),
        "State count mismatch for {}",
        lps_path.display()
    );
    assert_eq!(
        reference.num_of_transitions(),
        cfg.num_of_transitions(),
        "Transition count mismatch for {}",
        lps_path.display()
    );
    assert!(
        compare_lts(Equivalence::StrongBisim, reference, cfg, false, false, &Timing::new()).0,
        "Control flow and explicit LTSs are not strongly bisimilar for {}",
        lps_path.display()
    );
}

/// Runs `mcrl22lps` on a `.mcrl2` specification and asserts that control-flow
/// and plain explicit exploration agree.
fn compare_cfg_with_explicit(spec_relative_path: &str) {
    let Ok(mcrl2_path) = std::env::var("MCRL2_PATH") else {
        println!("Skipping test: MCRL2_PATH not set");
        return;
    };

    let mcrl22lps = Path::new(&mcrl2_path).join("mcrl22lps");
    let spec_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(spec_relative_path);
    assert!(spec_path.exists(), "Spec file not found: {}", spec_path.display());

    let temp_dir = temp_dir("test_cfg_lps").unwrap();
    let lps_path = temp_dir.path().join("spec.lps");

    let status =
        traced_command(Command::new(&mcrl22lps).arg(&spec_path).arg(&lps_path)).expect("Failed to execute mcrl22lps");
    assert!(status.success(), "mcrl22lps failed with status: {status}");

    assert_cfg_matches_explicit(&lps_path);
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cfg_abp() {
    compare_cfg_with_explicit("../../../../examples/mCRL2/academic/abp/abp.mcrl2");
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cfg_cabp() {
    compare_cfg_with_explicit("../../../../examples/mCRL2/academic/cabp/cabp.mcrl2");
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cfg_allow() {
    compare_cfg_with_explicit("../../../../examples/mCRL2/academic/allow/allow.mcrl2");
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cfg_block() {
    compare_cfg_with_explicit("../../../../examples/mCRL2/academic/block/block.mcrl2");
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cfg_dining3() {
    compare_cfg_with_explicit("../../../../examples/mCRL2/academic/dining/dining3.mcrl2");
}

#[test]
#[cfg_attr(miri, ignore)]
fn test_cfg_par() {
    compare_cfg_with_explicit("../../../../examples/mCRL2/academic/par/par.mcrl2");
}

/// Unordered pairs, in which `pair(2, 1) == pair(1, 2)` holds although they are
/// different normal forms, so a summand guarded by `p == pair(1, 2)` can fire
/// in the initial state `pair(2, 1)`.
#[test]
#[cfg_attr(miri, ignore)]
fn test_cfg_user_defined_equality() {
    compare_cfg_with_explicit("tests/upair.mcrl2");
}

/// Generates random LPS specs with [`random_lps`] and asserts that
/// control-flow and plain explicit exploration agree on each.
#[test]
#[cfg_attr(miri, ignore)]
fn test_cfg_random_lps() {
    let Ok(mcrl2_path) = std::env::var("MCRL2_PATH") else {
        println!("Skipping test: MCRL2_PATH not set");
        return;
    };

    let txt2lps = Path::new(&mcrl2_path).join("txt2lps");

    let temp_dir = temp_dir("test_cfg_random_lps").unwrap();
    let spec_path = temp_dir.path().join("spec.mcrl2");
    let lps_path = temp_dir.path().join("spec.lps");

    random_test(20, |rng| {
        let spec = random_lps(rng, 5, 3, 0.4);
        std::fs::write(&spec_path, spec.to_string()).expect("Failed to write random LPS spec");

        let status =
            traced_command(Command::new(&txt2lps).arg(&spec_path).arg(&lps_path)).expect("Failed to execute txt2lps");
        assert!(status.success(), "txt2lps failed with status: {status}");

        // Ensure the generated LPS can be read back before exploring it.
        let _ = File::open(&lps_path).expect("Failed to open generated LPS");
        assert_cfg_matches_explicit(&lps_path);
    });
}
