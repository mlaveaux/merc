use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use indoc::indoc;

use itertools::Itertools;
use merc_aterm::Term;
use merc_sabre::AnnouncementInnermost;
use merc_sabre::RewriteSpecification;
use merc_sabre::SetAutomaton;
use merc_sabre::matching::conditions::EMACondition;
use merc_sabre::matching::nonlinear::EquivalenceClass;
use merc_sabre::utilities::Config;
use merc_sabre::utilities::DataPosition;
use merc_sabre::utilities::TermStack;
use merc_utilities::MercError;

use crate::indenter::IndentFormatter;

/// Writes a self-contained `lib.rs` under `source_dir` that implements an
/// innermost rewriter for `spec`, driven by a generated match automaton.
///
/// # Errors
///
/// Returns an error if the generated source cannot be written.
pub fn generate(spec: &RewriteSpecification, source_dir: &Path) -> Result<(), MercError> {
    let mut file = File::create(PathBuf::from(source_dir).join("lib.rs"))?;

    let mut formatter = IndentFormatter::new(&mut file);
    let apma = SetAutomaton::new(spec, AnnouncementInnermost::new, true);
    debug_assert!(!apma.states().is_empty(), "Automaton must have at least one state");

    // Emit transitions in a deterministic (state, symbol) order so the generated
    // source is reproducible and diffable.
    let sorted_transitions: Vec<(usize, usize, &_)> = apma
        .iter_transitions()
        .sorted_by_key(|(from, symbol, _)| (*from, *symbol))
        .collect();

    // Write imports and the main rewrite function
    writeln!(
        &mut formatter,
        indoc! {"#![allow(unused_variables)]
        #![allow(improper_ctypes_definitions)]

        use std::ffi::c_void;

        use merc_sabre_ffi::set_rewrite_vtable;
        use merc_sabre_ffi::SabreRewriteVTable;
        use merc_sabre_ffi::DataExpressionFFI;
        use merc_sabre_ffi::DataExpressionRefFFI;

        /// The initialisation function used to install the host vtable into the shared library.
        /// All term pool access is routed back into the host through it.
        #[unsafe(no_mangle)]
        pub unsafe extern \"C-unwind\" fn initialise(vtable: *mut c_void) {{
            unsafe {{ set_rewrite_vtable(vtable as *const SabreRewriteVTable); }}
        }}

        /// Generic rewrite function using the innermost strategy.
        ///
        /// First rewrites all arguments to normal form, reconstructs the term,
        /// and then tries to match the reconstructed term using the automaton.
        #[unsafe(no_mangle)]
        pub unsafe extern \"C-unwind\" fn rewrite(term: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{
            match term.arity() {{
                0 => rewrite_arity_0(term),
                1 => rewrite_arity_1(term),
                2 => rewrite_arity_2(term),
                3 => rewrite_arity_3(term),
                4 => rewrite_arity_4(term),
                5 => rewrite_arity_5(term),
                6 => rewrite_arity_6(term),
                7 => rewrite_arity_7(term),
                _ => rewrite_arity_generic(term),
            }}
        }}

        /// Try to match the given term using the automaton and apply a rewrite rule.
        /// If no rule matches, returns the term unchanged.
        fn match_term(term: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{
            match_0(&term.copy())
        }}

        /// Rewrite arity 0 (constant term)
        fn rewrite_arity_0(term: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{
            match_term(&term.copy())
        }}

        /// Rewrite arity 1
        fn rewrite_arity_1(term: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{
            unsafe {{
                let arg0 = rewrite(&term.data_arg(0));
                let symbol = term.data_function_symbol().into();
                let reconstructed = DataExpressionFFI::create(symbol, &[arg0.copy()]);
                match_term(&reconstructed.copy())
            }}
        }}

        /// Rewrite arity 2
        fn rewrite_arity_2(term: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{
            unsafe {{
                let arg0 = rewrite(&term.data_arg(0));
                let arg1 = rewrite(&term.data_arg(1));
                let symbol = term.data_function_symbol().into();
                let reconstructed = DataExpressionFFI::create(symbol, &[arg0.copy(), arg1.copy()]);
                match_term(&reconstructed.copy())
            }}
        }}

        /// Rewrite arity 3
        fn rewrite_arity_3(term: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{
            unsafe {{
                let arg0 = rewrite(&term.data_arg(0));
                let arg1 = rewrite(&term.data_arg(1));
                let arg2 = rewrite(&term.data_arg(2));
                let symbol = term.data_function_symbol().into();
                let reconstructed = DataExpressionFFI::create(symbol, &[arg0.copy(), arg1.copy(), arg2.copy()]);
                match_term(&reconstructed.copy())
            }}
        }}

        /// Rewrite arity 4
        fn rewrite_arity_4(term: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{
            unsafe {{
                let arg0 = rewrite(&term.data_arg(0));
                let arg1 = rewrite(&term.data_arg(1));
                let arg2 = rewrite(&term.data_arg(2));
                let arg3 = rewrite(&term.data_arg(3));
                let symbol = term.data_function_symbol().into();
                let reconstructed = DataExpressionFFI::create(symbol, &[arg0.copy(), arg1.copy(), arg2.copy(), arg3.copy()]);
                match_term(&reconstructed.copy())
            }}
        }}

        /// Rewrite arity 5
        fn rewrite_arity_5(term: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{
            unsafe {{
                let arg0 = rewrite(&term.data_arg(0));
                let arg1 = rewrite(&term.data_arg(1));
                let arg2 = rewrite(&term.data_arg(2));
                let arg3 = rewrite(&term.data_arg(3));
                let arg4 = rewrite(&term.data_arg(4));
                let symbol = term.data_function_symbol().into();
                let reconstructed = DataExpressionFFI::create(symbol, &[arg0.copy(), arg1.copy(), arg2.copy(), arg3.copy(), arg4.copy()]);
                match_term(&reconstructed.copy())
            }}
        }}

        /// Rewrite arity 6
        fn rewrite_arity_6(term: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{
            unsafe {{
                let arg0 = rewrite(&term.data_arg(0));
                let arg1 = rewrite(&term.data_arg(1));
                let arg2 = rewrite(&term.data_arg(2));
                let arg3 = rewrite(&term.data_arg(3));
                let arg4 = rewrite(&term.data_arg(4));
                let arg5 = rewrite(&term.data_arg(5));
                let symbol = term.data_function_symbol().into();
                let reconstructed = DataExpressionFFI::create(symbol, &[arg0.copy(), arg1.copy(), arg2.copy(), arg3.copy(), arg4.copy(), arg5.copy()]);
                match_term(&reconstructed.copy())
            }}
        }}

        /// Rewrite arity 7
        fn rewrite_arity_7(term: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{
            unsafe {{
                let arg0 = rewrite(&term.data_arg(0));
                let arg1 = rewrite(&term.data_arg(1));
                let arg2 = rewrite(&term.data_arg(2));
                let arg3 = rewrite(&term.data_arg(3));
                let arg4 = rewrite(&term.data_arg(4));
                let arg5 = rewrite(&term.data_arg(5));
                let arg6 = rewrite(&term.data_arg(6));
                let symbol = term.data_function_symbol().into();
                let reconstructed = DataExpressionFFI::create(symbol, &[arg0.copy(), arg1.copy(), arg2.copy(), arg3.copy(), arg4.copy(), arg5.copy(), arg6.copy()]);
                match_term(&reconstructed.copy())
            }}
        }}

        /// Rewrite arity 8 or higher (uses vector allocation)
        fn rewrite_arity_generic(term: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{
            unsafe {{
                let arity = term.arity();
                let args: Vec<DataExpressionFFI> = (0..arity).map(|i| rewrite(&term.data_arg(i))).collect();
                let arg_refs: Vec<DataExpressionRefFFI> = args.iter().map(|a| a.copy()).collect();
                let symbol = term.data_function_symbol().into();
                let reconstructed = DataExpressionFFI::create(symbol, &arg_refs);
                match_term(&reconstructed.copy())
            }}
        }}
        "}
    )?;

    // Keep track of all positions that need to be read from terms, to generate getters for them later.
    let mut positions: HashSet<DataPosition> = HashSet::new();

    // Introduce a match function for every state of the set automaton.
    for (index, state) in apma.states().iter().enumerate() {
        writeln!(&mut formatter, "// Position {}", state.label())?;

        for goal in state.match_goals() {
            writeln!(&mut formatter, "// Goal {goal:?}")?;
        }

        writeln!(
            &mut formatter,
            "fn match_{index}(t: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{"
        )?;

        let indent = formatter.indent();

        writeln!(
            &mut formatter,
            "let arg = get_data_position_{}(t);",
            UnderscoreFormatter(state.label())
        )?;
        writeln!(&mut formatter, "let symbol = arg.data_function_symbol();")?;

        positions.insert(state.label().clone());

        writeln!(&mut formatter, "match symbol.operation_id() {{")?;

        let match_indent = formatter.indent();
        for (from, symbol, transition) in &sorted_transitions {
            // Consider only transitions that match the current state index
            if *from == index {
                writeln!(&mut formatter, "{symbol} => {{")?;

                // Indent the case block
                let case_indent = formatter.indent();
                writeln!(&mut formatter, "// Symbol {}", transition.symbol)?;

                for (ann_idx, (announcement, _annotation)) in transition.announcements.iter().enumerate() {
                    writeln!(&mut formatter, "// Announcement {announcement:?}")?;

                    writeln!(
                        &mut formatter,
                        "if check_equivalence_classes_{index}_{symbol}_{ann_idx}(t) && check_condition_{index}_{symbol}_{ann_idx}(t) {{",
                    )?;

                    let condition_indent = formatter.indent();
                    writeln!(
                        &mut formatter,
                        "return rewrite_term_stack_{index}_{symbol}_{ann_idx}(t)"
                    )?;
                    drop(condition_indent);

                    writeln!(&mut formatter, "}}")?;
                }

                if transition.destinations.is_empty() {
                    writeln!(&mut formatter, "t.protect()")?;
                }

                for (position, to) in &transition.destinations {
                    positions.insert(position.clone());

                    writeln!(&mut formatter, "match_{to}(&t)",)?;
                }

                drop(case_indent);
                writeln!(&mut formatter, "}}")?;
            }
        }

        // No match
        writeln!(&mut formatter, "_ => {{")?;

        // Indent the default case
        {
            let _default_indent = formatter.indent();
            writeln!(&mut formatter, "t.protect()")?;
        }

        writeln!(&mut formatter, "}}")?;

        drop(match_indent);
        writeln!(&mut formatter, "}}")?;

        drop(indent);
        writeln!(&mut formatter, "}}")?;
        writeln!(&mut formatter)?;
    }

    writeln!(formatter, "// term stack rewrite functions")?;
    writeln!(formatter)?;
    for (from, symbol, transition) in &sorted_transitions {
        for (annotation_index, (_announcement, annotation)) in transition.announcements.iter().enumerate() {
            generate_rewrite_term_stack(
                &mut formatter,
                *from,
                *symbol,
                annotation_index,
                &mut positions,
                &annotation.rhs_stack,
            )?;
        }
    }

    writeln!(formatter, "// check condition functions")?;
    writeln!(formatter)?;
    for (from, symbol, transition) in &sorted_transitions {
        for (annotation_index, (_announcement, annotation)) in transition.announcements.iter().enumerate() {
            generate_check_condition(
                &mut formatter,
                *from,
                *symbol,
                annotation_index,
                &mut positions,
                &annotation.conditions,
            )?;
        }
    }

    writeln!(formatter, "// equivalence classes check")?;
    writeln!(formatter)?;
    for (from, symbol, transition) in &sorted_transitions {
        for (annotation_index, (_announcement, annotation)) in transition.announcements.iter().enumerate() {
            generate_check_equivalence_classes(
                &mut formatter,
                *from,
                *symbol,
                annotation_index,
                &mut positions,
                &annotation.equivalence_classes,
            )?;
        }
    }

    writeln!(formatter, "// position getters")?;
    writeln!(formatter)?;
    generate_position_getters(&mut formatter, &positions)?;

    formatter.flush()?;

    Ok(())
}

/// Emits variable declarations and construct calls for `term_stack` into
/// `formatter`, prefixing every variable name with `prefix`. Returns the name
/// of the variable holding the final result.
///
/// Simulates the runtime stack: non-output slots start as `[1 .. stack_size)`,
/// and each reversed-BFS construct drains the last `arity` indices as its
/// arguments.
///
/// # Safety contract of the emitted code (not this function itself)
///
/// For every `Config::Construct`/`Config::Term` entry, this bakes the *current* raw
/// term-pool address of that symbol/subterm (`shared().ptr().as_ptr() as usize`) into the
/// generated source as a `DataExpressionRefFFI::from_ptr(<addr>)` literal, read once at
/// codegen time and dereferenced again on every call, for as long as the compiled library
/// stays loaded — with no re-validation at codegen, compile or load time.
///
/// This is sound only because every embedded address is reachable from a `Rule` inside the
/// `RewriteSpecification` (directly for a `Construct` symbol, or as a subterm of `rule.rhs`/a
/// condition — see `TermStack::from_term`), and `SabreCompilingRewriter::new` clones that
/// specification into its `_spec` field (see `sabre_compiling.rs:30`-`32`) before returning.
/// Hash-consing means the clone keeps the exact same pool entry alive at the same address, not
/// merely an equal one. Callers of `generate` must not let any term/symbol whose address is
/// embedded here become unreachable while the generated library can still be invoked.
fn generate_rewrite_term_stack_impl(
    formatter: &mut IndentFormatter<File>,
    prefix: &str,
    term_stack: &TermStack,
    positions: &mut HashSet<DataPosition>,
) -> Result<String, MercError> {
    // Declare variables bound to subterms of the matched term.
    for (position, stack_index) in &term_stack.variables {
        positions.insert(position.clone());
        writeln!(
            formatter,
            "let {prefix}var_{stack_index} = get_data_position_{}(t);",
            UnderscoreFormatter(position)
        )?;
    }

    // `integrate` allocates slots 1..stack_size for non-output entries.
    // Evaluation pops constructs from the config stack (LIFO = reversed BFS)
    // and each one consumes the last `arity` elements of the term-stack.
    // Simulate that here so we know which var_N holds each argument.
    let mut virtual_stack: Vec<usize> = (1..term_stack.stack_size).collect();

    let read = term_stack.innermost_stack.read();
    for config in read.iter().rev() {
        match config {
            Config::Construct(symbol, arity, stack_index) => {
                let len = virtual_stack.len();
                let arg_indices: Vec<usize> = virtual_stack.drain(len - arity..).collect();

                if *arity > 0 {
                    writeln!(
                        formatter,
                        "let {prefix}var_{stack_index} = match_term(&DataExpressionFFI::create(unsafe {{ DataExpressionRefFFI::from_ptr({:?}) }}, &[{}]).copy());",
                        symbol.shared().ptr().as_ptr() as *mut () as usize,
                        arg_indices
                            .iter()
                            .map(|i| format!("{prefix}var_{i}.copy()"))
                            .format(", ")
                    )?;
                } else {
                    writeln!(
                        formatter,
                        "let {prefix}var_{stack_index} = match_term(&DataExpressionFFI::constant(unsafe {{ DataExpressionRefFFI::from_ptr({:?}) }}).copy());",
                        symbol.shared().ptr().as_ptr() as *mut () as usize,
                    )?;
                }
            }
            Config::Term(data_expression_ref, index) => {
                writeln!(
                    formatter,
                    "let {prefix}var_{index} = match_term(&unsafe {{ DataExpressionRefFFI::from_ptr({:?}) }});",
                    data_expression_ref.shared().ptr().as_ptr() as *mut () as usize,
                )?;
            }
            Config::Rewrite(_) | Config::Return() => {
                unreachable!("The term stack never contains these configurations")
            }
        }
    }

    // Determine the result variable name.
    let result = if let Some(stack_index) = term_stack.innermost_stack.read().iter().find_map(|c| {
        if let Config::Construct(_, _, idx) = c {
            Some(*idx)
        } else {
            None
        }
    }) {
        // The root of the BFS tree (first construct) holds the final result.
        format!("{prefix}var_{stack_index}")
    } else if term_stack.stack_size == 1 && term_stack.variables.len() == 1 {
        // RHS is a bare variable; return the matched subterm directly.
        let (_, stack_index) = &term_stack.variables[0];
        format!("{prefix}var_{stack_index}")
    } else {
        // Should not occur for valid rewrite rules.
        "t".to_string()
    };

    Ok(result)
}

/// Generates a `rewrite_term_stack_{index}_{symbol}_{ann_idx}` function that
/// constructs the RHS of a rewrite rule and returns it in normal form.
fn generate_rewrite_term_stack(
    formatter: &mut IndentFormatter<File>,
    index: usize,
    symbol: usize,
    ann_idx: usize,
    positions: &mut HashSet<DataPosition>,
    term_stack: &TermStack,
) -> Result<(), MercError> {
    writeln!(formatter, "/// Rewriting {:?}", term_stack)?;
    writeln!(
        formatter,
        "fn rewrite_term_stack_{index}_{symbol}_{ann_idx}(t: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{"
    )?;

    let indent = formatter.indent();
    let result_var = generate_rewrite_term_stack_impl(formatter, "", term_stack, positions)?;
    writeln!(formatter, "{result_var}.protect()")?;
    drop(indent);

    writeln!(formatter, "}}")?;
    writeln!(formatter)?;

    Ok(())
}

/// Generates a `check_condition_{index}_{symbol}_{annotation_index}` function that
/// evaluates each condition's LHS and RHS, rewrites both, and returns `false`
/// if any condition is violated.
fn generate_check_condition(
    formatter: &mut IndentFormatter<File>,
    index: usize,
    symbol: usize,
    annotation_index: usize,
    positions: &mut HashSet<DataPosition>,
    conditions: &[EMACondition],
) -> Result<(), MercError> {
    writeln!(formatter, "/// Checking condition {:?}", conditions)?;
    writeln!(
        formatter,
        "fn check_condition_{index}_{symbol}_{annotation_index}(t: &DataExpressionRefFFI<'_>) -> bool {{"
    )?;

    let indent = formatter.indent();

    for (cond_idx, condition) in conditions.iter().enumerate() {
        writeln!(formatter, "// Condition {cond_idx}")?;
        writeln!(formatter, "{{")?;

        let cond_indent = formatter.indent();

        let lhs_var = generate_rewrite_term_stack_impl(
            formatter,
            &format!("c{cond_idx}_lhs_"),
            &condition.lhs_term_stack,
            positions,
        )?;
        let rhs_var = generate_rewrite_term_stack_impl(
            formatter,
            &format!("c{cond_idx}_rhs_"),
            &condition.rhs_term_stack,
            positions,
        )?;

        // With maximal sharing, pointer equality ↔ term equality.
        if condition.equality {
            writeln!(formatter, "if {lhs_var}.shared() != {rhs_var}.shared() {{")?;
        } else {
            writeln!(formatter, "if {lhs_var}.shared() == {rhs_var}.shared() {{")?;
        }
        {
            let body_indent = formatter.indent();
            writeln!(formatter, "return false;")?;
            drop(body_indent);
        }
        writeln!(formatter, "}}")?;

        drop(cond_indent);
        writeln!(formatter, "}}")?;
    }

    writeln!(formatter, "true")?;
    drop(indent);

    writeln!(formatter, "}}")?;
    Ok(())
}

/// Generates a `check_equivalence_classes_{index}_{symbol}_{annotation_index}` function
/// that verifies all positions belonging to the same variable refer to identical
/// subterms (required for non-linear patterns).
fn generate_check_equivalence_classes(
    formatter: &mut IndentFormatter<File>,
    index: usize,
    symbol: usize,
    annotation_index: usize,
    positions: &mut HashSet<DataPosition>,
    equivalence_classes: &[EquivalenceClass],
) -> Result<(), MercError> {
    writeln!(formatter, "/// Check equivalence classes {:?}", equivalence_classes)?;
    writeln!(
        formatter,
        "fn check_equivalence_classes_{index}_{symbol}_{annotation_index}<'a>(t: &DataExpressionRefFFI<'a>) -> bool {{",
    )?;

    let indent = formatter.indent();

    for ec in equivalence_classes {
        debug_assert!(
            ec.positions.len() >= 2,
            "An equivalence class must contain at least two positions"
        );

        writeln!(formatter, "// Variable {} must match at all positions", ec.variable)?;

        let first_pos = &ec.positions[0];
        positions.insert(first_pos.clone());
        writeln!(
            formatter,
            "let base = get_data_position_{}(t);",
            UnderscoreFormatter(first_pos)
        )?;

        for other_pos in &ec.positions[1..] {
            positions.insert(other_pos.clone());
            writeln!(
                formatter,
                "if base.shared() != get_data_position_{}(t).shared() {{ return false; }}",
                UnderscoreFormatter(other_pos)
            )?;
        }
    }

    writeln!(formatter, "true")?;
    drop(indent);

    writeln!(formatter, "}}")?;
    writeln!(formatter)?;
    Ok(())
}

/// Generates getter functions for all positions that must be read from terms.
fn generate_position_getters(
    formatter: &mut IndentFormatter<File>,
    positions: &HashSet<DataPosition>,
) -> Result<(), MercError> {
    // Emit in a deterministic order; `positions` is a `HashSet`.
    for position in positions.iter().sorted() {
        writeln!(formatter, "/// Get position {:?} from term", position)?;
        writeln!(
            formatter,
            "fn get_data_position_{}<'a>(t: &DataExpressionRefFFI<'a>) -> DataExpressionRefFFI<'a> {{",
            UnderscoreFormatter(position)
        )?;

        let indent = formatter.indent();

        if position.is_empty() {
            writeln!(formatter, "t.copy()")?;
        } else {
            write!(formatter, "t")?;

            for index in position.indices().iter() {
                write!(formatter, ".data_arg({})", index - 1)?; // positions are 1-indexed
            }

            // Add newline after the chain of method calls
            writeln!(formatter)?;
        }

        drop(indent);
        writeln!(formatter, "}}")?;
        writeln!(formatter)?;
    }

    Ok(())
}

struct UnderscoreFormatter<'a>(&'a DataPosition);

impl fmt::Display for UnderscoreFormatter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            write!(f, "epsilon")?;
        } else {
            let mut first = true;
            for p in self.0.indices().iter() {
                if first {
                    write!(f, "{p}")?;
                    first = false;
                } else {
                    write!(f, "_{p}")?;
                }
            }
        }

        Ok(())
    }
}
