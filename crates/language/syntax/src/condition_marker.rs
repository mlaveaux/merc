//! Workaround for the quadratic/exponential PEG backtracking of `ProcExprIf`/`ProcExprIfThen`.
//!
//! Both rules start with a `DataExpr` that is only known to be a condition once its `->` is
//! found, and `DataExpr` shares its `+`, `.` and `||` tokens with the process operators. Pest
//! therefore walks the rest of a long operand chain at every operand position (quadratic), and
//! the optional `<>` tail of a bare condition makes chains of conditions exponential.
//!
//! Before parsing, a linear scan over the source inserts [`COND_MARKER`] in front of every
//! condition, or [`ELSE_MARKER`] when the condition has a matching `<>`. The grammar requires
//! these markers before it attempts `DataExpr` (and before the `<>` tail), so every other
//! position fails immediately. [`with_markers`] maps the spans of the parsed tree back to the
//! original text afterwards.
//!
//! The scan only approximates where pest would find a condition. A misplaced or missing marker
//! can only make the parse fail, because the grammar requires them; it can never change the AST.
//! There is no fallback to the unmarked text, so scan gaps surface as parse errors, and the
//! positions in such errors refer to the marked text.

use std::ops::Range;

use crate::Keyword;
use crate::OffsetSpans;
use crate::PROC_EXPR_KEYWORDS;

/// Private-use codepoint marking the start of a condition without a matching `<>`. Must match
/// `CondMarker` in the grammar.
const COND_MARKER: char = '\u{E000}';

/// Private-use codepoint marking the start of a condition with a matching `<>`. Must match
/// `ElseMarker` in the grammar.
const ELSE_MARKER: char = '\u{E001}';

/// Keywords that start a section whose contents are not process expressions.
const DATA_SECTION_KEYWORDS: &[&str] = &[
    Keyword::Sort.name(),
    Keyword::Cons.name(),
    Keyword::Map.name(),
    Keyword::Glob.name(),
    Keyword::Act.name(),
    Keyword::Var.name(),
    Keyword::Eqn.name(),
];

/// Keywords that start a section containing process expressions.
const PROCESS_SECTION_KEYWORDS: &[&str] = &[Keyword::Proc.name(), Keyword::Init.name()];

/// Keywords whose parenthesised argument is a process expression.
const PROCESS_OPERATOR_KEYWORDS: &[&str] = &[
    Keyword::Hide.name(),
    Keyword::Block.name(),
    Keyword::Allow.name(),
    Keyword::Comm.name(),
    Keyword::Rename.name(),
];

fn is_data_section_keyword(text: &str) -> bool {
    DATA_SECTION_KEYWORDS.contains(&text)
}

fn is_process_section_keyword(text: &str) -> bool {
    PROCESS_SECTION_KEYWORDS.contains(&text)
}

fn is_process_operator_keyword(text: &str) -> bool {
    PROCESS_OPERATOR_KEYWORDS.contains(&text)
}

/// Keywords that can never occur inside a data expression, so a data expression run ends there.
///
/// `tau` and `delta` are excluded: the grammar's `Id` accepts them, so pest may still parse them
/// as part of a condition, and stopping here would move the marker and change the AST.
fn is_stop_keyword(text: &str) -> bool {
    (PROC_EXPR_KEYWORDS.contains(&text) && text != Keyword::Tau.name() && text != Keyword::Delta.name())
        || is_data_section_keyword(text)
        || is_process_section_keyword(text)
}

/// Multi-character tokens, longest first, so that e.g. `->` is not read as `-` and `>`.
const OPERATORS: &[&str] = &[
    "||_", "||", "|>", "<>", "<|", "<<", "<=", ">=", "==", "!=", "=>", "->", "&&", "++",
];

/// Returns the source text with condition markers inserted, together with the sorted byte offsets
/// of the inserted markers in the *marked* text, or `None` when there is no condition to mark.
///
/// Walks the tokens once. At every position where a process operand may start it computes how
/// far a `DataExpr` could extend ([`run_end`]); if that run ends in `->` the position is a
/// condition. The run is skipped as a whole, so each token is visited a bounded number of times.
pub fn mark_process_conditions(source: &str) -> Option<(String, Vec<usize>)> {
    let tokens = tokenize(source);
    let (matching, process_like) = match_brackets(source, &tokens);

    // Conditions found so far as (byte offset in `source`, has a matching `<>`).
    let mut insertions: Vec<(usize, bool)> = Vec::new();
    // Conditions still waiting for a `<>`, innermost last, as (bracket depth, index into `insertions`).
    let mut pending_ifs: Vec<(usize, usize)> = Vec::new();
    // Per bracket depth: operand starts before this token index are inside a data expression run
    // that was already found not to end in `->`, so they cannot be conditions either. This keeps
    // the scan linear for long operand chains.
    let mut doomed: Vec<usize> = vec![0];
    let mut depth = 0;
    // Per bracket depth: whether the bracket contains process expressions. Argument lists and
    // sets contain data and sorts instead, where a `->` is not a condition.
    let mut proc_ctx = vec![true];
    // Whether the scan is inside a `proc` or `init` section.
    let mut in_proc = false;
    // Whether the current token can start a process operand.
    let mut operand = false;

    let mut i = 0;
    while i < tokens.len() {
        let text = &source[tokens[i].clone()];

        if is_data_section_keyword(text) {
            in_proc = false;
        } else if is_process_section_keyword(text) {
            in_proc = true;
        }

        if in_proc && operand && proc_ctx[depth] && i >= doomed[depth] {
            // The maximal data expression starting here is a condition iff it is followed by `->`.
            let end = run_end(source, &tokens, &matching, &process_like, i);
            if end > i && end < tokens.len() && &source[tokens[end].clone()] == "->" {
                // Skip the condition itself; the process operand after the `->` is scanned next.
                insertions.push((tokens[i].start, false));
                pending_ifs.push((depth, insertions.len() - 1));
                i = end + 1;
                continue;
            }
            doomed[depth] = end;
        }

        let was_operand = operand;
        // `=` only declares a process at the top level; inside brackets it is an assignment.
        operand =
            (text == "=" && depth == 0) || matches!(text, "," | "+" | "." | "|" | "||" | "||_" | "<<" | "<>" | "init");
        match text {
            "(" | "[" | "{" => {
                depth += 1;
                // Brackets around a process operand, or the arguments of the operators that take a
                // process as operand, contain process expressions.
                let process = text == "("
                    && (was_operand || i > 0 && is_process_operator_keyword(&source[tokens[i - 1].clone()]));
                proc_ctx.push(process);
                doomed.resize(depth + 1, 0);
                doomed[depth] = 0;
                operand = process;
            }
            ")" | "]" | "}" => {
                pending_ifs.retain(|&(d, _)| d < depth);
                if depth > 0 {
                    depth -= 1;
                    proc_ctx.pop();
                }
            }
            ";" => pending_ifs.clear(),
            // A `<>` closes the nearest enclosing condition of the same bracket level.
            "<>" => {
                if let Some(&(_, index)) = pending_ifs.last().filter(|&&(d, _)| d == depth) {
                    pending_ifs.pop();
                    insertions[index].1 = true;
                }
            }
            _ => {}
        }
        i += 1;
    }

    if insertions.is_empty() {
        return None;
    }

    // Insert the markers, recording where they end up in the marked text.
    let mut marked = String::with_capacity(source.len() + insertions.len() * COND_MARKER.len_utf8());
    let mut offsets = Vec::with_capacity(insertions.len());
    let mut last = 0;
    for (at, has_else) in insertions {
        marked.push_str(&source[last..at]);
        offsets.push(marked.len());
        marked.push(if has_else { ELSE_MARKER } else { COND_MARKER });
        last = at;
    }
    marked.push_str(&source[last..]);
    Some((marked, offsets))
}

/// Runs `parse` on `spec` with condition markers inserted (if there are any), and maps the spans
/// of the result back to offsets in `spec`.
///
/// The offset of a marked position is corrected by the length of the markers that precede it.
pub fn with_markers<T: OffsetSpans, E>(spec: &str, parse: impl FnOnce(&str) -> Result<T, E>) -> Result<T, E> {
    let Some((marked, offsets)) = mark_process_conditions(spec) else {
        return parse(spec);
    };

    let mut result = parse(&marked)?;
    let len = COND_MARKER.len_utf8();
    let correct = |offset: usize| offset - len * offsets.partition_point(|&at| at < offset);
    result.map_spans(&mut |span| {
        span.start = correct(span.start);
        span.end = correct(span.end);
    });
    Ok(result)
}

/// Index of the first token after the maximal run of tokens, starting at `start`, that a
/// `DataExpr` could consume. Bracketed groups are skipped as a whole unless they are
/// `process_like` (see [`match_brackets`]).
fn run_end(
    source: &str,
    tokens: &[Range<usize>],
    matching: &[Option<usize>],
    process_like: &[bool],
    start: usize,
) -> usize {
    let mut j = start;
    while j < tokens.len() {
        let text = &source[tokens[j].clone()];
        match text {
            "(" | "[" | "{" => match matching[j] {
                Some(close) if !process_like[j] => j = close,
                _ => return j,
            },
            ")" | "]" | "}" | ";" | "," | "<>" | "->" | "=" | ":" | "|" | "||_" | "<<" | "@" => return j,
            _ if is_stop_keyword(text) => return j,
            _ => {}
        }
        j += 1;
    }
    j
}

/// For every opening bracket, the index of its closing bracket (if any), and whether it is
/// `process_like`: a `(` group that contains, directly or in a nested group, tokens that can never
/// occur in a data expression, such as the `=` of a process assignment or an empty argument list.
fn match_brackets(source: &str, tokens: &[Range<usize>]) -> (Vec<Option<usize>>, Vec<bool>) {
    let mut matching = vec![None; tokens.len()];
    let mut process_like = vec![false; tokens.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        match &source[token.clone()] {
            "(" | "[" | "{" => stack.push(index),
            ")" | "]" | "}" => {
                if let Some(open) = stack.pop() {
                    matching[open] = Some(index);
                    // An empty application `f()` is not a data expression.
                    process_like[open] |= open + 1 == index && &source[tokens[open].clone()] == "(";
                    if let Some(&parent) = stack.last() {
                        process_like[parent] |= process_like[open];
                    }
                }
            }
            "=" | "->" | "<>" | "sum" | "dist" | "|" | "||_" | "<<" | "@" => {
                if let Some(&open) = stack.last() {
                    process_like[open] |= &source[tokens[open].clone()] == "(";
                }
            }
            _ => {}
        }
    }
    (matching, process_like)
}

/// Splits `source` into identifier/number tokens, [`OPERATORS`] and single characters, skipping
/// whitespace and `%` comments. Returns the byte range of every token.
fn tokenize(source: &str) -> Vec<Range<usize>> {
    let bytes = source.as_bytes();
    let is_id = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'\'';
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let len = if b.is_ascii_whitespace() {
            i += 1;
            continue;
        } else if b == b'%' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        } else if is_id(b) {
            bytes[i..].iter().take_while(|&&c| is_id(c)).count()
        } else if let Some(op) = OPERATORS.iter().find(|op| source[i..].starts_with(**op)) {
            op.len()
        } else {
            // Single (possibly multi-byte) character.
            source[i..].chars().next().map_or(1, char::len_utf8)
        };
        tokens.push(i..i + len);
        i += len;
    }
    tokens
}
