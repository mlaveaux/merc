//! Tests for the [`NumberEncoding`] option, which selects both the system
//! specification pulled in for the numeric sorts and the representation numeric
//! literals are lowered to.

use merc_syntax::SourceMap;
use merc_syntax::UntypedDataSpecification;
use merc_typecheck::DataSpecification;
use merc_typecheck::NumberEncoding;

/// Type checks `text` under `encoding`.
#[track_caller]
fn typed(text: &str, encoding: NumberEncoding) -> DataSpecification {
    let untyped = UntypedDataSpecification::parse(text).expect("the specification should parse");
    DataSpecification::from_untyped_with(untyped, encoding, &mut SourceMap::new())
        .unwrap_or_else(|error| panic!("{encoding:?} should type check:\n{text}\nerror: {error:?}"))
}

/// The right-hand side of the `q = ...` equation, as lowered under `encoding`.
#[track_caller]
fn lowered_rhs(expr: &str, sort: &str, encoding: NumberEncoding) -> String {
    let text = format!("map q: {sort};\neqn q = {expr};");
    let spec = typed(&text, encoding);
    let lowered = spec.lower_data_specification();
    lowered
        .equations()
        .iter()
        .find(|equation| equation.lhs().to_string() == "q")
        .expect("the q equation should be lowered")
        .rhs()
        .to_string()
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_binary_is_the_default() {
    assert_eq!(NumberEncoding::default(), NumberEncoding::Binary);
    assert!(!NumberEncoding::Binary.is_machine_word());
    assert!(NumberEncoding::MachineWord.is_machine_word());

    // `from_untyped` keeps using the default encoding.
    let spec = DataSpecification::from_untyped(UntypedDataSpecification::parse("map f: Nat;").unwrap()).unwrap();
    assert_eq!(spec.number_encoding(), NumberEncoding::Binary);
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_encoding_is_recorded() {
    for encoding in [NumberEncoding::Binary, NumberEncoding::MachineWord] {
        assert_eq!(typed("map f: Nat;", encoding).number_encoding(), encoding);
    }
}

/// Every numeric and container sort type checks under both encodings.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_both_encodings_type_check_the_standard_sorts() {
    let specifications = [
        "map f: Pos -> Pos;\nvar p: Pos;\neqn f(p) = p * 2;",
        "map f: Nat -> Nat;\nvar n: Nat;\neqn f(n) = n + 1;",
        "map f: Int -> Int;\nvar i: Int;\neqn f(i) = i - 3;",
        "map f: Real -> Real;\nvar r: Real;\neqn f(r) = r + 1;",
        "map f: List(Nat) -> Nat;\nvar s: List(Nat);\neqn f(s) = #s;",
    ];

    for text in specifications {
        for encoding in [NumberEncoding::Binary, NumberEncoding::MachineWord] {
            let spec = typed(text, encoding);
            // Lowering must succeed too — the system equations of the selected
            // templates are lowered alongside the user's.
            assert!(
                !spec.lower_data_specification().equations().is_empty(),
                "{encoding:?} lowered no equations for:\n{text}"
            );
        }
    }
}

/// The machine-word encoding pulls in `machine_word.mcrl2`, which declares the
/// `@word` digit sort; the binary encoding has no such sort.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_system_specification_differs_per_encoding() {
    let declares_word = |encoding| {
        typed("map f: Nat;", encoding)
            .system_defined_specification()
            .sort_declarations
            .iter()
            .any(|declaration| declaration.identifier == "@word")
    };

    assert!(
        !declares_word(NumberEncoding::Binary),
        "the binary encoding has no @word"
    );
    assert!(
        declares_word(NumberEncoding::MachineWord),
        "the machine-word encoding declares @word"
    );
}

/// The machine-word system specification is strictly larger: the `*64`
/// templates define the digit operations on top of the same interface.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_machine_word_specification_has_more_equations() {
    let count = |encoding| {
        typed("map f: Nat;", encoding)
            .system_defined_specification()
            .equation_declarations
            .iter()
            .map(|block| block.equations.len())
            .sum::<usize>()
    };

    assert!(
        count(NumberEncoding::MachineWord) > count(NumberEncoding::Binary),
        "expected the machine-word specification to define more equations"
    );
}

#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_literals_lower_to_the_selected_representation() {
    // Zero infers as `Nat` directly, so it is lowered without a coercion.
    assert_eq!(lowered_rhs("0", "Nat", NumberEncoding::Binary), "@c0");
    assert_eq!(
        lowered_rhs("0", "Nat", NumberEncoding::MachineWord),
        "@most_significant_digitNat(0)"
    );

    // A positive literal infers as `Pos`, but a *bare* literal widened to `Nat`
    // is rebuilt at `Nat` rather than wrapped in the encoding's Pos-to-Nat
    // conversion.
    assert_eq!(
        lowered_rhs("5", "Nat", NumberEncoding::Binary),
        "@cNat(@cDub(true, @cDub(false, @c1)))"
    );
    assert_eq!(
        lowered_rhs("5", "Nat", NumberEncoding::MachineWord),
        "@most_significant_digitNat(5)"
    );

    // 2^64 is the first literal needing two digits.
    assert_eq!(
        lowered_rhs("18446744073709551616", "Nat", NumberEncoding::MachineWord),
        "@concat_digit(@most_significant_digitNat(1), 0)"
    );
}

/// A `Pos` literal used where a `Nat` is expected is widened with the
/// constructor of the selected encoding.
#[test]
#[cfg_attr(miri, ignore)] // Test is too slow under miri
fn test_pos_to_nat_coercion_follows_the_encoding() {
    assert_eq!(
        lowered_rhs("1 + 1", "Nat", NumberEncoding::Binary),
        "@cNat(+(@c1, @c1))"
    );
    assert_eq!(
        lowered_rhs("1 + 1", "Nat", NumberEncoding::MachineWord),
        "Pos2Nat(+(@most_significant_digit(1), @most_significant_digit(1)))"
    );
}
