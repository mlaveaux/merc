use std::collections::HashSet;
use std::fmt;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use indoc::indoc;
use merc_sabre::AnnouncementInnermost;
use merc_sabre::RewriteSpecification;
use merc_sabre::SetAutomaton;
use merc_sabre::utilities::DataPosition;
use merc_sabre::utilities::TermStack;
use merc_utilities::MercError;

use crate::indenter::IndentFormatter;

/// Generates Rust code for term rewriting based on the provided specification.
///
/// Takes a rewrite specification and a source directory path, and generates the
/// necessary code for term rewriting using an automaton-based approach.
pub fn generate(spec: &RewriteSpecification, source_dir: &Path) -> Result<(), MercError> {
    let mut file = File::create(PathBuf::from(source_dir).join("lib.rs"))?;

    // Create an indented formatter for clean code generation
    let mut formatter = IndentFormatter::new(&mut file);

    // Generate the automata used for matching
    let apma = SetAutomaton::new(spec, AnnouncementInnermost::new, true);

    // Debug assertion to verify we have at least one state in the automaton
    debug_assert!(!apma.states().is_empty(), "Automaton must have at least one state");

    // Write imports and the main rewrite function
    writeln!(
        &mut formatter,
        indoc! {"use merc_sabre_ffi::DataExpressionFFI;
        use merc_sabre_ffi::DataExpressionRefFFI;

        /// Generic rewrite function
        #[unsafe(no_mangle)]
        pub unsafe extern \"C\" fn rewrite(term: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{
            rewrite_0(&term.copy())
        }}
        "}
    )?;

    // Introduce a match function for every state of the set automaton.
    let mut positions: HashSet<DataPosition> = HashSet::new();
    let mut term_stacks: Vec<TermStack> = Vec::new();

    for (index, state) in apma.states().iter().enumerate() {
        writeln!(&mut formatter, "// Position {}", state.label())?;

        for goal in state.match_goals() {
            writeln!(&mut formatter, "// Goal {goal:?}")?;
        }

        writeln!(
            &mut formatter,
            "fn rewrite_{index}(t: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{"
        )?;

        // Use the IndentFormatter to properly indent the function body
        let indent = formatter.indent();

        writeln!(
            &mut formatter,
            "let arg = get_data_position_{}(t);",
            UnderscoreFormatter(state.label())
        )?;
        writeln!(&mut formatter, "let symbol = arg.data_function_symbol();")?;

        positions.insert(state.label().clone());

        writeln!(&mut formatter, "match symbol.operation_id() {{")?;

        // Indent the match block
        let match_indent = formatter.indent();

        for ((from, symbol), transition) in apma.transitions() {
            if *from == index {
                // Outgoing transition
                writeln!(&mut formatter, "{symbol} => {{")?;

                // Indent the case block
                let case_indent = formatter.indent();
                writeln!(&mut formatter, "// Symbol {}", transition.symbol)?;

                // Continue on the outgoing transition.
                for (announcement, annotation) in &transition.announcements {
                    // Check for conditions and non linear patterns.
                    writeln!(&mut formatter, "// Announcement {announcement:?}")?;

                    writeln!(&mut formatter, "rewrite_term_stack_{}(t)", term_stacks.len())?;
                    term_stacks.push(annotation.rhs_stack.clone());
                }

                if transition.destinations.is_empty() {
                    writeln!(&mut formatter, "t.protect()")?;
                }

                for (position, to) in &transition.destinations {
                    positions.insert(position.clone());

                    writeln!(&mut formatter, "rewrite_{to}(&t)",)?;
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

    // Generate position getters
    generate_termstack_constructors(&mut formatter, &mut positions, &term_stacks)?;
    generate_position_getters(&mut formatter, &positions)?;

    // Ensure all data is written
    formatter.flush()?;

    // Post-condition assertion
    debug_assert!(!positions.is_empty(), "At least one position should be generated");

    Ok(())
}

/// Generates TermStack-based constructor functions for all rewrite rules
fn generate_termstack_constructors(
    formatter: &mut IndentFormatter<File>,
    positions: &mut HashSet<DataPosition>,
    term_stacks: &[TermStack],
) -> Result<(), MercError> {
    writeln!(formatter, "// TermStack-based constructor functions")?;

    for (index, term_stack) in term_stacks.iter().enumerate() {
        writeln!(
            formatter,
            "fn construct_term_stack_{index}(t: &DataExpressionRefFFI<'_>) -> DataExpressionFFI {{"
        )?;

        let indent = formatter.indent();

        writeln!(formatter, "// TermStack {:?}", term_stack)?;

        // Generate variable extraction code
        for (position, stack_index) in &term_stack.variables {
            positions.insert(position.clone());
            writeln!(
                formatter,
                "let var_{stack_index} = get_data_position_{}(t);",
                UnderscoreFormatter(position)
            )?;
        }

        // Generate TermStack evaluation code
        writeln!(formatter, "// TODO: Implement TermStack evaluation")?;
        writeln!(formatter, "// This would use the innermost_stack configuration")?;
        writeln!(formatter, "// and the extracted variables to construct the RHS")?;
        writeln!(formatter, "t.protect() // Placeholder")?;

        drop(indent);
        writeln!(formatter, "}}")?;
        writeln!(formatter)?;
    }

    Ok(())
}

/// Generates getter functions for all positions that must be read from terms.
fn generate_position_getters(
    formatter: &mut IndentFormatter<File>,
    positions: &HashSet<DataPosition>,
) -> Result<(), MercError> {
    writeln!(formatter, "// Get positions from term")?;

    for position in positions {
        writeln!(
            formatter,
            "fn get_data_position_{}<'a>(t: &DataExpressionRefFFI<'a>) -> DataExpressionRefFFI<'a> {{",
            UnderscoreFormatter(position)
        )?;

        // Indent the function body
        let indent = formatter.indent();

        if position.is_empty() {
            writeln!(formatter, "t.copy()")?;
        } else {
            write!(formatter, "t")?;

            for index in position.indices().iter() {
                write!(formatter, ".data_arg({index})")?;
            }

            // Add newline after the chain of method calls
            writeln!(formatter)?;
        }

        // The function indent is automatically decreased
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
