//! Integration tests for [`Permutation`], exercising only its public API.
//!
//! `Permutation`'s backing `mapping` field is private, so equality between
//! two permutations (available via its derived `PartialEq`) stands in for
//! direct field comparisons.

use rand::RngExt;
use rand::seq::IteratorRandom;
use rand::seq::SliceRandom;

use merc_utilities::random_test;

use merc_pbes::Permutation;
use merc_pbes::permutation::permutation_group;
use merc_pbes::permutation::permutation_group_size;

/// `permutation_group_size` computes `n!` with `saturating_mul` rather than
/// `.product()`, since `.product()` overflows `usize` for `n >= 21`
/// (21! > `usize::MAX` on 64-bit). Large inputs must therefore return
/// `usize::MAX` rather than panicking in debug builds or wrapping silently
/// in release.
#[test]
fn permutation_group_size_saturates_instead_of_overflowing() {
    assert_eq!(permutation_group_size(0), 1, "0! = 1");
    assert_eq!(permutation_group_size(1), 1, "1! = 1");
    assert_eq!(permutation_group_size(3), 6, "3! = 6");
    assert_eq!(permutation_group_size(5), 120, "5! = 120");
    // 21! = 51_090_942_171_709_440_000 which exceeds u64::MAX, so on any
    // 64-bit platform this saturates to usize::MAX.
    assert_eq!(
        permutation_group_size(21),
        usize::MAX,
        "21! must saturate to usize::MAX rather than overflow"
    );
}

#[test]
fn test_permutation_from_input() {
    let permutation = Permutation::from_mapping_notation("[0->   2, 1   ->0, 2->1]").unwrap();

    assert_eq!(permutation, Permutation::from_mapping(vec![(0, 2), (1, 0), (2, 1)]));
}

#[test]
fn test_cycle_notation() {
    let permutation = Permutation::from_mapping_notation("[0->2, 1->0, 2->1, 3->4, 4->3]").unwrap();

    assert_eq!(permutation.to_string(), "(0 2 1)(3 4)");
}

#[test]
fn test_cycle_notation_parsing() {
    let permutation = Permutation::from_cycle_notation("(0 2 1)(3 4)").unwrap();

    assert_eq!(
        permutation,
        Permutation::from_mapping(vec![(0, 2), (1, 0), (2, 1), (3, 4), (4, 3)])
    );
}

#[test]
fn test_permutation_group() {
    let indices = vec![0, 3, 5];
    let permutations: Vec<Permutation> = permutation_group(indices.clone()).collect();
    for p in &permutations {
        println!("{}", p);
    }

    assert_eq!(permutations.len(), permutation_group_size(indices.len()));
}

#[test]
fn test_random_cycle_notation() {
    random_test(100, |rng| {
        // Pick a random subset size >= 2 to allow a derangement.
        let m = rng.random_range(2..10);

        // Choose a random subset of distinct domain elements.
        let domain: Vec<usize> = (0..10).sample(rng, m);

        // Create a random derangement of the chosen domain.
        let mut image = domain.clone();
        image.shuffle(rng);

        let mapping: Vec<(usize, usize)> = domain.into_iter().zip(image).filter(|(x, y)| x != y).collect();
        println!("Mapping: {:?}", mapping);

        let permutation = Permutation::from_mapping(mapping.clone());

        let cycle_notation = permutation.to_string();
        let parsed_permutation = Permutation::from_cycle_notation(&cycle_notation).unwrap();

        assert_eq!(
            permutation, parsed_permutation,
            "Failed on permutation {:?}",
            permutation
        );
    })
}

#[test]
fn test_random_mapping_notation() {
    random_test(100, |rng| {
        // Pick a random subset size >= 2 to allow a derangement.
        let m = rng.random_range(2..10);

        // Choose a random subset of distinct domain elements.
        let domain: Vec<usize> = (0..10).sample(rng, m);

        // Create a random derangement of the chosen domain.
        let mut image = domain.clone();
        image.shuffle(rng);

        let mapping: Vec<(usize, usize)> = domain.into_iter().zip(image).filter(|(x, y)| x != y).collect();
        println!("Mapping: {:?}", mapping);

        let permutation = Permutation::from_mapping(mapping.clone());

        let mapping_notation = format!("{:?}", permutation);
        let parsed_permutation = Permutation::from_mapping_notation(&mapping_notation).unwrap();

        assert_eq!(
            permutation, parsed_permutation,
            "Failed on permutation {:?}",
            permutation
        );
    })
}

/// `from_cycle_notation` must not choke on a singleton cycle such as `(5)`,
/// which is ordinary permutation-cycle notation for "5 is a fixed point" and
/// is exactly the kind of string a user passes on the `merc-pbes` CLI's
/// `--generators` flag (see `parse_generators` in `tools/mcrl2/pbes/src/main.rs`,
/// which calls `Permutation::from_cycle_notation` directly on unvalidated
/// user input). `from_cycle_notation` builds a `(5, 5)` identity pair for a
/// singleton cycle and passes it straight to `Permutation::from_mapping`,
/// whose `debug_assert!` requires the mapping to contain *no* identity
/// pairs -- so in a debug build this panics instead of returning either an
/// identity-free `Permutation` or a parse error.
#[test]
fn from_cycle_notation_accepts_singleton_cycle_without_panicking() {
    let permutation = Permutation::from_cycle_notation("(5)").expect("singleton cycle should parse");
    assert!(
        permutation.is_identity(),
        "a singleton cycle names a fixed point and must produce the identity permutation"
    );
}

/// Same defect as `from_cycle_notation_accepts_singleton_cycle_without_panicking`, reached
/// through the other public string parser: `"[5->5]"` is a syntactically valid (if trivial)
/// mapping-notation permutation naming a fixed point, and `--generators` accepts either
/// notation (see `parse_generators` in `tools/mcrl2/pbes/src/main.rs`). `is_valid_permutation`
/// only checks bijectivity, so it does not reject the identity pair either, and the same
/// `debug_assert!` in `Permutation::from_mapping` panics.
#[test]
fn from_mapping_notation_accepts_identity_pair_without_panicking() {
    let permutation = Permutation::from_mapping_notation("[5->5]").expect("identity pair should parse");
    assert!(
        permutation.is_identity(),
        "a [5->5] pair names a fixed point and must produce the identity permutation"
    );
}
