//! Serializes a rewrite specification to the AProVE TRS format.
//! Format reference: https://aprove.informatik.rwth-aachen.de/help_new/trs.html

use std::fmt;

use ahash::HashSet;
use merc_aterm::ATermRef;
use merc_aterm::Term;
use merc_data::DataExpressionKind;
use merc_data::DataExpressionRef;
use merc_data::DataVariableRef;
use merc_data::is_data_variable;
use merc_sabre::RewriteSpecification;
use merc_sabre::is_supported_rule;

/// Finds all data symbols in the term and adds them to the symbol index.
fn find_variables(t: &DataExpressionRef<'_>, variables: &mut HashSet<String>) {
    for child in t.iter() {
        if is_data_variable(&child) {
            variables.insert(DataVariableRef::from(child.copy()).name().into());
        }
    }
}

pub struct SimpleTermFormatter<'a> {
    term: ATermRef<'a>,
}

impl SimpleTermFormatter<'_> {
    pub fn new<'a, 'b, T: Term<'a, 'b>>(term: &'b T) -> SimpleTermFormatter<'a> {
        SimpleTermFormatter { term: term.copy() }
    }
}

impl fmt::Display for SimpleTermFormatter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match DataExpressionRef::from(self.term.copy()).kind() {
            DataExpressionKind::FunctionSymbol(symbol) => write!(f, "{}_{}", symbol.name(), symbol.operation_id()),
            DataExpressionKind::Application(_) => {
                let mut args = self.term.arguments();

                let head = args.next().expect("An application has a head");
                write!(f, "{}", SimpleTermFormatter::new(&head))?;

                let mut first = true;
                for arg in args {
                    if !first {
                        write!(f, ", ")?;
                    } else {
                        write!(f, "(")?;
                    }

                    write!(f, "{}", SimpleTermFormatter::new(&arg))?;
                    first = false;
                }

                if !first {
                    write!(f, ")")?;
                }

                Ok(())
            }
            DataExpressionKind::Variable(variable) => write!(f, "{}", variable.name()),
            DataExpressionKind::MachineNumber(_)
            | DataExpressionKind::Abstraction(_)
            | DataExpressionKind::WhereClause(_) => write!(f, "{}", self.term),
        }
    }
}

pub struct TrsFormatter<'a> {
    spec: &'a RewriteSpecification,
}

impl TrsFormatter<'_> {
    pub fn new(spec: &RewriteSpecification) -> TrsFormatter<'_> {
        TrsFormatter { spec }
    }
}

impl fmt::Display for TrsFormatter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Find all the variables in the specification
        let variables = {
            let mut variables = HashSet::default();

            for rule in self.spec.rewrite_rules().iter() {
                find_variables(&rule.lhs.copy(), &mut variables);
                find_variables(&rule.rhs.copy(), &mut variables);

                for cond in &rule.conditions {
                    find_variables(&cond.lhs.copy(), &mut variables);
                    find_variables(&cond.rhs.copy(), &mut variables);
                }
            }

            variables
        };

        // Print the list of variables.
        writeln!(f, "(VAR ")?;
        for var in variables {
            writeln!(f, "\t {var} ")?;
        }
        writeln!(f, ") ")?;

        // Print the list of rules.
        writeln!(f, "(RULES ")?;
        for rule in self.spec.rewrite_rules().iter() {
            if is_supported_rule(rule) {
                let mut output = format!(
                    "\t {} -> {}",
                    SimpleTermFormatter::new(&rule.lhs),
                    SimpleTermFormatter::new(&rule.rhs)
                );
                for cond in &rule.conditions {
                    if cond.equality {
                        output += &format!(
                            " COND ==({},{}) -> true",
                            SimpleTermFormatter::new(&cond.lhs),
                            SimpleTermFormatter::new(&cond.rhs)
                        )
                    } else {
                        output += &format!(
                            " COND !=({},{}) -> true",
                            SimpleTermFormatter::new(&cond.lhs),
                            SimpleTermFormatter::new(&cond.rhs)
                        )
                    };
                }

                writeln!(
                    f,
                    "{}",
                    output.replace('|', "bar").replace('=', "eq").replace("COND", "|")
                )?;
            }
        }
        writeln!(f, ")")?;

        Ok(())
    }
}
