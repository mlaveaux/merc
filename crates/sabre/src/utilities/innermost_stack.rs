#![forbid(unsafe_code)]

use std::fmt;

use merc_aterm::Protected;
use merc_aterm::ProtectedWriteGuard;
use merc_data::DataExpressionRef;
use merc_data::DataFunctionSymbolRef;
use merc_utilities::debug_trace;

// Only used in debug_trace!
#[allow(unused_imports)]
use itertools::Itertools;

use crate::utilities::DataPositionIndexed;

use super::Config;
use super::TermStack;

/// This stack is used to avoid recursion and also to keep track of terms in
/// normal forms by explicitly representing the rewrites of a right hand
/// side.
#[derive(Default)]
pub struct InnermostStack {
    pub configs: Protected<Vec<Config<'static>>>,
    pub terms: Protected<Vec<Option<DataExpressionRef<'static>>>>,
}

impl InnermostStack {
    /// Updates the InnermostStack to integrate the rhs_stack instructions.
    pub fn integrate(
        write_configs: &mut ProtectedWriteGuard<Vec<Config<'static>>>,
        write_terms: &mut ProtectedWriteGuard<Vec<Option<DataExpressionRef<'static>>>>,
        rhs_stack: &TermStack,
        term: &DataExpressionRef<'_>,
        result_index: usize,
    ) {
        // TODO: This ignores the first element of the stack, but that is kind of difficult to deal with.
        let top_of_stack = write_terms.len();
        write_terms.reserve(rhs_stack.stack_size - 1); // We already reserved space for the result.
        for _ in 0..rhs_stack.stack_size - 1 {
            write_terms.push(None);
        }

        let mut first = true;
        for config in rhs_stack.innermost_stack.read().iter() {
            match config {
                Config::Construct(symbol, arity, offset) => {
                    if first {
                        // The first result must be placed on the original result index.
                        InnermostStack::add_result(write_configs, symbol.copy(), *arity, result_index);
                    } else {
                        // Otherwise, we put it on the end of the stack.
                        InnermostStack::add_result(write_configs, symbol.copy(), *arity, top_of_stack + offset - 1);
                    }
                }
                Config::Term(term, index) => {
                    let term = write_configs.protect(term);
                    write_configs.push(Config::Term(term.into(), *index));
                }
                Config::Rewrite(_) => {
                    unreachable!("This case should not happen");
                }
                Config::Return() => {
                    unreachable!("This case should not happen");
                }
            }
            first = false;
        }
        debug_trace!(
            "\t applied stack size: {}, substitution: {{{}}}, stack: [{}]",
            rhs_stack.stack_size,
            rhs_stack.variables.iter().format_with(", ", |element, f| {
                f(&format_args!("{} -> {}", element.0, element.1))
            }),
            rhs_stack.innermost_stack.read().iter().format("\n")
        );

        debug_assert!(
            rhs_stack.stack_size != 1 || rhs_stack.variables.len() <= 1,
            "There can only be a single variable in the right hand side"
        );
        if rhs_stack.stack_size == 1 && rhs_stack.variables.len() == 1 {
            // This is a special case where we place the result on the correct position immediately.
            // The right hand side is only a variable
            write_terms[result_index] = Some(
                write_terms
                    .protect(&term.get_data_position(&rhs_stack.variables[0].0))
                    .into(),
            );
        } else {
            for (position, index) in &rhs_stack.variables {
                // Add the positions to the stack.
                write_terms[top_of_stack + index - 1] =
                    Some(write_terms.protect(&term.get_data_position(position)).into());
            }
        }
    }

    /// Indicate that the given symbol with arity can be constructed at the given index.
    pub fn add_result(
        write_configs: &mut ProtectedWriteGuard<Vec<Config<'static>>>,
        symbol: DataFunctionSymbolRef<'_>,
        arity: usize,
        index: usize,
    ) {
        let symbol = write_configs.protect(&symbol);
        write_configs.push(Config::Construct(symbol.into(), arity, index));
    }

    /// Indicate that the term must be rewritten and its result must be placed at the given index.
    pub fn add_rewrite(
        write_configs: &mut ProtectedWriteGuard<Vec<Config<'static>>>,
        write_terms: &mut ProtectedWriteGuard<Vec<Option<DataExpressionRef<'static>>>>,
        term: DataExpressionRef<'_>,
        index: usize,
    ) {
        let term = write_terms.protect(&term);
        write_configs.push(Config::Rewrite(index));
        write_terms.push(Some(term.into()));
    }
}

impl fmt::Display for InnermostStack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Terms: [")?;
        for (i, entry) in self.terms.read().iter().enumerate() {
            match entry {
                Some(term) => writeln!(f, "{i}\t{term}")?,
                None => writeln!(f, "{i}\tNone")?,
            }
        }
        writeln!(f, "]")?;

        writeln!(f, "Configs: [")?;
        for config in self.configs.read().iter() {
            writeln!(f, "\t{config}")?;
        }
        write!(f, "]")
    }
}
