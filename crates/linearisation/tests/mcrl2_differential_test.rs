//! Compares `linearise` against mCRL2's `mcrl22lps`: both LPSs are explored with mCRL2's `lps2lts`,
//! and the resulting state spaces must be strongly bisimilar.
//!
//! The test needs an mCRL2 installation and is skipped unless `MERC_MCRL2_BIN` is set to the
//! directory containing `mcrl22lps`, `lps2lts` and `ltscompare`.

use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use rand::RngExt;

use merc_linearisation::LinearisationOptions;
use merc_syntax::RandomLinearisationOptions;
use merc_syntax::SourceMap;
use merc_syntax::UntypedProcessSpecification;
use merc_syntax::random_linearisation_specification;
use merc_typecheck::NumberEncoding;
use merc_typecheck::ProcessSpecification;
use merc_utilities::random_test;

fn mcrl2_bin() -> Option<PathBuf> {
    std::env::var_os("MERC_MCRL2_BIN").map(PathBuf::from)
}

#[track_caller]
fn run(bin: &Path, tool: &str, args: &[&Path]) -> String {
    let output = Command::new(bin.join(tool))
        .args(args)
        .output()
        .unwrap_or_else(|error| panic!("cannot run {tool}: {error}"));
    assert!(
        output.status.success(),
        "{tool} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr)
}

/// Asserts that merc and mCRL2 linearise `text` to bisimilar processes.
#[track_caller]
fn assert_equivalent(bin: &Path, text: &str) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = |name: &str| directory.path().join(name);

    std::fs::write(path("spec.mcrl2"), text).expect("the specification can be written");
    run(bin, "mcrl22lps", &[&path("spec.mcrl2"), &path("mcrl2.lps")]);
    run(bin, "lps2lts", &[&path("mcrl2.lps"), &path("mcrl2.aut")]);

    // mCRL2 represents numbers by machine words, so merc must use the same encoding.
    let untyped = UntypedProcessSpecification::parse(text).expect("the specification parses");
    let spec = ProcessSpecification::from_untyped_with(untyped, NumberEncoding::MachineWord, &mut SourceMap::new())
        .and_then(|mut spec| spec.lower())
        .expect("the specification type checks");
    let lps = merc_linearisation::linearise(&spec, merc_linearisation::rewriter, &LinearisationOptions::default())
        .unwrap_or_else(|error| panic!("linearisation failed:\n{text}\nerror: {error}"));
    let file = std::fs::File::create(path("merc.lps")).expect("the LPS file can be created");
    merc_lps::write_lps(&lps, file).expect("the LPS can be written");
    run(bin, "lps2lts", &[&path("merc.lps"), &path("merc.aut")]);

    let comparison = run(
        bin,
        "ltscompare",
        &[Path::new("-ebisim"), &path("mcrl2.aut"), &path("merc.aut")],
    );
    assert!(
        comparison.contains("true"),
        "the state spaces are not bisimilar:\n{text}\nmerc's LPS:\n{lps}\n{comparison}"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Runs external tools
fn test_handwritten_specifications_match_mcrl2() {
    let Some(bin) = mcrl2_bin() else {
        return;
    };
    for text in [
        "act a; init a;",
        "act a; init a . delta;",
        "act a, b; proc P(n: Nat) = (n < 3) -> a . P(n + 1) <> b . delta; init P(0);",
        "act a, b; proc P(x: Bool) = a . Q(!x); Q(y: Bool) = b . P(y); init P(true);",
        "act a, b, c; proc P = a . Q . c . P; Q = b + a . Q; init P;",
        "act a, b; init a || b;",
        "act a, b; init allow({a|b}, a || b);",
        "act a, b; init hide({a}, a . b . delta);",
        "act a: Bool; b; proc P(x: Bool) = (sum x: Bool . a(x)) . Q(x); Q(y: Bool) = b . P(!y); init P(true);",
        "act s, r, c: Bool; a; proc S = sum x: Bool . s(x) . a . S; R = sum y: Bool . r(y) . R;
         init allow({c, a}, comm({s|r -> c}, S || R));",
        "act a, b, c; proc P = a . b . P; Q = b . a . Q;
         init block({a, b}, comm({a|b -> c}, P || Q || P));",
        "act a, b, c, d; proc P = a . b . c . P;
         init allow({d, c|b}, hide({a}, rename({b -> d}, P || P)));",
    ] {
        assert_equivalent(&bin, text);
    }
}

#[test]
#[cfg_attr(miri, ignore)] // Runs external tools
fn test_random_specifications_match_mcrl2() {
    let Some(bin) = mcrl2_bin() else {
        return;
    };
    random_test(100, |rng| {
        let options = RandomLinearisationOptions {
            equation_count: rng.random_range(1..=4),
            depth: rng.random_range(1..=4),
            components: rng.random_range(1..=3),
            operators: rng.random_range(0..=3),
        };
        let spec = random_linearisation_specification(rng, options);
        assert_equivalent(&bin, &spec.to_string());
    });
}
