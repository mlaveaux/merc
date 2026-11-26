#![forbid(unsafe_code)]

use std::fmt;

use itertools::Itertools;
use merc_data::DataExpression;
use merc_data::DataFunctionSymbol;
use merc_data::MachineWordOp;
use merc_data::Mcrl2DataSpecification;

/// A rewrite specification is a set of rewrite rules, given by [Rule], plus
/// the machine-word (`@word`) operations available natively.
#[derive(Debug, Default, Clone)]
pub struct RewriteSpecification {
    rewrite_rules: Vec<Rule>,
    native_symbols: Vec<(DataFunctionSymbol, MachineWordOp)>,
}

impl RewriteSpecification {
    /// Create a new rewrite specification from the given rewrite rules, with
    /// no native machine-word operations available.
    pub fn new(rewrite_rules: Vec<Rule>) -> RewriteSpecification {
        RewriteSpecification {
            rewrite_rules,
            native_symbols: Vec::new(),
        }
    }

    /// Builds a rewrite specification from the equations of a fully typed
    /// mCRL2 data specification, e.g. the output of
    /// `merc_lowering::lower_data_specification`.
    ///
    /// A conditional equation `condition -> lhs = rhs` becomes a rule with a
    /// single condition that the (rewritten) condition equals the `Bool`
    /// literal `true`, matching how the mCRL2 rewriter treats equation
    /// conditions.
    ///
    /// The machine-word operations among `spec.constructors()` (`@zero_word`,
    /// `@succ_word`) and `spec.mappings()` (the rest) are carried along as
    /// [`RewriteSpecification::native_symbols`] — they have no equations of
    /// their own (they are `defined_by_code`), so they would otherwise be
    /// invisible to a rewrite engine built from this specification.
    pub fn from_data_specification(spec: &Mcrl2DataSpecification) -> RewriteSpecification {
        let true_literal = DataExpression::bool_literal(true);

        let rewrite_rules = spec
            .equations()
            .iter()
            .map(|equation| {
                let conditions = match equation.condition() {
                    Some(condition) => vec![Condition::new(condition.protect(), true_literal.clone(), true)],
                    None => Vec::new(),
                };

                Rule::with_condition(conditions, equation.lhs().protect(), equation.rhs().protect())
            })
            .collect();

        // Resolving a name to a `MachineWordOp` happens exactly once per native
        // symbol, here — every downstream consumer (the `SetAutomaton`, built
        // per state, per symbol) looks the operation up by the symbol's
        // identity instead of ever re-parsing its name.
        let native_symbols = spec
            .constructors()
            .iter()
            .chain(spec.mappings())
            .filter_map(|symbol| MachineWordOp::from_name(symbol.name().value()).map(|op| (symbol.clone(), op)))
            .collect();

        RewriteSpecification {
            rewrite_rules,
            native_symbols,
        }
    }

    /// Returns the rewrite rules of this specification.
    pub fn rewrite_rules(&self) -> &[Rule] {
        &self.rewrite_rules
    }

    /// Returns the native machine-word operations available to this
    /// specification, paired with their correctly-sorted function symbols.
    pub fn native_symbols(&self) -> &[(DataFunctionSymbol, MachineWordOp)] {
        &self.native_symbols
    }
}

/// A condition of a conditional rewrite rule.
///
/// Either `lhs == rhs` or `lhs != rhs` depending on equality being true.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub struct Condition {
    pub lhs: DataExpression,
    pub rhs: DataExpression,
    pub equality: bool,
}

impl Condition {
    /// Create a new condition with the given left-hand side, right-hand side and equality.
    pub fn new(lhs: DataExpression, rhs: DataExpression, equality: bool) -> Condition {
        Condition { lhs, rhs, equality }
    }
}

/// A rewrite rule.
#[derive(Debug, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct Rule {
    /// A conjunction of clauses
    pub conditions: Vec<Condition>,
    pub lhs: DataExpression,
    pub rhs: DataExpression,
}

impl Rule {
    /// Create a rewrite rule with the given conditions, left-hand side and right-hand side.
    pub fn with_condition(conditions: Vec<Condition>, lhs: DataExpression, rhs: DataExpression) -> Rule {
        Rule { conditions, lhs, rhs }
    }

    /// Create a rewrite rule without conditions.
    pub fn new(lhs: DataExpression, rhs: DataExpression) -> Rule {
        Rule {
            conditions: Vec::new(),
            lhs,
            rhs,
        }
    }
}

impl fmt::Display for RewriteSpecification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for rule in &self.rewrite_rules {
            writeln!(f, "{rule}")?;
        }
        Ok(())
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.conditions.is_empty() {
            write!(f, "{} = {}", self.lhs, self.rhs)
        } else {
            write!(
                f,
                "{} -> {} = {}",
                self.conditions.iter().format(", "),
                self.lhs,
                self.rhs
            )
        }
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.equality {
            write!(f, "{} == {}", self.lhs, self.rhs)
        } else {
            write!(f, "{} <> {}", self.lhs, self.rhs)
        }
    }
}
