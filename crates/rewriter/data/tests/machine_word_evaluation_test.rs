use merc_data::DataApplication;
use merc_data::DataExpression;
use merc_data::DataFunctionSymbol;
use merc_data::MachineNumber;
use merc_data::MachineWordOp;
use merc_data::try_evaluate_machine_word;

/// Every operation with its declared name and arity.
const OPERATIONS: [(MachineWordOp, &str, usize); 42] = [
    (MachineWordOp::ZeroWord, "@zero_word", 0),
    (MachineWordOp::OneWord, "@one_word", 0),
    (MachineWordOp::TwoWord, "@two_word", 0),
    (MachineWordOp::ThreeWord, "@three_word", 0),
    (MachineWordOp::FourWord, "@four_word", 0),
    (MachineWordOp::MaxWord, "@max_word", 0),
    (MachineWordOp::SuccWord, "@succ_word", 1),
    (MachineWordOp::PredWord, "@pred_word", 1),
    (MachineWordOp::SqrtWord, "@sqrt_word", 1),
    (MachineWordOp::EqualsZeroWord, "@equals_zero_word", 1),
    (MachineWordOp::NotEqualsZeroWord, "@not_equals_zero_word", 1),
    (MachineWordOp::EqualsOneWord, "@equals_one_word", 1),
    (MachineWordOp::EqualsMaxWord, "@equals_max_word", 1),
    (MachineWordOp::RightmostBit, "@rightmost_bit", 1),
    (MachineWordOp::AddWord, "@add_word", 2),
    (MachineWordOp::AddWithCarryWord, "@add_with_carry_word", 2),
    (MachineWordOp::TimesWord, "@times_word", 2),
    (MachineWordOp::TimesOverflowWord, "@times_overflow_word", 2),
    (MachineWordOp::MinusWord, "@minus_word", 2),
    (MachineWordOp::MonusWord, "@monus_word", 2),
    (MachineWordOp::DivWord, "@div_word", 2),
    (MachineWordOp::ModWord, "@mod_word", 2),
    (MachineWordOp::AddOverflowWord, "@add_overflow_word", 2),
    (
        MachineWordOp::AddWithCarryOverflowWord,
        "@add_with_carry_overflow_word",
        2,
    ),
    (MachineWordOp::Equal, "@equal", 2),
    (MachineWordOp::NotEqual, "@not_equal", 2),
    (MachineWordOp::Less, "@less", 2),
    (MachineWordOp::LessEqual, "@less_equal", 2),
    (MachineWordOp::Greater, "@greater", 2),
    (MachineWordOp::GreaterEqual, "@greater_equal", 2),
    (MachineWordOp::ShiftRight, "@shift_right", 2),
    (MachineWordOp::SqrtDoubleword, "@sqrt_doubleword", 2),
    (MachineWordOp::TimesWithCarryWord, "@times_with_carry_word", 3),
    (
        MachineWordOp::TimesWithCarryOverflowWord,
        "@times_with_carry_overflow_word",
        3,
    ),
    (MachineWordOp::DivDoubleword, "@div_doubleword", 3),
    (MachineWordOp::ModDoubleword, "@mod_doubleword", 3),
    (MachineWordOp::SqrtTripleword, "@sqrt_tripleword", 3),
    (MachineWordOp::SqrtTriplewordOverflow, "@sqrt_tripleword_overflow", 3),
    (MachineWordOp::DivDoubleDoubleword, "@div_double_doubleword", 4),
    (MachineWordOp::SqrtQuadrupleword, "@sqrt_quadrupleword", 4),
    (
        MachineWordOp::SqrtQuadruplewordOverflow,
        "@sqrt_quadrupleword_overflow",
        4,
    ),
    (MachineWordOp::DivTripleDoubleword, "@div_triple_doubleword", 5),
];

/// A nullary function symbol, e.g. a `Bool` literal or an unevaluated operand.
fn symbol(name: &str) -> DataExpression {
    DataFunctionSymbol::new(name).into()
}

fn word(value: u64) -> DataExpression {
    MachineNumber::new(value).into()
}

fn bool_literal(value: bool) -> DataExpression {
    DataExpression::bool_literal(value)
}

/// `op` applied to `args`; a nullary operation is the bare function symbol.
fn apply(op: MachineWordOp, args: &[DataExpression]) -> DataExpression {
    if args.is_empty() {
        symbol(op.name())
    } else {
        DataApplication::with_args(&DataFunctionSymbol::new(op.name()), args).into()
    }
}

fn evaluate(op: MachineWordOp, args: &[DataExpression]) -> Option<DataExpression> {
    try_evaluate_machine_word(&apply(op, args))
}

/// Arguments of the right sort for `op`, with every word equal to `value`.
fn arguments(op: MachineWordOp, arity: usize, value: u64) -> Vec<DataExpression> {
    (0..arity)
        .map(|index| {
            if op == MachineWordOp::ShiftRight && index == 0 {
                bool_literal(false)
            } else {
                word(value)
            }
        })
        .collect()
}

#[test]
fn test_name_arity_and_from_name_agree_for_every_operation() {
    for (op, name, arity) in OPERATIONS {
        assert_eq!(op.name(), name);
        assert_eq!(op.arity(), arity, "{name}");
        assert_eq!(MachineWordOp::from_name(name), Some(op));
    }

    // `@`-prefixed symbols that are governed by rewrite rules are not operations.
    assert_eq!(MachineWordOp::from_name("@cNat"), None);
    assert_eq!(MachineWordOp::from_name("@cDub"), None);
    assert_eq!(MachineWordOp::from_name("f"), None);
    assert_eq!(MachineWordOp::from_name(""), None);
}

/// Every operation evaluates exactly when it gets `arity` concrete arguments.
#[test]
fn test_every_operation_evaluates_only_at_its_arity() {
    for (op, name, arity) in OPERATIONS {
        assert!(
            evaluate(op, &arguments(op, arity, 1)).is_some(),
            "{name} with {arity} arguments"
        );
        assert_eq!(
            op.evaluate(arguments(op, arity, 1).iter().map(|arg| arg.copy())),
            evaluate(op, &arguments(op, arity, 1))
        );

        if arity > 0 {
            assert_eq!(
                evaluate(op, &arguments(op, arity - 1, 1)),
                None,
                "{name} with {} arguments",
                arity - 1
            );
        }
        // `@shift_right` reads its two arguments and ignores any further ones, and more than five
        // word arguments are out of range for `evaluate`.
        if op != MachineWordOp::ShiftRight && arity < 5 {
            assert_eq!(
                evaluate(op, &arguments(op, arity + 1, 1)),
                None,
                "{name} with {} arguments",
                arity + 1
            );
        }
    }
}

#[test]
fn test_constants() {
    assert_eq!(evaluate(MachineWordOp::ZeroWord, &[]), Some(word(0)));
    assert_eq!(evaluate(MachineWordOp::OneWord, &[]), Some(word(1)));
    assert_eq!(evaluate(MachineWordOp::TwoWord, &[]), Some(word(2)));
    assert_eq!(evaluate(MachineWordOp::ThreeWord, &[]), Some(word(3)));
    assert_eq!(evaluate(MachineWordOp::FourWord, &[]), Some(word(4)));
    assert_eq!(evaluate(MachineWordOp::MaxWord, &[]), Some(word(u64::MAX)));
}

/// Every word-valued operation is dispatched to its own implementation, with arguments for which
/// the operations of the same arity give different results.
#[test]
fn test_word_valued_operations() {
    let cases: [(MachineWordOp, &[u64], u64); 22] = [
        (MachineWordOp::SuccWord, &[41], 42),
        (MachineWordOp::PredWord, &[41], 40),
        (MachineWordOp::SqrtWord, &[50], 7),
        (MachineWordOp::AddWord, &[17, 5], 22),
        (MachineWordOp::AddWithCarryWord, &[17, 5], 23),
        (MachineWordOp::TimesWord, &[17, 5], 85),
        (MachineWordOp::TimesOverflowWord, &[u64::MAX, 4], 3),
        (MachineWordOp::MinusWord, &[17, 5], 12),
        (MachineWordOp::MonusWord, &[5, 17], 0),
        (MachineWordOp::DivWord, &[17, 5], 3),
        (MachineWordOp::ModWord, &[17, 5], 2),
        (MachineWordOp::SqrtDoubleword, &[1, 0], 1 << 32),
        (MachineWordOp::TimesWithCarryWord, &[17, 5, 3], 88),
        (MachineWordOp::TimesWithCarryOverflowWord, &[u64::MAX, 4, 4], 4),
        // (2^64 * 1 + 0) div 2 == 2^63.
        (MachineWordOp::DivDoubleword, &[1, 0, 2], 1 << 63),
        (MachineWordOp::ModDoubleword, &[1, 1, 2], 1),
        // sqrt(2^128) == 2^64: low word 0, high word 1.
        (MachineWordOp::SqrtTripleword, &[1, 0, 0], 0),
        (MachineWordOp::SqrtTriplewordOverflow, &[1, 0, 0], 1),
        (MachineWordOp::DivDoubleDoubleword, &[1, 0, 0, 2], 1 << 63),
        (MachineWordOp::SqrtQuadrupleword, &[0, 0, 0, 49], 7),
        // sqrt(2^192) == 2^96, whose most-significant word is 2^32.
        (MachineWordOp::SqrtQuadruplewordOverflow, &[1, 0, 0, 0], 1 << 32),
        (MachineWordOp::DivTripleDoubleword, &[0, 1, 0, 0, 2], 1 << 63),
    ];

    for (op, args, expected) in cases {
        let args: Vec<DataExpression> = args.iter().map(|value| word(*value)).collect();
        assert_eq!(evaluate(op, &args), Some(word(expected)), "{}", op.name());
    }

    // Wrapping semantics.
    assert_eq!(
        evaluate(MachineWordOp::AddWord, &[word(u64::MAX), word(1)]),
        Some(word(0))
    );
    assert_eq!(evaluate(MachineWordOp::SuccWord, &[word(u64::MAX)]), Some(word(0)));
    assert_eq!(evaluate(MachineWordOp::PredWord, &[word(0)]), Some(word(u64::MAX)));
    assert_eq!(
        evaluate(MachineWordOp::MinusWord, &[word(0), word(1)]),
        Some(word(u64::MAX))
    );
}

#[test]
fn test_bool_valued_operations() {
    let cases: [(MachineWordOp, &[u64], bool); 20] = [
        (MachineWordOp::EqualsZeroWord, &[0], true),
        (MachineWordOp::EqualsZeroWord, &[1], false),
        (MachineWordOp::NotEqualsZeroWord, &[0], false),
        (MachineWordOp::EqualsOneWord, &[1], true),
        (MachineWordOp::EqualsOneWord, &[0], false),
        (MachineWordOp::EqualsMaxWord, &[u64::MAX], true),
        (MachineWordOp::RightmostBit, &[3], true),
        (MachineWordOp::RightmostBit, &[2], false),
        (MachineWordOp::AddOverflowWord, &[u64::MAX, 1], true),
        (MachineWordOp::AddOverflowWord, &[u64::MAX, 0], false),
        (MachineWordOp::AddWithCarryOverflowWord, &[u64::MAX, 0], true),
        (MachineWordOp::Equal, &[7, 7], true),
        (MachineWordOp::Equal, &[7, 8], false),
        (MachineWordOp::NotEqual, &[7, 8], true),
        (MachineWordOp::Less, &[1, 2], true),
        (MachineWordOp::Less, &[2, 2], false),
        (MachineWordOp::LessEqual, &[2, 2], true),
        (MachineWordOp::Greater, &[2, 2], false),
        (MachineWordOp::Greater, &[3, 2], true),
        (MachineWordOp::GreaterEqual, &[2, 2], true),
    ];

    for (op, args, expected) in cases {
        let args: Vec<DataExpression> = args.iter().map(|value| word(*value)).collect();
        assert_eq!(
            evaluate(op, &args),
            Some(bool_literal(expected)),
            "{} {args:?}",
            op.name()
        );
    }
}

#[test]
fn test_shift_right_takes_a_bool_argument() {
    // @shift_right(false, 0b101) == 0b10.
    assert_eq!(
        evaluate(MachineWordOp::ShiftRight, &[bool_literal(false), word(0b101)]),
        Some(word(0b10))
    );
    // @shift_right(true, 0) inserts the new most-significant bit.
    assert_eq!(
        evaluate(MachineWordOp::ShiftRight, &[bool_literal(true), word(0)]),
        Some(word(1u64 << 63))
    );
    // The literal is recognised by name, whatever its sort.
    assert_eq!(
        evaluate(MachineWordOp::ShiftRight, &[symbol("true"), word(0b100)]),
        Some(word((1u64 << 63) | 0b10))
    );
}

#[test]
fn test_non_word_operations_are_ignored() {
    // Not a @word operation.
    assert_eq!(
        try_evaluate_machine_word(
            &DataApplication::with_args(&DataFunctionSymbol::new("f"), &[word(1), word(2)]).into()
        ),
        None
    );
    // @-prefixed, but not a machine-word operation.
    assert_eq!(
        try_evaluate_machine_word(&DataApplication::with_args(&DataFunctionSymbol::new("@cNat"), &[word(1)]).into()),
        None
    );
    // A machine number is a value, not an operation to evaluate.
    assert_eq!(try_evaluate_machine_word(&word(5)), None);
}

#[test]
fn test_non_concrete_arguments_are_ignored() {
    // The second argument is not a machine number, so no native reduction happens.
    assert_eq!(evaluate(MachineWordOp::AddWord, &[word(3), symbol("x")]), None);
    // @shift_right with a non-Bool first argument, or a non-word second argument.
    assert_eq!(evaluate(MachineWordOp::ShiftRight, &[word(1), word(4)]), None);
    assert_eq!(
        evaluate(MachineWordOp::ShiftRight, &[bool_literal(true), symbol("x")]),
        None
    );
}
