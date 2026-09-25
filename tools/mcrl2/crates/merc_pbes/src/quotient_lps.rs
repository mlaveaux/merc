use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write as _;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;

use itertools::Itertools;
use log::debug;
use log::info;
use merc_explore::LPS;
use merc_explore::StateEffect;
use merc_explore::Summand;
use merc_utilities::MercError;

use crate::bsgs::Bsgs;
use crate::bsgs::CanonicalizeContext;
use crate::bsgs::permutation_to_gap_cycles;
use crate::explore_common::ParameterLayoutLPS;
use crate::graph_symmetry::GapConfig;
use crate::permutation::Permutation;

/// A persistent GAP session that computes lex-min orbit representatives under a
/// precomputed group. Kept behind an [`Arc`] in [`Canonicaliser::GapLexmin`] so
/// it can be shared (and serialised) across the exploration threads.
pub struct GapLexminSession {
    child: Mutex<std::process::Child>,
    n: usize,
}

impl GapLexminSession {
    fn start(gens: &[Permutation], n: usize, config: &GapConfig) -> Self {
        let mut child = Command::new(&config.executable)
            .args(["-q", "-A", "--quitonbreak"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to start GAP executable '{}': {e}", config.executable));

        let mut stdin = child.stdin.take().expect("GAP stdin should be piped");

        let gens_joined = if gens.is_empty() {
            String::from("()")
        } else {
            gens.iter().map(|p| permutation_to_gap_cycles(p, n)).join(", ")
        };
        let setup = format!("G := Group({gens_joined});; elems := Elements(G);; Print(\"LEXMIN-READY\\n\");\n");
        debug!("GAP setup command: {}", setup);
        stdin.write_all(setup.as_bytes()).expect("write to GAP stdin");
        stdin.flush().expect("flush GAP stdin");

        // Read until the setup sentinel while stdin is still open.
        let mut stdout = child.stdout.take().expect("GAP stdout should be piped");
        let _ready = read_until_sentinel(&mut stdout, "LEXMIN-READY").expect("GAP session startup failed");

        // Put stdin and stdout back into the Child so they survive for later
        // queries.
        child.stdin = Some(stdin);
        child.stdout = Some(stdout);

        info!(
            "GAP lex-min session started: {n} parameter(s), {} generator(s)",
            gens.len()
        );
        GapLexminSession {
            child: Mutex::new(child),
            n,
        }
    }

    fn lex_min(&self, state: &[usize], param_offset: usize) -> Vec<usize> {
        let params = &state[param_offset..];
        let params_str = format!("[{}]", params.iter().join(","));

        let mut child = self.child.lock().expect("GAP session lock poisoned");
        let mut stdin = child.stdin.take().expect("GAP stdin should be available");
        let mut stdout = child.stdout.take().expect("GAP stdout should be available");

        let query = format!(
            "Print(\"LEXMIN-BEGIN\\n\"); Print(Minimum(List(elems, g -> Permuted({params_str}, g))), \"\\n\"); Print(\"LEXMIN-END\\n\");\n"
        );
        debug!("GAP lex-min query: {}", query);
        stdin.write_all(query.as_bytes()).expect("write lex-min query");
        stdin.flush().expect("flush lex-min query");

        // Put stdin back so the child stays alive.
        child.stdin = Some(stdin);

        let response = read_until_sentinel(&mut stdout, "LEXMIN-END").expect("GAP lex-min response not found");

        // Put stdout back for the next query.
        child.stdout = Some(stdout);

        parse_lex_min_response(&response, self.n, param_offset, state)
    }
}

impl Drop for GapLexminSession {
    fn drop(&mut self) {
        let child = self.child.get_mut().expect("GAP session lock poisoned");
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(b"quit;\n");
            let _ = stdin.flush();
        }
        let _ = child.wait();
    }
}

/// Read from `reader` until ` sentinel` is found.  Returns the accumulated
/// output **before** the sentinel, or `None` if EOF was reached first.
fn read_until_sentinel<R: Read>(reader: &mut R, sentinel: &str) -> std::io::Result<String> {
    let mut buf = String::new();
    let mut tmp = [0u8; 1024];
    loop {
        let n = reader.read(&mut tmp)?;
        if n == 0 {
            return Err(Error::new(
                ErrorKind::UnexpectedEof,
                format!("{sentinel} not found in: {buf}"),
            ));
        }

        // SAFETY: GAP only produces ASCII output.
        buf.push_str(&String::from_utf8_lossy(&tmp[..n]));
        if let Some(pos) = buf.find(sentinel) {
            debug!("GAP response before sentinel '{}': {}", sentinel, &buf[..pos]);
            return Ok(buf[..pos].to_string());
        }
    }

    // Unreachable, sentinel not found.
}

/// Parse the flat integer list that GAP printed between `LEXMIN-BEGIN` and
/// `LEXMIN-END`, and reconstruct the full state vector (prefix + canonicalised
/// parameter block).
fn parse_lex_min_response(response: &str, n: usize, param_offset: usize, original_state: &[usize]) -> Vec<usize> {
    // Discard everything up to and including the leading `LEXMIN-BEGIN` marker;
    // the remaining text holds the bare `[ ... ]` list printed by GAP.
    let after_marker = response
        .find("LEXMIN-BEGIN")
        .map(|pos| &response[pos + "LEXMIN-BEGIN".len()..])
        .unwrap_or(response);

    let flat: String = after_marker
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '[' && *c != ']')
        .collect();

    let min_params: Vec<usize> = flat
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|entry| {
            entry
                .parse::<usize>()
                .unwrap_or_else(|_| panic!("expected a number from GAP, got: {entry}"))
        })
        .collect();

    assert_eq!(min_params.len(), n, "GAP returned a minimum of the wrong degree");

    let mut result = original_state[..param_offset].to_vec();
    result.extend_from_slice(&min_params);
    result
}

/// Strategy for canonicalizing states to their orbit representative.
///
/// Either uses a precomputed BSGS (stabilizer chain) for fast in-process
/// canonicalization, or invokes GAP's lex-min computation via an external
/// process for each state.
pub enum Canonicaliser {
    /// Canonicalize via a precomputed stabilizer chain.
    Bsgs(Arc<Bsgs>),

    /// Canonicalize by invoking GAP's `Minimum(List(Elements(G), g -> Permuted(s, g)))` for each state.
    ///
    /// The GAP process is started once with the group and its elements
    /// precomputed; each `canonicalize` call only sends the state.
    GapLexmin(Arc<GapLexminSession>),
}

impl Canonicaliser {
    /// Canonicalize `state` to the lexicographically smallest orbit
    /// representative, writing the result into `out`.
    ///
    /// `param_offset` is the first index in `state` that belongs to the
    /// permuted parameter block; positions before it are copied through.
    pub fn canonicalize(&self, state: &[usize], param_offset: usize, out: &mut Vec<usize>) {
        match self {
            Canonicaliser::Bsgs(bsgs) => {
                bsgs.canonicalize_into(state, param_offset, &mut CanonicalizeContext::default(), out);
            }
            Canonicaliser::GapLexmin(session) => {
                let result = session.lex_min(state, param_offset);
                out.clear();
                out.extend_from_slice(&result);
            }
        }
    }

    /// The degree of the permutation group (number of parameters acted on).
    pub fn degree(&self) -> usize {
        match self {
            Canonicaliser::Bsgs(bsgs) => bsgs.n,
            Canonicaliser::GapLexmin(session) => session.n,
        }
    }

    /// Start a persistent GAP lex-min session.
    pub fn gap_lexmin(gens: Vec<Permutation>, n: usize, config: &GapConfig) -> Self {
        let session = GapLexminSession::start(&gens, n, config);
        Canonicaliser::GapLexmin(Arc::new(session))
    }
}

/// Wraps any `LPS<Value = usize>` and canonicalizes every enumerated next-state
/// to the lexicographically smallest orbit representative before passing it to
/// the caller.
///
/// State vectors are laid out as `[eq_idx_0..eq_idx_{offset-1}, param_0..param_{n-1}]`
/// where only positions `param_offset..` are touched by the group action.  Position 0
/// (the equation index) is never permuted.
///
/// # Wrapping order
///
/// When combined with [`merc_explore::CacheLPS`], place the cache *inside* and the
/// quotient *outside*:
/// ```text
/// QuotientLps<CacheLPS<PbesLps>>
/// ```
/// This keeps cache keys narrow (raw, un-canonicalized write positions) and avoids
/// forcing the cache to track all parameters as written.
pub struct QuotientLps<P: ParameterLayoutLPS<Value = usize>> {
    inner: Arc<P>,
    canonicaliser: Arc<Canonicaliser>,
    summands: Vec<QuotientSummand<P>>,
    param_offset: usize,
}

/// A single summand of a [`QuotientLps`].
///
/// Delegates enumeration to the corresponding inner summand and canonicalizes
/// each next-state before reporting it.
pub struct QuotientSummand<P: ParameterLayoutLPS<Value = usize>> {
    index: usize,
    inner: Arc<P>,
    canonicaliser: Arc<Canonicaliser>,
    param_offset: usize,
    read_positions: Vec<usize>,
}

/// Per-thread enumeration context for a [`QuotientLps`].
pub struct QuotientContext<P: ParameterLayoutLPS<Value = usize>> {
    inner: <P::Summand as Summand>::Context,

    /// Working buffers of [`Bsgs::canonicalize_into`], so that canonicalizing a
    /// next state costs no allocation. Used only by the BSGS variant; ignored
    /// by the GAP lex-min variant.
    scratch: CanonicalizeContext,

    /// The canonicalized next state handed to the caller's callback.
    canonical: Vec<usize>,
}

// SAFETY: Neither struct has interior mutability of its own (no UnsafeCell /
// raw pointers). All concurrent access is read-only via `&self`. The only
// non-trivially-Sync field is `Arc<P>`: sharing `&Arc<P>` across threads only
// requires `P: Sync` (which the bound enforces). The stdlib's conservative
// `impl<T: Send + Sync> Sync for Arc<T>` also requires `T: Send` to handle the
// last Arc being dropped on a foreign thread, but `QuotientLps` is not `Send`,
// so that case cannot arise. `Arc<Bsgs>` is unconditionally fine because `Bsgs`
// contains only `usize`, `Vec`, and `HashMap` of plain data, all auto-`Sync`.
unsafe impl<P: ParameterLayoutLPS<Value = usize> + Sync> Sync for QuotientLps<P> {}
unsafe impl<P: ParameterLayoutLPS<Value = usize> + Sync> Sync for QuotientSummand<P> {}

impl<P> QuotientLps<P>
where
    P: ParameterLayoutLPS<Value = usize>,
{
    /// Wraps `inner` in a canonicalizing quotient layer.
    ///
    /// `param_offset` is the first position in the state vector that belongs to
    /// the PBES parameters (always `1` for `PbesSrfLps`, where position 0 is the
    /// equation index).
    pub fn new(inner: P, canonicaliser: Arc<Canonicaliser>, param_offset: usize) -> Self {
        let inner = Arc::new(inner);

        let summands = inner
            .summands()
            .iter()
            .enumerate()
            .map(|(i, s)| QuotientSummand {
                index: i,
                inner: Arc::clone(&inner),
                canonicaliser: Arc::clone(&canonicaliser),
                param_offset,
                read_positions: s.read_positions().to_vec(),
            })
            .collect();

        QuotientLps {
            inner,
            canonicaliser,
            summands,
            param_offset,
        }
    }
}

impl<P> LPS for QuotientLps<P>
where
    P: ParameterLayoutLPS<Value = usize>,
{
    type Value = usize;
    type Label = P::Label;
    type StateInfo = P::StateInfo;
    const HAS_LABELS: bool = P::HAS_LABELS;
    type Summand = QuotientSummand<P>;

    fn initial_state(&self) -> Vec<usize> {
        let mut out = Vec::new();
        self.canonicaliser
            .canonicalize(&self.inner.initial_state(), self.param_offset, &mut out);
        out
    }

    fn summands(&self) -> &[Self::Summand] {
        &self.summands
    }

    fn create_context(&self) -> QuotientContext<P> {
        QuotientContext {
            inner: self.inner.create_context(),
            scratch: CanonicalizeContext::default(),
            canonical: Vec::new(),
        }
    }

    fn prepare<'a>(&'a self, context: &mut QuotientContext<P>, state: &'a [usize]) -> impl Iterator<Item = usize> + 'a {
        self.inner.prepare(&mut context.inner, state)
    }

    fn state_info(&self, state: &[usize], context: &QuotientContext<P>) -> P::StateInfo {
        self.inner.state_info(state, &context.inner)
    }
}

impl<P> Summand for QuotientSummand<P>
where
    P: ParameterLayoutLPS<Value = usize>,
{
    type Value = usize;
    type Label = P::Label;
    type Context = QuotientContext<P>;

    fn read_positions(&self) -> &[usize] {
        &self.read_positions
    }

    fn effect(&self) -> StateEffect<'_> {
        // Canonicalization can move a value to any parameter position, and the
        // states it passes through unchanged are of other lengths entirely.
        StateEffect::Opaque
    }

    fn enumerate<F>(&self, context: &mut Self::Context, state: &[usize], mut report: F) -> Result<(), MercError>
    where
        F: FnMut(&Self::Label, &[usize]) -> Result<(), MercError>,
    {
        let canonicaliser = &self.canonicaliser;
        let inner = &*self.inner;
        let param_offset = self.param_offset;

        // Destructured so the closure can borrow the canonicalization buffers
        // while the inner summand holds its own context.
        let QuotientContext {
            inner: inner_context,
            scratch,
            canonical,
        } = context;

        self.inner.summands()[self.index].enumerate(inner_context, state, |label, next| {
            match inner.parameter_range(next) {
                Some(range) => {
                    debug_assert_eq!(
                        range.start, param_offset,
                        "the parameter block must start where the group acts"
                    );
                    debug_assert_eq!(
                        range.len(),
                        canonicaliser.degree(),
                        "the group must act on the whole parameter block"
                    );
                    match &**canonicaliser {
                        Canonicaliser::Bsgs(bsgs) => {
                            bsgs.canonicalize_into(next, param_offset, scratch, canonical);
                        }
                        Canonicaliser::GapLexmin(_) => {
                            Canonicaliser::canonicalize(canonicaliser, next, param_offset, canonical);
                        }
                    }
                    report(label, canonical)
                }
                // Sinks and subformula vertices carry no data parameters, so the
                // group does not act on them; permuting their payload would
                // corrupt a priority or an interned formula index.
                None => report(label, next),
            }
        })
    }
}
