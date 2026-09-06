//! Preprocessing pass that works around two independent PEG backtracking pathologies in
//! `ProcExpr`/`DataExpr` parsing (see `docs/procexpr-parsing-performance-plan.md` at the
//! repository root):
//!
//! - §1: `ProcExprIfPrefix`/`ProcExprIfThen` both start by greedily parsing a full `DataExpr`,
//!   hoping to find a trailing `->`. Because `DataExpr`'s infix operators (`+`, `.`, `||`) are the
//!   same tokens `ProcExpr` uses for choice/sequence/parallel, a failed attempt walks all the way
//!   to the end of the enclosing process-expression chain before backtracking — and this is
//!   retried at every operand position, giving O(n²) (and, for long chains, a stack overflow) on
//!   `+`/`.`/`||`-heavy specifications such as `garage-ver.mcrl2`.
//! - §1b: once a condition and its `->` are found, `ProcExprIfPrefix`'s optional `(ProcExprNoIf ~
//!   "<>")?` tail (and `ProcExprIfThen`'s otherwise-identical mandatory one) *unconditionally*
//!   attempts to also find a matching `<>`. `ProcExprNoIf`'s own repetition is possessive/greedy in
//!   exactly the same way, and — because every further condition encountered along that walk
//!   recursively attempts the very same optional tail — a chain of *n* if-without-`<>` terms costs
//!   not just O(n²) but genuinely exponential time (verified: n=10 → 165ms, n=20 → does not finish
//!   in 3s), which is what still made `MLV.mcrl2`'s `ConfigurationMemory` process hang even with
//!   every one of its conditions correctly marked by §1's fix alone.
//!
//! [mark_process_conditions] finds, in one linear scan, every position that is genuinely the start
//! of such a condition (inserting [CONDITION_MARKER] there) and, for each one, whether it has a
//! reachable matching `<>` under the standard nearest-enclosing-if rule (inserting [ELSE_MARKER]
//! right after its `->` if so). `mcrl2_grammar.pest`'s `ProcExprIfPrefix`/`ProcExprIfThen` require
//! [CONDITION_MARKER] as their first token and [ELSE_MARKER] before ever attempting the `<>`-tail,
//! so every position/attempt that would otherwise have walked to the end of the chain now fails (or
//! succeeds, for the tail) in O(1) instead. [merc_utilities::with_offset_corrections] undoes the
//! resulting span shift so the produced AST is unaffected.
//!
//! Getting every edge case of "is this a genuine condition" / "does it have an else" exactly right
//! is not required for correctness: both markers are codepoints that cannot appear anywhere in
//! valid mCRL2 source and that no grammar rule other than `ProcExprIfPrefix`/`ProcExprIfThen`
//! expects, so a wrongly placed (or wrongly omitted) marker can only ever cause a hard parse error,
//! never a silently different parse.
//! [UntypedProcessSpecification::parse](crate::UntypedProcessSpecification::parse) tries the
//! marked text first and falls back to parsing the original text unmodified if that fails, so a
//! gap in this scanner's coverage costs the speedup for that input, never correctness.

/// The marker inserted before every condition this scan finds. Taken from the Private Use Area,
/// so it cannot occur in, or be confused with, any valid mCRL2 token: identifiers are restricted
/// to ASCII alphanumerics/`_`/`'`, and every other grammar rule matches specific ASCII punctuation
/// or keywords.
pub(crate) const CONDITION_MARKER: char = '\u{E000}';

/// The number of bytes [CONDITION_MARKER] occupies once UTF-8 encoded, i.e. how far every
/// following position shifts relative to the unmarked text. Shared by [mark_process_conditions]
/// for both [CONDITION_MARKER] and [ELSE_MARKER] insertions (see the `const` assertion below), and
/// by every caller of [merc_utilities::with_offset_corrections], since that function's
/// `insertion_len` applies uniformly to every position in its `insertions` list.
pub(crate) const CONDITION_MARKER_LEN: usize = CONDITION_MARKER.len_utf8();

/// The marker inserted immediately after the `->` of every condition this scan proves has a
/// reachable `<>` — see the module documentation's §1b. Same Private-Use-Area rationale as
/// [CONDITION_MARKER], and deliberately the same encoded length (checked below) so both markers
/// can be inserted by one pass and corrected by one [merc_utilities::with_offset_corrections] call.
pub(crate) const ELSE_MARKER: char = '\u{E001}';

const _: () = assert!(ELSE_MARKER.len_utf8() == CONDITION_MARKER_LEN);

/// One pending marker insertion: the byte offset (in the *original*, unmarked source) to splice
/// either [CONDITION_MARKER] or [ELSE_MARKER] at.
type Marks = Vec<(usize, char)>;

/// Scans `source` for the start of every `ProcExprIfPrefix`/`ProcExprIfThen` condition and returns
/// a copy of `source` with [CONDITION_MARKER] inserted at each one (and [ELSE_MARKER] after every
/// one that has a reachable `<>`), together with the sorted (marked-text) byte offsets the markers
/// were inserted at, for use with [merc_utilities::with_offset_corrections].
///
/// This only looks inside `proc`/`init` bodies (the only place `ProcExpr` appears) and only
/// reasons about the token *shapes* `DataExpr` and `ProcExpr` share — it does not build an AST or
/// otherwise duplicate their grammars, and it is deliberately conservative: seeing something it
/// does not recognize just stops that particular scan rather than guessing. See the module
/// documentation for why it does not need to be exhaustively correct.
pub(crate) fn mark_process_conditions(source: &str) -> (String, Vec<usize>) {
    let bytes = source.as_bytes();
    let mut marks: Marks = Vec::new();
    scan_top_level(bytes, &mut marks);
    marks.sort_unstable_by_key(|&(position, marker)| (position, marker));
    marks.dedup();

    if marks.is_empty() {
        return (source.to_string(), Vec::new());
    }

    let mut marked = String::with_capacity(source.len() + marks.len() * CONDITION_MARKER_LEN);
    let mut marked_positions = Vec::with_capacity(marks.len());
    let mut previous = 0;
    for &(position, marker) in &marks {
        // Every mark is computed at a recognized token boundary (a word, an operator, an opening
        // bracket, or the position right after a `->`), which is always a valid char boundary in
        // well-formed mCRL2 source — but if that assumption is ever wrong for some input, fail
        // safe by not marking anything rather than panicking on a mid-character slice.
        let (Some(prefix), true) = (source.get(previous..position), source.is_char_boundary(position)) else {
            return (source.to_string(), Vec::new());
        };
        marked.push_str(prefix);
        marked_positions.push(marked.len());
        marked.push(marker);
        previous = position;
    }
    marked.push_str(&source[previous..]);

    (marked, marked_positions)
}

/// Skips whitespace and `%`-comments, mirroring the grammar's `WHITESPACE`/`COMMENT` rules.
fn skip_trivia(bytes: &[u8], mut pos: usize) -> usize {
    loop {
        match bytes.get(pos) {
            Some(b' ' | b'\t' | b'\r' | b'\n') => pos += 1,
            Some(b'%') => {
                pos += 1;
                while !matches!(bytes.get(pos), None | Some(b'\n')) {
                    pos += 1;
                }
            }
            _ => return pos,
        }
    }
}

/// Whether `bytes[pos]` starts an identifier/number/keyword character, per the grammar's `Id`.
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'\''
}

/// The end of the maximal run of [is_word_byte] characters starting at `pos`.
fn word_end(bytes: &[u8], mut pos: usize) -> usize {
    while bytes.get(pos).is_some_and(|&b| is_word_byte(b)) {
        pos += 1;
    }
    pos
}

/// Whether `keyword` occurs at `pos` as a whole word (not a prefix of a longer identifier).
fn matches_keyword(bytes: &[u8], pos: usize, keyword: &str) -> bool {
    bytes[pos..].starts_with(keyword.as_bytes()) && !bytes.get(pos + keyword.len()).is_some_and(|&b| is_word_byte(b))
}

/// Given `pos` at an opening `(`/`[`/`{`, returns the index of its matching closing bracket,
/// treating comments as opaque and tracking all three bracket kinds so mismatched nesting inside
/// (e.g. `(a[b]c)`) does not confuse it. Returns `None` on unbalanced/malformed input.
fn find_matching_close(bytes: &[u8], pos: usize) -> Option<usize> {
    let mut depth: Vec<u8> = vec![bytes[pos]];
    let mut i = pos + 1;
    loop {
        let start = skip_trivia(bytes, i);
        match *bytes.get(start)? {
            open @ (b'(' | b'[' | b'{') => {
                depth.push(open);
                i = start + 1;
            }
            close @ (b')' | b']' | b'}') => {
                let open = match close {
                    b')' => b'(',
                    b']' => b'[',
                    _ => b'{',
                };
                if depth.pop() != Some(open) {
                    return None;
                }
                if depth.is_empty() {
                    return Some(start);
                }
                i = start + 1;
            }
            _ => i = start + 1,
        }
    }
}

/// Whether the content between `open` (a `(`) and `close` (its matching `)`) could be a
/// `DataExprList` (one or more comma-separated plain `DataExpr`s) — as opposed to empty, or
/// `AssignmentList`-shaped (`x=y, ...`), which `Action`/`ProcExprId` also accept as argument lists
/// but `DataExprApplication` does not. Only a top-level (not inside a further nested bracket) bare
/// `=` is checked for — `==`/`!=`/`<=`/`>=`/`=>` are not assignments.
fn looks_like_data_expr_list(bytes: &[u8], open: usize, close: usize) -> bool {
    let mut i = skip_trivia(bytes, open + 1);
    if i >= close {
        return false; // empty
    }
    while i < close {
        match bytes[i] {
            b'(' | b'[' | b'{' => i = find_matching_close(bytes, i).map_or(close, |c| c + 1),
            // "==" / "=>": consumed as one unit so neither half is ever inspected on its own —
            // in particular, so the second "=" of "==" is never mistaken for a bare one.
            b'=' if matches!(bytes.get(i + 1), Some(b'=' | b'>')) => i += 2,
            // "!=" / "<=" / ">=": likewise, so the "=" half is never inspected on its own.
            b'!' | b'<' | b'>' if bytes.get(i + 1) == Some(&b'=') => i += 2,
            b'=' => return false, // a bare "=": assignment-shaped
            _ => i += 1,
        }
    }
    true
}

/// The top-level scan: finds `proc`/`init` sections (the only places `ProcExpr` occurs) anywhere
/// at bracket depth 0 and hands their bodies to [scan_expr_region]. Everything else is skipped
/// without being inspected.
fn scan_top_level(bytes: &[u8], marks: &mut Marks) {
    let mut pos = 0;
    while pos < bytes.len() {
        let start = skip_trivia(bytes, pos);
        if start >= bytes.len() {
            return;
        }

        match bytes[start] {
            b'(' | b'[' | b'{' => {
                pos = match find_matching_close(bytes, start) {
                    Some(close) => close + 1,
                    None => return,
                };
            }
            b if is_word_byte(b) => {
                if matches_keyword(bytes, start, "proc") {
                    pos = scan_proc_spec(bytes, word_end(bytes, start), marks);
                } else if matches_keyword(bytes, start, "init") {
                    // `Init = "init" ~ ProcExpr ~ ";"`: the body starts right after the keyword.
                    let body_start = skip_trivia(bytes, word_end(bytes, start));
                    pos = scan_decl_body(bytes, body_start, marks);
                } else {
                    pos = word_end(bytes, start);
                }
            }
            _ => pos = start + 1,
        }
    }
}

/// Scans zero or more `Id ~ ("(" ~ VarsDeclList ~ ")")? ~ "=" ~ ProcExpr ~ ";"` declarations
/// starting at `pos` (right after the `proc` keyword), stopping cleanly the first time what
/// follows does not look like another one.
fn scan_proc_spec(bytes: &[u8], mut pos: usize, marks: &mut Marks) -> usize {
    loop {
        let start = skip_trivia(bytes, pos);
        if !bytes.get(start).is_some_and(|&b| is_word_byte(b)) {
            return start;
        }

        let mut after_name = word_end(bytes, start);
        after_name = skip_trivia(bytes, after_name);
        if bytes.get(after_name) == Some(&b'(') {
            after_name = match find_matching_close(bytes, after_name) {
                Some(close) => skip_trivia(bytes, close + 1),
                None => return start,
            };
        }
        if bytes.get(after_name) != Some(&b'=') {
            // Not another `ProcDecl` (likely the next top-level section's keyword) — stop here
            // and let `scan_top_level` re-dispatch from `start`.
            return start;
        }

        let body_start = skip_trivia(bytes, after_name + 1);
        pos = scan_decl_body(bytes, body_start, marks);
    }
}

/// Scans one `ProcExpr` body starting at `pos`, up to (and including) its terminating top-level
/// `;`. Returns the position right after the `;`, or right where it stopped if none is found.
fn scan_decl_body(bytes: &[u8], pos: usize, marks: &mut Marks) -> usize {
    let (end, _) = scan_expr_region(bytes, pos, marks);
    let after = skip_trivia(bytes, end);
    if bytes.get(after) == Some(&b';') {
        after + 1
    } else {
        after
    }
}

/// Scans one `ProcExpr`/`ProcExprNoIf`-shaped region starting at `pos`, up to (but not including)
/// the first top-level `;`, `,`, or unmatched closing bracket — i.e. up to whatever ends the
/// enclosing construct — recursing into nested `ProcExpr`s (bare parentheses, and the last
/// argument of `block`/`allow`/`hide`/`rename`/`comm`) along the way. Returns the position where
/// it stopped, and whether this region is *definitely* not representable as a plain `DataExpr`
/// (see below).
///
/// `chain_start` tracks the leftmost position of the run of `DataExpr`-compatible tokens (atoms
/// joined by `+`/`.`/`||`/…) ending at the current position — exactly what `DataExpr`'s own
/// possessive infix loop would have consumed, greedily, starting from wherever the run began. On
/// every `->` found while some run is open, `chain_start` is exactly the position
/// `ProcExprIfPrefix`/`ProcExprIfThen` would need to be attempted from to reach it, so that is
/// where the marker goes.
///
/// The returned `is_process_only` flag is what lets a caller holding a chain from *before* a
/// nested `(...)` decide whether that chain can extend through it: `DataExprBrackets` only matches
/// when the parenthesized content is, in its entirety, a plain `DataExpr` — so if this scan found
/// process-only syntax (a condition, `sum`/`dist`, a shape operator, or a process-only infix)
/// directly inside those parens, `DataExprBrackets` provably fails there, and `DataExpr`'s own
/// infix loop — greedy, but never partially matching — backtracks out of that whole iteration
/// instead of extending through it. Not extending the chain here does not risk *missing* a
/// condition: whatever the chain right before those parens would have reached is, at worst,
/// something this scanner never needed to reach in the first place (see the module docs on why
/// only the leftmost start of a chain ever needs to be checked).
///
/// Every exit point additionally treats `chain_start == None` as process-only in its own right,
/// even absent any explicit process-only token: it means the *tail* of whatever this scan just
/// looked at (an empty/assignment-shaped call, or simply nothing at all) is not part of any
/// `DataExpr`-reaching run, so the region as a whole has "leftover" content a plain `DataExpr`
/// could never fully consume either — the same reason `DataExprBrackets` cannot wrap it.
///
/// `pending_ifs` implements §1b's `<>`-matching: it holds the "just after `->`" position of every
/// condition found so far *in this region* that has not yet been matched with a `<>`, most-recent
/// last. Finding a bare `<>` token pops and resolves the most recent one — the standard
/// nearest-enclosing-if rule, which is also exactly what a left-to-right scan naturally produces
/// (the innermost/most-recently-opened condition's own greedy tail-attempt would, in the real
/// grammar, reach that same `<>` first, being nested deeper in the parse). It is local to each
/// call/recursion of this function by construction, which is exactly the scoping needed: a `<>`
/// inside a nested `(...)` can only close an `if` that is itself inside those same parens (a
/// self-contained `ProcExpr`), and this function recurses into a *fresh* `scan_expr_region` call
/// for every such nested region, so nothing needs to be done to enforce that boundary — entries
/// left unresolved when a call returns (region ends at `;`, `,`, a closing bracket, or EOF) are
/// simply dropped, correctly leaving those conditions marked as having no `<>`.
fn scan_expr_region(bytes: &[u8], mut pos: usize, marks: &mut Marks) -> (usize, bool) {
    let mut chain_start: Option<usize> = None;
    let mut is_process_only = false;
    let mut pending_ifs: Vec<usize> = Vec::new();

    loop {
        pos = skip_trivia(bytes, pos);
        let Some(&byte) = bytes.get(pos) else {
            return (pos, is_process_only || chain_start.is_none());
        };

        if matches_keyword(bytes, pos, "sum") || matches_keyword(bytes, pos, "dist") {
            is_process_only = true;
            match skip_binder_prefix(bytes, pos) {
                Some(next) => {
                    pos = next;
                    // `ProcExprSum`/`ProcExprDist` are process-only: they can never be part of a
                    // `DataExpr`, so whatever chain was open before them is unreachable from here.
                    // The prefix's own operand starts wherever the next real token is, which the
                    // next loop iteration establishes precisely (after skipping trivia) via
                    // `get_or_insert` — setting it here would instead point at the (to-be-skipped)
                    // trivia right after the prefix's ".", one span position too early.
                    chain_start = None;
                }
                None => return (pos, is_process_only || chain_start.is_none()),
            }
            continue;
        }

        if matches_keyword(bytes, pos, "forall")
            || matches_keyword(bytes, pos, "exists")
            || matches_keyword(bytes, pos, "lambda")
        {
            // Unlike `sum`/`dist`, these *are* `DataExprPrefix` — `DataExprForall`/`Exists`/
            // `Lambda` — so they belong to whatever `DataExpr` chain was already open (or start
            // one, if none was), same as `!`/unary `-`/`#` below, and do not make this region
            // process-only.
            let start = pos;
            match skip_binder_prefix(bytes, pos) {
                Some(next) => {
                    chain_start.get_or_insert(start);
                    pos = next;
                }
                None => return (pos, is_process_only || chain_start.is_none()),
            }
            continue;
        }

        let is_shape_operator = ["block", "allow", "hide", "rename", "comm"]
            .iter()
            .any(|keyword| matches_keyword(bytes, pos, keyword));
        if is_shape_operator && let Some(next) = scan_shape_operator(bytes, pos, marks) {
            // `block(...)`/etc. is never itself a `DataExpr` (its first argument is always a
            // `{...}`-shaped set, never a valid `DataExprPrimary`), so it cannot continue a
            // chain from before it either — same reasoning as the `(` case below.
            is_process_only = true;
            chain_start = None;
            pos = next;
            continue;
        }

        match byte {
            b'(' => {
                let Some(close) = find_matching_close(bytes, pos) else {
                    return (pos, is_process_only || chain_start.is_none());
                };
                let (_, inner_process_only) = scan_expr_region(bytes, pos + 1, marks);
                if inner_process_only {
                    // `DataExprBrackets` requires its entire content to be a plain `DataExpr`;
                    // since it is not, `DataExprBrackets` fails here, so `DataExpr`'s own infix
                    // loop backtracks out of this iteration instead of extending through these
                    // parens — whatever chain was open before them cannot reach past them. And
                    // since these parens are exactly this region's content so far too (whether
                    // they're its entirety or just the current run), this region inherits the
                    // same fact for whoever scans a `(...)` around *it* in turn.
                    chain_start = None;
                    is_process_only = true;
                } else {
                    chain_start.get_or_insert(pos);
                }
                pos = close + 1;
            }
            b'[' | b'{' => {
                // `DataExprListEnum`/`EmptySet`/`EmptyBag`/`BagEnum`/`SetBagComp`/`SetEnum`/
                // `DataExprUpdate`: pure `DataExpr` content that can never itself contain a
                // `ProcExpr` condition, and may contain its own unrelated `->` (`DataExprUpdate`)
                // or `SortExpr` (`SetBagComp`'s `VarDecl`) — always skipped whole, never scanned.
                let Some(close) = find_matching_close(bytes, pos) else {
                    return (pos, is_process_only || chain_start.is_none());
                };
                chain_start.get_or_insert(pos);
                pos = close + 1;
            }
            b';' | b',' | b')' | b']' | b'}' => return (pos, is_process_only || chain_start.is_none()),
            // Process-only tokens: `DataExpr`'s own grammar could never have matched through any
            // of these (either they aren't a `DataExprInfix` token at all, or — for `<<`, which
            // overlaps with a leading `DataExprLess` — matching just the `<` leaves nothing able
            // to match the rest, so the whole infix attempt backtracks out). The chain therefore
            // provably ends here, same as at a `sum`/`dist` prefix, and this region is thereby
            // proven process-only.
            b'<' if bytes[pos..].starts_with(b"<<") => {
                pos += 2; // ProcExprUntil
                chain_start = None;
                is_process_only = true;
            }
            b'<' if bytes[pos..].starts_with(b"<>") => {
                pos += 2; // else branch of a condition
                chain_start = None;
                is_process_only = true;
                // §1b: this closes the *nearest* still-open condition in this region, if any —
                // see `pending_ifs`'s doc comment above. If none is open, this `<>` does not
                // belong to anything this scan is tracking (e.g. malformed input, or a condition
                // this scanner failed to recognize) — leaving it unresolved is safe, it just means
                // that one condition's optional tail does not get gated.
                if let Some(arrow_end) = pending_ifs.pop() {
                    marks.push((arrow_end, ELSE_MARKER));
                }
            }
            b'|' if bytes[pos..].starts_with(b"||_") => {
                pos += 3; // ProcExprLeftMerge
                chain_start = None;
                is_process_only = true;
            }
            b'|' if bytes[pos..].starts_with(b"||") => {
                pos += 2; // DataExprDisj / ProcExprParallel: shared, continues the chain.
            }
            b'|' if bytes[pos..].starts_with(b"|>") => {
                pos += 2; // DataExprCons: DataExpr-only, but still continues the chain.
            }
            b'|' => {
                pos += 1; // ProcExprSync
                chain_start = None;
                is_process_only = true;
            }
            b'@' => {
                // ProcExprAt's operand is `DataExprUnit`, which has no infix loop of its own: one
                // atom, then back to plain ProcExpr with no chain carried through the `@`.
                is_process_only = true;
                pos = skip_trivia(bytes, pos + 1);
                pos = match bytes.get(pos) {
                    Some(b'(' | b'[' | b'{') => match find_matching_close(bytes, pos) {
                        Some(close) => close + 1,
                        None => return (pos, is_process_only || chain_start.is_none()),
                    },
                    Some(&b) if is_word_byte(b) => word_end(bytes, pos),
                    _ => return (pos, is_process_only || chain_start.is_none()),
                };
                chain_start = None;
            }
            b'-' if bytes.get(pos + 1) == Some(&b'>') => {
                is_process_only = true;
                if let Some(start) = chain_start {
                    marks.push((start, CONDITION_MARKER));
                    // The position right after this `->` is where `ProcExprElseMarker` would need
                    // to go if a matching `<>` turns out to exist — recorded now, resolved (or
                    // not) by the `<>` case above.
                    pending_ifs.push(pos + 2);
                }
                pos += 2;
                chain_start = None;
            }

            // Everything below is `DataExpr`-only vocabulary (never used by `ProcExpr`), so it
            // never causes the ambiguity markers exist for, and never makes this region
            // process-only — but the scanner still has to recognize it, or it wrongly treats it
            // as unknown and stops tracking the chain.

            // `DataExprPrefix`: belongs to whatever chain was already open, or starts one.
            b'!' if bytes.get(pos + 1) != Some(&b'=') => {
                chain_start.get_or_insert(pos);
                pos += 1;
            }
            b'#' => {
                chain_start.get_or_insert(pos);
                pos += 1;
            }
            b'-' => {
                chain_start.get_or_insert(pos);
                pos += 1;
            }
            // `whr AssignmentList end` (a postfix, so it never starts a chain on its own, but does
            // not end one either): its assignments are pure `DataExpr`, opaque like `[...]`/
            // `{...}`, and can themselves nest another `whr ... end`.
            b'w' if matches_keyword(bytes, pos, "whr") => match skip_whr(bytes, pos) {
                Some(next) => pos = next,
                None => return (pos, is_process_only || chain_start.is_none()),
            },
            // Binary `DataExprInfix` tokens: continue whatever chain was already open.
            b'=' if bytes[pos..].starts_with(b"=>") => pos += 2,
            b'=' if bytes[pos..].starts_with(b"==") => pos += 2,
            b'!' => pos += 2, // already known to start "!=" from the guarded arm above
            b'&' if bytes[pos..].starts_with(b"&&") => pos += 2,
            b'<' if bytes[pos..].starts_with(b"<=") => pos += 2,
            b'<' if bytes[pos..].starts_with(b"<|") => pos += 2,
            b'<' => pos += 1,
            b'>' if bytes[pos..].starts_with(b">=") => pos += 2,
            b'>' => pos += 1,
            b'+' if bytes[pos..].starts_with(b"++") => pos += 2,
            b'+' | b'.' => pos += 1, // DataExprAdd/At and ProcExprChoice/Seq: shared.
            b'/' | b'*' => pos += 1,

            b if is_word_byte(b) => {
                let atom_start = pos;
                let after_word = word_end(bytes, pos);
                let call_start = skip_trivia(bytes, after_word);
                chain_start.get_or_insert(atom_start);
                if bytes.get(call_start) == Some(&b'(') {
                    let Some(close) = find_matching_close(bytes, call_start) else {
                        return (pos, is_process_only || chain_start.is_none());
                    };
                    // `Action`/`ProcExprId`'s argument list is `DataExprList`/`AssignmentList` —
                    // pure `DataExpr` content, never a `ProcExpr`, so this is opaque like
                    // `[...]`/`{...}` either way, and always fully skipped over: pest itself never
                    // pauses to attempt a fresh `ProcExprPrefix*` in the middle of one identifier
                    // token and its own immediately-following call parens, so a marker can never
                    // legally go there either — leaving it unskipped for the main loop to
                    // reprocess would place one at exactly such an invalid position. But
                    // `DataExprApplication` only accepts the `DataExprList` shape (comma-separated
                    // plain `DataExpr`s, at least one) — empty parens, or assignment-shorthand
                    // parens (`x=y, ...`), are valid `Action`/`ProcExprId` syntax that
                    // `DataExprApplication` rejects. `DataExpr`'s postfix loop then does not
                    // consume these parens at all, and so cannot reach anything past them either —
                    // matching this call's own bare name (already recorded above) is as far as any
                    // chain through it can go, hence resetting `chain_start` (but still moving
                    // `pos` past the parens).
                    if !looks_like_data_expr_list(bytes, call_start, close) {
                        chain_start = None;
                    }
                    pos = close + 1;
                } else {
                    pos = after_word;
                }
            }

            // Not a token this scanner specifically recognizes. Rather than stop scanning the
            // whole region here (which, before this fallback existed, silently truncated the scan
            // — and with it every condition later in the file — the moment it saw any token this
            // match didn't already list), treat it conservatively as a hard chain boundary and
            // keep going: worst case this one position's chain-tracking is imprecise, instead of
            // every later condition going undetected.
            _ => {
                chain_start = None;
                pos += 1;
            }
        }
    }
}

/// Skips a `sum`/`dist`/`forall`/`exists`/`lambda` prefix's `VarsDeclList` (and, for `dist`, the
/// following `"[" DataExpr "]"`), up to and including the terminating `"."`. This is always
/// opaque: its `SortExpr`s can contain their own unrelated `->` (`SortExprFunction`), which must
/// never be mistaken for a condition arrow.
fn skip_binder_prefix(bytes: &[u8], pos: usize) -> Option<usize> {
    // `dist`'s trailing `"[" DataExpr "]"` is bracketed, so the generic bracket-skip below already
    // treats it as opaque before the terminating "." — nothing further to special-case for it.
    let mut i = word_end(bytes, pos);

    loop {
        i = skip_trivia(bytes, i);
        match bytes.get(i)? {
            b'(' | b'[' | b'{' => i = find_matching_close(bytes, i)? + 1,
            b'.' => return Some(i + 1),
            _ => i += 1,
        }
    }
}

/// Scans a `block`/`allow`/`hide`/`rename`/`comm` call at `pos` (positioned at the keyword):
/// `keyword ~ "(" ~ <curly-braced set> ~ "," ~ ProcExpr ~ ")"`. The set is always `{...}`-shaped
/// and opaque; the trailing `ProcExpr` is a fresh nested region, scanned recursively. Returns the
/// position right after the whole call, or `None` if the expected shape is not there.
fn scan_shape_operator(bytes: &[u8], pos: usize, marks: &mut Marks) -> Option<usize> {
    let mut i = skip_trivia(bytes, word_end(bytes, pos));
    if bytes.get(i) != Some(&b'(') {
        return None;
    }
    i = skip_trivia(bytes, i + 1);
    if bytes.get(i) != Some(&b'{') {
        return None;
    }
    i = skip_trivia(bytes, find_matching_close(bytes, i)? + 1);
    if bytes.get(i) != Some(&b',') {
        return None;
    }
    i = skip_trivia(bytes, i + 1);

    let (end, _) = scan_expr_region(bytes, i, marks);
    if bytes.get(end) == Some(&b')') {
        Some(end + 1)
    } else {
        None
    }
}

/// Skips a `whr AssignmentList end`, which can itself nest another `whr ... end` (its assignments
/// are plain `DataExpr`s, which can carry their own `whr` postfix) — tracked the same way bracket
/// nesting is, just keyed on the keywords instead of punctuation.
fn skip_whr(bytes: &[u8], pos: usize) -> Option<usize> {
    let mut depth = 1u32;
    let mut i = word_end(bytes, pos);
    loop {
        let start = skip_trivia(bytes, i);
        if matches_keyword(bytes, start, "whr") {
            depth += 1;
            i = word_end(bytes, start);
        } else if matches_keyword(bytes, start, "end") {
            depth -= 1;
            i = word_end(bytes, start);
            if depth == 0 {
                return Some(i);
            }
        } else {
            match *bytes.get(start)? {
                b'(' | b'[' | b'{' => i = find_matching_close(bytes, start)? + 1,
                _ if is_word_byte(bytes[start]) => i = word_end(bytes, start),
                _ => i = start + 1,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Marks `source` and renders the result as `original text` with `#` standing in for each
    /// [CONDITION_MARKER] and `$` for each [ELSE_MARKER], which makes the expected strings in
    /// these tests plain ASCII.
    fn marked(source: &str) -> String {
        let (text, _) = mark_process_conditions(source);
        text.replace(CONDITION_MARKER, "#").replace(ELSE_MARKER, "$")
    }

    #[test]
    fn test_marks_a_plain_condition() {
        assert_eq!(
            marked("proc P = a -> b <> c; init P;"),
            "proc P = #a ->$ b <> c; init P;"
        );
    }

    #[test]
    fn test_marks_the_whole_chain_as_the_condition() {
        // Matches pest's own greedy, leftmost-first behaviour: `a+b` is one `DataExpr`, not `a`
        // choice-joined with a separate condition `b -> ...`.
        assert_eq!(marked("init a+b -> c <> d;"), "init #a+b ->$ c <> d;");
    }

    #[test]
    fn test_marks_after_a_process_only_operator() {
        assert_eq!(marked("init a || (b -> c <> d);"), "init a || (#b ->$ c <> d);");
    }

    #[test]
    fn test_marks_after_choice() {
        assert_eq!(marked("init a + (b -> c <> d);"), "init a + (#b ->$ c <> d);");
    }

    #[test]
    fn test_no_condition_no_marks() {
        assert_eq!(marked("init a + b + c;"), "init a + b + c;");
    }

    #[test]
    fn test_ignores_sort_function_arrow_in_binder() {
        assert_eq!(
            marked("proc P = sum f: Nat -> Bool . f(0) -> P <> delta; init P;"),
            "proc P = sum f: Nat -> Bool . #f(0) ->$ P <> delta; init P;"
        );
    }

    #[test]
    fn test_ignores_data_expr_update() {
        assert_eq!(marked("init a[x -> y](1) -> b <> c;"), "init #a[x -> y](1) ->$ b <> c;");
    }

    #[test]
    fn test_ignores_arrow_in_eqn_spec() {
        assert_eq!(marked("eqn f -> g = h;"), "eqn f -> g = h;");
    }

    #[test]
    fn test_ignores_arrow_in_sort_spec() {
        assert_eq!(marked("sort F = Nat -> Bool; init a;"), "sort F = Nat -> Bool; init a;");
    }

    #[test]
    fn test_marks_inside_block_operand() {
        assert_eq!(
            marked("init block({a}, x -> P <> delta);"),
            "init block({a}, #x ->$ P <> delta);"
        );
    }

    #[test]
    fn test_marks_nested_condition_in_else_branch() {
        assert_eq!(
            marked("init a -> b <> (c -> d <> e);"),
            "init #a ->$ b <> (#c ->$ d <> e);"
        );
    }

    #[test]
    fn test_marks_multiple_proc_decls() {
        assert_eq!(
            marked("proc P = a -> b <> c; Q = d -> e <> f; init P;"),
            "proc P = #a ->$ b <> c; Q = #d ->$ e <> f; init P;"
        );
    }

    #[test]
    fn test_action_arguments_are_opaque() {
        // The nested call `g(x)` inside `f(...)`'s argument list must not confuse the bracket
        // matching that skips over the (opaque, DataExpr-only) argument list as a whole.
        assert_eq!(marked("init f(g(x)) -> h <> i;"), "init #f(g(x)) ->$ h <> i;");
    }

    // `DataExprPrefix` tokens (`!`, unary `-`, `#`, `forall`/`exists`/`lambda`) were originally not
    // recognized at all, which made the scanner bail out (silently truncating the whole scan) the
    // moment it saw one anywhere in a `proc`/`init` body — this was the actual cause of every
    // observed regression, not sparseness in the condition search itself.

    #[test]
    fn test_negation_prefix_does_not_stop_the_scan() {
        assert_eq!(marked("init (!b -> P <> Q) . c;"), "init (#!b ->$ P <> Q) . c;");
    }

    #[test]
    fn test_unary_minus_and_size_prefixes_do_not_stop_the_scan() {
        assert_eq!(
            marked("init (-n == #l -> P <> Q) . c;"),
            "init (#-n == #l ->$ P <> Q) . c;"
        );
    }

    #[test]
    fn test_exists_prefix_does_not_stop_the_scan() {
        assert_eq!(
            marked("init (exists n:Nat . n > 0) -> P <> Q;"),
            "init #(exists n:Nat . n > 0) ->$ P <> Q;"
        );
    }

    #[test]
    fn test_prefix_after_process_only_construct_still_scans() {
        // Regression case distilled from `examples/mCRL2/industrial/DIRAC/WMS.mcrl2`'s `Monitor`
        // process: a `!`-negated condition following a `.`/`+`-chain must not be missed.
        assert_eq!(marked("init tau.P + (!b -> Q <> R);"), "init tau.P + (#!b ->$ Q <> R);");
    }

    // A chain does not extend through a parenthesized operand whose content is not itself a plain
    // `DataExpr` (i.e. contains process-only syntax): `DataExprBrackets` fails to match such
    // parens, so `DataExpr`'s own infix loop backtracks out instead of extending through them.
    // Getting this wrong previously let a stale, unrelated chain start survive an entire
    // if-then-else and wrongly attach to a `->` much later in the same operand chain — distilled
    // from `examples/mCRL2/academic/swp/swp_func.mcrl2`'s `R` process.

    #[test]
    fn test_chain_does_not_extend_through_a_process_only_parenthesized_operand() {
        // `a(1)`'s chain correctly finds no `->` (it cannot extend through the process-only
        // parens that follow it), so it is not marked; `b(2)`'s chain — starting fresh right
        // after those parens — is the one that actually reaches the final `->`.
        assert_eq!(
            marked("init a(1).((c -> P <> Q))+b(2) -> R <> S;"),
            "init a(1).((#c ->$ P <> Q))+#b(2) ->$ R <> S;"
        );
    }

    #[test]
    fn test_chain_does_not_extend_through_a_process_only_shape_operator() {
        assert_eq!(
            marked("init a.block({x}, c -> P <> Q)+b -> R <> S;"),
            "init a.block({x}, #c ->$ P <> Q)+#b ->$ R <> S;"
        );
    }

    // `Action`/`ProcExprId` accept two argument-list shapes `DataExprApplication` does not: empty
    // (`f()`) and assignment-shorthand (`f(x=y)`). `DataExpr`'s postfix loop cannot consume either,
    // so a chain cannot reach through them (in either direction: into them from before, or out of
    // them to something after) — distilled from `examples/mCRL2/academic/peterson_justness/mutex.mcrl2`'s
    // `Turn` process, where getting this wrong let a stale chain from much earlier "skip over" an
    // intervening `f()` and wrongly attach to a `->` on the far side of it.

    #[test]
    fn test_empty_call_does_not_extend_a_chain() {
        assert_eq!(marked("init a.f() -> P <> Q;"), "init a.f() -> P <> Q;");
    }

    #[test]
    fn test_assignment_shorthand_call_does_not_extend_a_chain() {
        assert_eq!(marked("init a.f(x=y) -> P <> Q;"), "init a.f(x=y) -> P <> Q;");
    }

    #[test]
    fn test_equality_comparison_call_args_are_not_mistaken_for_assignment_shorthand() {
        // Regression: the second "=" of "==" was checked as if it might be a lone assignment "="
        // in its own right (only the *first* character of "==", "!=", "<=", ">=" was excluded, not
        // the second) — distilled from `examples/mCRL2/academic/onebit/onebit.mcrl2`'s `S` process,
        // where this wrongly cut a chain short right before "S(b2==p,...)  + !sts -> ...".
        assert_eq!(
            marked("init a.f(b==c) + d -> P <> Q;"),
            "init #a.f(b==c) + d ->$ P <> Q;"
        );
    }

    #[test]
    fn test_fresh_chain_starts_right_after_an_empty_call() {
        assert_eq!(marked("init a.f()+b -> P <> Q;"), "init a.f()+#b ->$ P <> Q;");
    }

    #[test]
    fn test_mutex_turn_process_condition_after_greedy_dataexpr_prefix() {
        // The whole run `r_assign_turnA.Turn(A) + r_assign_turnB.Turn(B)` is one valid `DataExpr`
        // (`Turn(A)`/`Turn(B)` are non-empty `DataExprApplication`s), so pest's own greedy,
        // leftmost-first `DataExprIfPrefix` attempt absorbs it into the *first* condition exactly
        // as this scanner predicts — matching the source's actual structure requires the *second*
        // condition to still be found fresh, right after the first one's `then` branch ends at the
        // empty-argument `Turn()`.
        assert_eq!(
            marked(
                "proc Turn(t: TurnType) =\n    r_assign_turnA.Turn(A)\n  + r_assign_turnB.Turn(B)\n  + (t==A) -> s_read_turnA|label(a_read_turnA).Turn()\n  + (t==B) -> s_read_turnB|label(a_read_turnB).Turn();\ninit Turn(A);\n"
            ),
            "proc Turn(t: TurnType) =\n    #r_assign_turnA.Turn(A)\n  + r_assign_turnB.Turn(B)\n  + (t==A) -> s_read_turnA|label(a_read_turnA).Turn()\n  + #(t==B) -> s_read_turnB|label(a_read_turnB).Turn();\ninit Turn(A);\n"
        );
    }

    #[test]
    fn test_a_parenthesized_condition_does_not_extend_a_chain_across_an_assignment_shorthand_then_branch() {
        // Regression: `is_process_only` was only ever set from an *explicit* process-only token
        // (a condition, `sum`/`dist`, a process-only infix); it was not also set when a scan
        // simply ran out of chain (`chain_start == None`) right at its own terminator — e.g.
        // because the last thing in it was an assignment-shorthand call. That let `(cond1) ->
        // (then1)` be wrongly treated as one `DataExprBrackets`-compatible atom continuing into a
        // *second*, unrelated `(cond2) -> ...`, distilled from
        // `examples/mCRL2/industrial/ieee-11073/11073.mcrl2`'s `Agent` process (`then1` there is
        // `transport_connect_agent . Agent(Agent_state = Agent_Connected)`, whose assignment
        // shorthand call is exactly the kind of content `DataExpr` cannot fully consume).
        assert_eq!(
            marked("init (a == b) -> (c . f(x = y)) + (d == e) -> g <> h;"),
            "init #(a == b) -> (c . f(x = y)) + #(d == e) ->$ g <> h;"
        );
    }

    // §1b regression tests: `ProcExprIfPrefix`'s optional `(ProcExprElseMarker ~ ProcExprNoIf ~
    // "<>")?` tail (and `ProcExprIfThen`'s mandatory one) must only be attempted when a matching
    // `<>` genuinely exists — see the module docs and docs/procexpr-parsing-performance-plan.md
    // §1b for why an ungated attempt is not just quadratic but exponential.

    #[test]
    fn test_bare_condition_without_else_gets_no_else_marker() {
        assert_eq!(marked("init a -> b; init c;"), "init #a -> b; init c;");
    }

    #[test]
    fn test_condition_with_else_gets_an_else_marker() {
        assert_eq!(marked("init a -> b <> c;"), "init #a ->$ b <> c;");
    }

    #[test]
    fn test_a_chain_of_many_bare_conditions_gets_no_else_markers() {
        // The `MLV.mcrl2` shape that originally exposed §1b: a long `+`-chain where every term is
        // its own condition with no `<>` at all. None of them should get an `ELSE_MARKER` — every
        // one of their optional tails must fail in O(1) rather than walk the rest of the chain.
        // (The second and third `CONDITION_MARKER`s land on the *preceding* `p(N)` call rather than
        // on the `(a==N)` that follows it — unrelated to this test, but expected: `p(0) + (a==1)`
        // is itself one continuous plain-`DataExpr`-shaped run reaching the next `->`, so per the
        // leftmost-subsumption rule the marker goes at its own leftmost position, same as
        // `test_marks_the_whole_chain_as_the_condition` above.)
        assert_eq!(
            marked("init (a==0) -> p(0) + (a==1) -> p(1) + (a==2) -> p(2);"),
            "init #(a==0) -> #p(0) + (a==1) -> #p(1) + (a==2) -> p(2);"
        );
    }

    #[test]
    fn test_else_binds_to_the_nearest_enclosing_unparenthesized_if() {
        // Standard dangling-else resolution: with no parentheses to disambiguate, a `<>` closes
        // the *most recently opened* still-open condition, not an earlier one in the same chain —
        // so only `cond2` gets an `ELSE_MARKER` here, matching what pest's own greedy, depth-first
        // matching would resolve to (`cond2`'s own tail-attempt reaches the `<>` before `cond1`'s
        // ever would). (The second `CONDITION_MARKER` lands on `A`, not `cond2`, for the same
        // leftmost-subsumption reason as the test above: `A + cond2` is one continuous run.)
        assert_eq!(
            marked("init cond1 -> A + cond2 -> B <> C;"),
            "init #cond1 -> #A + cond2 ->$ B <> C;"
        );
    }

    #[test]
    fn test_else_marker_scope_does_not_cross_a_closing_bracket() {
        // A `<>` inside `(...)` can only close an `if` that is itself inside those same parens —
        // `cond1`'s own chain must not be resolved by `cond2`'s `<>`, which is on the far side of
        // an intervening `)`.
        assert_eq!(
            marked("init cond1 -> (cond2 -> A <> B);"),
            "init #cond1 -> (#cond2 ->$ A <> B);"
        );
    }
}
