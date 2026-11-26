use merc_number::machine_word as mw;

use crate::DataExpression;
use crate::DataExpressionKind;
use crate::DataExpressionRef;
use crate::MachineNumber;

/// A machine-word operation, resolved from the name of a `@`-prefixed function
/// symbol. Every such operation is `defined_by_code`: it has no rewrite rules
/// and is instead evaluated natively by [`MachineWordOp::evaluate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MachineWordOp {
    ZeroWord,
    OneWord,
    TwoWord,
    ThreeWord,
    FourWord,
    MaxWord,
    SuccWord,
    PredWord,
    SqrtWord,
    EqualsZeroWord,
    NotEqualsZeroWord,
    EqualsOneWord,
    EqualsMaxWord,
    RightmostBit,
    AddWord,
    AddWithCarryWord,
    TimesWord,
    TimesOverflowWord,
    MinusWord,
    MonusWord,
    DivWord,
    ModWord,
    AddOverflowWord,
    AddWithCarryOverflowWord,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    TimesWithCarryWord,
    TimesWithCarryOverflowWord,
    DivDoubleword,
    ModDoubleword,
    SqrtDoubleword,
    SqrtTripleword,
    SqrtTriplewordOverflow,
    DivDoubleDoubleword,
    SqrtQuadrupleword,
    SqrtQuadruplewordOverflow,
    DivTripleDoubleword,
    ShiftRight,
}

impl MachineWordOp {
    /// Resolves a function symbol name to the machine-word operation it
    /// denotes. Called once per distinct symbol, at rewriter-construction
    /// time — never in the per-term hot path.
    ///
    /// Returns `None` for ordinary, rule-governed symbols, including other
    /// `@`-prefixed ones such as `@cNat`/`@cDub`.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "@zero_word" => Self::ZeroWord,
            "@one_word" => Self::OneWord,
            "@two_word" => Self::TwoWord,
            "@three_word" => Self::ThreeWord,
            "@four_word" => Self::FourWord,
            "@max_word" => Self::MaxWord,
            "@succ_word" => Self::SuccWord,
            "@pred_word" => Self::PredWord,
            "@sqrt_word" => Self::SqrtWord,
            "@equals_zero_word" => Self::EqualsZeroWord,
            "@not_equals_zero_word" => Self::NotEqualsZeroWord,
            "@equals_one_word" => Self::EqualsOneWord,
            "@equals_max_word" => Self::EqualsMaxWord,
            "@rightmost_bit" => Self::RightmostBit,
            "@add_word" => Self::AddWord,
            "@add_with_carry_word" => Self::AddWithCarryWord,
            "@times_word" => Self::TimesWord,
            "@times_overflow_word" => Self::TimesOverflowWord,
            "@minus_word" => Self::MinusWord,
            "@monus_word" => Self::MonusWord,
            "@div_word" => Self::DivWord,
            "@mod_word" => Self::ModWord,
            "@add_overflow_word" => Self::AddOverflowWord,
            "@add_with_carry_overflow_word" => Self::AddWithCarryOverflowWord,
            "@equal" => Self::Equal,
            "@not_equal" => Self::NotEqual,
            "@less" => Self::Less,
            "@less_equal" => Self::LessEqual,
            "@greater" => Self::Greater,
            "@greater_equal" => Self::GreaterEqual,
            "@times_with_carry_word" => Self::TimesWithCarryWord,
            "@times_with_carry_overflow_word" => Self::TimesWithCarryOverflowWord,
            "@div_doubleword" => Self::DivDoubleword,
            "@mod_doubleword" => Self::ModDoubleword,
            "@sqrt_doubleword" => Self::SqrtDoubleword,
            "@sqrt_tripleword" => Self::SqrtTripleword,
            "@sqrt_tripleword_overflow" => Self::SqrtTriplewordOverflow,
            "@div_double_doubleword" => Self::DivDoubleDoubleword,
            "@sqrt_quadrupleword" => Self::SqrtQuadrupleword,
            "@sqrt_quadrupleword_overflow" => Self::SqrtQuadruplewordOverflow,
            "@div_triple_doubleword" => Self::DivTripleDoubleword,
            "@shift_right" => Self::ShiftRight,
            _ => return None,
        })
    }

    /// Returns the declared name of this operation, the inverse of
    /// [`MachineWordOp::from_name`].
    pub fn name(self) -> &'static str {
        match self {
            Self::ZeroWord => "@zero_word",
            Self::OneWord => "@one_word",
            Self::TwoWord => "@two_word",
            Self::ThreeWord => "@three_word",
            Self::FourWord => "@four_word",
            Self::MaxWord => "@max_word",
            Self::SuccWord => "@succ_word",
            Self::PredWord => "@pred_word",
            Self::SqrtWord => "@sqrt_word",
            Self::EqualsZeroWord => "@equals_zero_word",
            Self::NotEqualsZeroWord => "@not_equals_zero_word",
            Self::EqualsOneWord => "@equals_one_word",
            Self::EqualsMaxWord => "@equals_max_word",
            Self::RightmostBit => "@rightmost_bit",
            Self::AddWord => "@add_word",
            Self::AddWithCarryWord => "@add_with_carry_word",
            Self::TimesWord => "@times_word",
            Self::TimesOverflowWord => "@times_overflow_word",
            Self::MinusWord => "@minus_word",
            Self::MonusWord => "@monus_word",
            Self::DivWord => "@div_word",
            Self::ModWord => "@mod_word",
            Self::AddOverflowWord => "@add_overflow_word",
            Self::AddWithCarryOverflowWord => "@add_with_carry_overflow_word",
            Self::Equal => "@equal",
            Self::NotEqual => "@not_equal",
            Self::Less => "@less",
            Self::LessEqual => "@less_equal",
            Self::Greater => "@greater",
            Self::GreaterEqual => "@greater_equal",
            Self::TimesWithCarryWord => "@times_with_carry_word",
            Self::TimesWithCarryOverflowWord => "@times_with_carry_overflow_word",
            Self::DivDoubleword => "@div_doubleword",
            Self::ModDoubleword => "@mod_doubleword",
            Self::SqrtDoubleword => "@sqrt_doubleword",
            Self::SqrtTripleword => "@sqrt_tripleword",
            Self::SqrtTriplewordOverflow => "@sqrt_tripleword_overflow",
            Self::DivDoubleDoubleword => "@div_double_doubleword",
            Self::SqrtQuadrupleword => "@sqrt_quadrupleword",
            Self::SqrtQuadruplewordOverflow => "@sqrt_quadrupleword_overflow",
            Self::DivTripleDoubleword => "@div_triple_doubleword",
            Self::ShiftRight => "@shift_right",
        }
    }

    /// Returns the arity (number of arguments) of this operation.
    pub fn arity(self) -> usize {
        match self {
            Self::ZeroWord | Self::OneWord | Self::TwoWord | Self::ThreeWord | Self::FourWord | Self::MaxWord => 0,
            Self::SuccWord
            | Self::PredWord
            | Self::SqrtWord
            | Self::EqualsZeroWord
            | Self::NotEqualsZeroWord
            | Self::EqualsOneWord
            | Self::EqualsMaxWord
            | Self::RightmostBit => 1,
            Self::AddWord
            | Self::AddWithCarryWord
            | Self::TimesWord
            | Self::TimesOverflowWord
            | Self::MinusWord
            | Self::MonusWord
            | Self::DivWord
            | Self::ModWord
            | Self::AddOverflowWord
            | Self::AddWithCarryOverflowWord
            | Self::Equal
            | Self::NotEqual
            | Self::Less
            | Self::LessEqual
            | Self::Greater
            | Self::GreaterEqual
            | Self::ShiftRight
            | Self::SqrtDoubleword => 2,
            Self::TimesWithCarryWord
            | Self::TimesWithCarryOverflowWord
            | Self::DivDoubleword
            | Self::ModDoubleword
            | Self::SqrtTripleword
            | Self::SqrtTriplewordOverflow => 3,
            Self::DivDoubleDoubleword | Self::SqrtQuadrupleword | Self::SqrtQuadruplewordOverflow => 4,
            Self::DivTripleDoubleword => 5,
        }
    }

    /// Evaluates this operation given its arguments, in declaration order.
    ///
    /// Returns `None` when an argument is not (yet) a concrete value the
    /// operation needs (a `MachineNumber`, or for `@shift_right` a `true`/
    /// `false` literal for its first, `Bool`, argument). Under an engine that
    /// only reaches a native op once its arguments are in normal form, this
    /// means a genuine word operation always evaluates; under one that
    /// doesn't guarantee that (e.g. an outermost strategy), a `None` here
    /// simply leaves the application unevaluated, matching mCRL2's own
    /// behaviour of requiring concrete word arguments.
    pub fn evaluate<'a, I>(self, mut args: I) -> Option<DataExpression>
    where
        I: Iterator<Item = DataExpressionRef<'a>>,
    {
        // `@shift_right` is the only operation whose first argument is a `Bool`
        // rather than a `@word`, so it cannot go through the uniform word-argument
        // path below.
        if self == Self::ShiftRight {
            let bit = as_bool(&args.next()?)?;
            let n = as_word(&args.next()?)?;
            return Some(machine_number(mw::shift_right(bit, n)));
        }

        // Collect the arguments as machine words; bail out as soon as one is not a
        // concrete word.
        let mut words = [0u64; 5];
        let mut len = 0;
        for arg in args {
            words[len] = as_word(&arg)?;
            len += 1;
        }

        match (self, &words[..len]) {
            // Constants.
            (Self::ZeroWord, []) => Some(machine_number(mw::zero_word())),
            (Self::OneWord, []) => Some(machine_number(mw::one_word())),
            (Self::TwoWord, []) => Some(machine_number(mw::two_word())),
            (Self::ThreeWord, []) => Some(machine_number(mw::three_word())),
            (Self::FourWord, []) => Some(machine_number(mw::four_word())),
            (Self::MaxWord, []) => Some(machine_number(mw::max_word())),

            // Unary word -> word.
            (Self::SuccWord, [n]) => Some(machine_number(mw::succ_word(*n))),
            (Self::PredWord, [n]) => Some(machine_number(mw::pred_word(*n))),
            (Self::SqrtWord, [n]) => Some(machine_number(mw::sqrt_word(*n))),

            // Unary word -> Bool.
            (Self::EqualsZeroWord, [n]) => Some(DataExpression::bool_literal(mw::equals_zero_word(*n))),
            (Self::NotEqualsZeroWord, [n]) => Some(DataExpression::bool_literal(mw::not_equals_zero_word(*n))),
            (Self::EqualsOneWord, [n]) => Some(DataExpression::bool_literal(mw::equals_one_word(*n))),
            (Self::EqualsMaxWord, [n]) => Some(DataExpression::bool_literal(mw::equals_max_word(*n))),
            (Self::RightmostBit, [n]) => Some(DataExpression::bool_literal(mw::rightmost_bit(*n))),

            // Binary word # word -> word.
            (Self::AddWord, [a, b]) => Some(machine_number(mw::add_word(*a, *b))),
            (Self::AddWithCarryWord, [a, b]) => Some(machine_number(mw::add_with_carry_word(*a, *b))),
            (Self::TimesWord, [a, b]) => Some(machine_number(mw::times_word(*a, *b))),
            (Self::TimesOverflowWord, [a, b]) => Some(machine_number(mw::times_overflow_word(*a, *b))),
            (Self::MinusWord, [a, b]) => Some(machine_number(mw::minus_word(*a, *b))),
            (Self::MonusWord, [a, b]) => Some(machine_number(mw::monus_word(*a, *b))),
            (Self::DivWord, [a, b]) => Some(machine_number(mw::div_word(*a, *b))),
            (Self::ModWord, [a, b]) => Some(machine_number(mw::mod_word(*a, *b))),

            // Binary word # word -> Bool.
            (Self::AddOverflowWord, [a, b]) => Some(DataExpression::bool_literal(mw::add_overflow_word(*a, *b))),
            (Self::AddWithCarryOverflowWord, [a, b]) => {
                Some(DataExpression::bool_literal(mw::add_with_carry_overflow_word(*a, *b)))
            }
            (Self::Equal, [a, b]) => Some(DataExpression::bool_literal(mw::equal_word(*a, *b))),
            (Self::NotEqual, [a, b]) => Some(DataExpression::bool_literal(mw::not_equal_word(*a, *b))),
            (Self::Less, [a, b]) => Some(DataExpression::bool_literal(mw::less_word(*a, *b))),
            (Self::LessEqual, [a, b]) => Some(DataExpression::bool_literal(mw::less_equal_word(*a, *b))),
            (Self::Greater, [a, b]) => Some(DataExpression::bool_literal(mw::greater_word(*a, *b))),
            (Self::GreaterEqual, [a, b]) => Some(DataExpression::bool_literal(mw::greater_equal_word(*a, *b))),

            // Ternary word # word # word -> word.
            (Self::TimesWithCarryWord, [a, b, c]) => Some(machine_number(mw::times_with_carry_word(*a, *b, *c))),
            (Self::TimesWithCarryOverflowWord, [a, b, c]) => {
                Some(machine_number(mw::times_with_carry_overflow_word(*a, *b, *c)))
            }
            (Self::DivDoubleword, [a, b, c]) => Some(machine_number(mw::div_doubleword(*a, *b, *c))),
            (Self::ModDoubleword, [a, b, c]) => Some(machine_number(mw::mod_doubleword(*a, *b, *c))),
            (Self::SqrtDoubleword, [a, b]) => Some(machine_number(mw::sqrt_doubleword(*a, *b))),
            (Self::SqrtTripleword, [a, b, c]) => Some(machine_number(mw::sqrt_tripleword(*a, *b, *c))),
            (Self::SqrtTriplewordOverflow, [a, b, c]) => Some(machine_number(mw::sqrt_tripleword_overflow(*a, *b, *c))),

            // Quaternary word # word # word # word -> word.
            (Self::DivDoubleDoubleword, [a, b, c, d]) => {
                Some(machine_number(mw::div_double_doubleword(*a, *b, *c, *d)))
            }
            (Self::SqrtQuadrupleword, [a, b, c, d]) => Some(machine_number(mw::sqrt_quadrupleword(*a, *b, *c, *d))),
            (Self::SqrtQuadruplewordOverflow, [a, b, c, d]) => {
                Some(machine_number(mw::sqrt_quadrupleword_overflow(*a, *b, *c, *d)))
            }

            // Quinary word # word # word # word # word -> word.
            (Self::DivTripleDoubleword, [a, b, c, d, e]) => {
                Some(machine_number(mw::div_triple_doubleword(*a, *b, *c, *d, *e)))
            }

            _ => None,
        }
    }
}

/// Attempts to evaluate `expr` as a machine-word operation applied to concrete
/// arguments, returning the resulting `MachineNumber` or `Bool` literal.
///
/// Returns `None` when `expr` is not a `@word` operation, or when one of its
/// arguments is not yet a concrete value. This is a convenience wrapper around
/// [`MachineWordOp::from_name`] and [`MachineWordOp::evaluate`] for callers
/// that don't already have the operation resolved; a rewrite engine's hot path
/// should resolve it once (e.g. via the `SetAutomaton`) rather than calling
/// this on every constructed term.
pub fn try_evaluate_machine_word(expr: &DataExpression) -> Option<DataExpression> {
    if !matches!(
        expr.kind(),
        DataExpressionKind::FunctionSymbol(_) | DataExpressionKind::Application(_)
    ) {
        return None;
    }

    let symbol = expr.data_function_symbol();
    let op = MachineWordOp::from_name(symbol.name().value())?;
    op.evaluate(expr.data_arguments())
}

/// Builds a `MachineNumber` data expression wrapping `value`.
fn machine_number(value: u64) -> DataExpression {
    MachineNumber::new(value).into()
}

/// Reads a machine number argument as its `u64` value, or `None` when the
/// argument is not (yet) a concrete machine number.
fn as_word(arg: &DataExpressionRef<'_>) -> Option<u64> {
    match arg.kind() {
        DataExpressionKind::MachineNumber(number) => Some(number.value()),
        _ => None,
    }
}

/// Reads a `Bool` literal argument, or `None` when the argument is not (yet) a
/// concrete `true`/`false` literal.
fn as_bool(arg: &DataExpressionRef<'_>) -> Option<bool> {
    match arg.kind() {
        DataExpressionKind::FunctionSymbol(symbol) => match symbol.name().value() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}
