use std::fs::File;
use std::path::Path;
use std::process::Command;

use merc_io::DumpFiles;
use merc_lts::mutate_lts;
use merc_lts::random_lts;
use merc_lts::write_mcrl2_aut;
use merc_refinement::ExplorationStrategy;
use merc_refinement::RefinementType;
use merc_refinement::refines;
use merc_utilities::Timing;
use merc_utilities::random_test;

/// Compares our refinement checks against mCRL2's `ltscompare` on random inputs.
fn test_mcrl2_ltscompare_refinement(name: &str, refinement: RefinementType, argument: &str) {
    let Ok(mcrl2_path) = std::env::var("MCRL2_PATH") else {
        println!("Skipping test: MCRL2_PATH not set");
        return;
    };

    let ltscompare = Path::new(&mcrl2_path).join("ltscompare");

    // Write the random LTSs to temporary files for ltscompare to process.
    let temp_dir = tempfile::tempdir().unwrap();
    let impl_path = temp_dir.path().join("impl.aut");
    let spec_path = temp_dir.path().join("spec.aut");

    random_test(100, |rng| {
        let files = DumpFiles::new(name);

        let spec_lts = random_lts::<String, _>(rng, 1000, 3);
        let impl_lts = mutate_lts(&spec_lts, rng, 100).unwrap();

        write_mcrl2_aut(&mut File::create(&impl_path).unwrap(), &impl_lts).unwrap();
        files
            .dump("impl.aut", |writer| write_mcrl2_aut(writer, &impl_lts))
            .unwrap();

        write_mcrl2_aut(&mut File::create(&spec_path).unwrap(), &spec_lts).unwrap();
        files
            .dump("spec.aut", |writer| write_mcrl2_aut(writer, &spec_lts))
            .unwrap();

        let process = Command::new(&ltscompare)
            .arg(format!("-p{}", argument))
            .arg(&impl_path)
            .arg(&spec_path)
            .output()
            .expect("Failed to execute ltscompare");

        assert!(
            process.status.success(),
            "ltscompare failed with status: {}",
            process.status
        );

        let expected_result = String::from_utf8_lossy(&process.stdout).contains("true");
        let result = refines(
            impl_lts,
            spec_lts,
            refinement,
            ExplorationStrategy::BFS,
            false,
            false,
            &mut Timing::new(),
        )
        .0;

        assert_eq!(
            expected_result, result,
            "Mismatch between ltscompare and our implementation"
        );
    });
}

#[test]
#[cfg_attr(miri, ignore)] // Miri is too slow
fn test_mcrl2_ltscompare_trace() {
    test_mcrl2_ltscompare_refinement("test_mcrl2_ltscompare_trace", RefinementType::Trace, "trace-ac");
}

#[test]
#[cfg_attr(miri, ignore)] // Miri is too slow
fn test_mcrl2_ltscompare_weak_trace() {
    test_mcrl2_ltscompare_refinement(
        "test_mcrl2_ltscompare_weak_trace",
        RefinementType::Weaktrace,
        "weak-trace-ac",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Miri is too slow
fn test_mcrl2_ltscompare_stable_failures() {
    test_mcrl2_ltscompare_refinement(
        "test_mcrl2_ltscompare_stable_failures",
        RefinementType::StableFailures,
        "weak-failures",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Miri is too slow
fn test_mcrl2_ltscompare_failures_divergences() {
    test_mcrl2_ltscompare_refinement(
        "test_mcrl2_ltscompare_failures_divergences",
        RefinementType::FailuresDivergences,
        "failures-divergence",
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Miri is too slow
fn test_mcrl2_ltscompare_impossible_futures() {
    test_mcrl2_ltscompare_refinement(
        "test_mcrl2_ltscompare_impossible_futures",
        RefinementType::ImpossibleFutures,
        "impossible-futures",
    );
}
