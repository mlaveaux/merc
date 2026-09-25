use std::collections::VecDeque;
use std::fmt::Debug;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use crossbeam_deque::Steal;
use crossbeam_deque::Stealer;
use crossbeam_deque::Worker;
use crossbeam_utils::Backoff;
use rand::RngExt;
use rand::SeedableRng;
use rand::rngs::SmallRng;

use merc_lts::StateIndex;
use merc_utilities::MercError;
use merc_utilities::Timing;

use crate::DiscoveredSet;
use crate::LPS;
use crate::SequenceForestContext;
use crate::StateRef;
use crate::Summand;

/// Order in which discovered states are explored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[derive(Default)]
pub enum ExplorationStrategy {
    /// Depth-first exploration.
    #[default]
    Dfs,
    /// Breadth-first exploration.
    Bfs,
}

/// Explores the state space of `lps`, invoking caller-supplied closures for
/// each discovered state and transition.
///
/// # Closures
///
/// The mutable caller context `ctx` is passed into every closure invocation so
/// the closures themselves can stay non-capturing of the outer mutable state.
///
/// - `on_state(ctx, state_index, &state_info)` is called exactly once per
///   discovered state, in the order the state was popped from the working set,
///   after the implementation has been prepared for that state.
/// - `on_transition(ctx, from, &label, to)` is called for every transition
///   produced by the summands.
///
/// Returns the [`StateIndex`] assigned to the initial state of `lps`. The
/// caller is responsible for finalising any builder it owns; counting states
/// and transitions (and any progress reporting) is left to the closures.
///
/// # Errors
///
/// Returns the first error returned by `on_state` or `on_transition`,
/// aborting exploration at that point.
///
/// # Logging
///
/// At trace level every explored state vector and every enumerated transition
/// (with its summand index and target state vector) is logged, which is only
/// practical for small state spaces.
pub fn explore<P, Ctx, OnState, OnTransition>(
    lps: &P,
    strategy: ExplorationStrategy,
    timing: &Timing,
    ctx: &mut Ctx,
    mut on_state: OnState,
    mut on_transition: OnTransition,
) -> Result<StateIndex, MercError>
where
    P: LPS,
    P::Value: Debug,
    P::Label: Debug,
    OnState: FnMut(&mut Ctx, StateIndex, &P::StateInfo) -> Result<(), MercError>,
    OnTransition: FnMut(&mut Ctx, StateIndex, &P::Label, StateIndex) -> Result<(), MercError>,
{
    let discovered: DiscoveredSet<P::Value> = DiscoveredSet::new();
    let initial_state = lps.initial_state();
    let (initial_ref, _) = discovered.insert(&initial_state);
    log::trace!("explore: initial state {} = {initial_state:?}", initial_ref.index());

    let mut working: VecDeque<StateRef> = VecDeque::from([initial_ref]);
    // Reusable buffer holding the current state vector reconstructed from the
    // discovered set, avoiding an allocation per explored state.
    let mut current_state: Vec<P::Value> = Vec::new();
    // Reused interning scratch buffers, avoiding a reallocation per inserted
    // state.
    let mut forest_context = SequenceForestContext::new();
    // Per-thread enumeration context owning the backend and scratch buffers.
    // Single-threaded loop, so one context suffices.
    let mut enumerate_context = lps.create_context();

    timing.measure("explore", || -> Result<(), MercError> {
        loop {
            let current = match strategy {
                ExplorationStrategy::Dfs => working.pop_back(),
                ExplorationStrategy::Bfs => working.pop_front(),
            };
            let Some(current) = current else { break };

            // Reconstruct the state vector into the reusable buffer so
            // `discovered` can be mutated by inserts inside the summand callback
            // below.
            let found = discovered.get_into(current, &mut current_state);
            debug_assert!(found, "StateRef from working queue must be valid");
            let from = StateIndex::new(current.index());
            log::trace!("explore: state {from} = {current_state:?}");
            let summands_to_explore = lps.prepare(&mut enumerate_context, &current_state);

            let info = lps.state_info(&current_state, &enumerate_context);
            on_state(ctx, from, &info)?;

            let summands = lps.summands();
            for index in summands_to_explore {
                summands[index].enumerate(&mut enumerate_context, &current_state, |label, next_state| {
                    let (target_ref, is_new) = discovered.insert_with(next_state, &mut forest_context);
                    let to = StateIndex::new(target_ref.index());
                    log::trace!(
                        "explore: transition (summand {index}) {from} --{label:?}--> {to} = {next_state:?}{}",
                        if is_new { " (new)" } else { "" }
                    );
                    on_transition(ctx, from, label, to)?;
                    if is_new {
                        working.push_back(target_ref);
                    }
                    Ok(())
                })?;
            }
        }

        Ok(())
    })?;

    Ok(StateIndex::new(initial_ref.index()))
}

/// Explores the state space of `lps` in parallel using continuous,
/// work-stealing breadth-/depth-first search driven by `rayon`.
///
/// The interface is similar to [`explore`], but an extra closure is supplied to
/// produce a fresh per-thread accumulator for each worker.
///
/// - `make_local()` produces a fresh accumulator for each worker.
///
/// The returned `Vec<Local>` holds one accumulator per worker (one per pool
/// thread), each carrying the merged result of every state that worker
/// processed.
///
/// # State numbering
///
/// The [`StateIndex`] values passed to the closures come from the shared
/// discovered set, whose backing store hands each worker a whole block of
/// consecutive indices at once to keep index allocation contention-free. This
/// means that the returned state indices are not dense.
///
/// # Errors
///
/// Returns the first error returned by any worker's `on_state` or
/// `on_transition`, in worker-index order. Once one worker hits an error, every
/// worker stops promptly, but states and transitions already reported through
/// the callbacks before that point stand.
///
/// # Logging
///
/// At trace level every explored state vector and every enumerated transition
/// (with its summand index and target state vector) is logged, tagged with the
/// worker that processed it since workers interleave their output.
pub fn explore_parallel<P, Local, MakeLocal, OnState, OnTransition>(
    lps: &P,
    make_local: MakeLocal,
    on_state: OnState,
    on_transition: OnTransition,
) -> Result<(StateIndex, Vec<Local>), MercError>
where
    P: LPS + Sync,
    P::Value: Send + Sync + Debug,
    P::Label: Debug,
    Local: Send,
    MakeLocal: Fn() -> Local + Sync,
    OnState: Fn(&mut Local, StateIndex, &P::StateInfo) -> Result<(), MercError> + Sync,
    OnTransition: Fn(&mut Local, StateIndex, &P::Label, StateIndex) -> Result<(), MercError> + Sync,
{
    let discovered: DiscoveredSet<P::Value> = DiscoveredSet::new();
    let initial_state = lps.initial_state();
    let (initial_ref, _) = discovered.insert(&initial_state);
    log::trace!("explore: initial state {} = {initial_state:?}", initial_ref.index());

    let num_workers = rayon::current_num_threads().max(1);

    // One publication slot per worker.
    let stealers: Vec<OnceLock<Stealer<StateRef>>> = (0..num_workers).map(|_| OnceLock::new()).collect();

    // Count of states discovered but not yet fully processed; exploration is
    // complete once it reaches zero.
    let pending = AtomicUsize::new(1);
    // Set when any worker's callback returns an error so the others stop promptly.
    let aborted = AtomicBool::new(false);

    let results = rayon::broadcast(|broadcast| -> (Local, Option<MercError>) {
        let me = broadcast.index();

        // Each worker owns its deque, created here on its own thread
        // Publish its stealer so peers can balance against it.
        let local_deque: Worker<StateRef> = Worker::new_lifo();
        let _ = stealers[me].set(local_deque.stealer());

        // The first worker seeds the initial state onto its own deque; the
        // others pick it up by stealing.
        if me == 0 {
            local_deque.push(initial_ref);
        }

        // Thread-affine per-worker scratch and enumeration backend, created
        // and dropped on this same physical worker thread.
        let mut context = lps.create_context();
        let mut forest_context = SequenceForestContext::new();
        let mut state_buf: Vec<P::Value> = Vec::new();
        // Successors discovered while processing one state, counted in a
        // single atomic update and pushed onto the deque only afterwards.
        let mut successors: Vec<StateRef> = Vec::new();
        let mut local = make_local();
        let mut first_error: Option<MercError> = None;

        // Per-worker work-stealing instrumentation, emitted at debug level
        // when this worker finishes.
        let mut stats = StealMetrics::default();

        // Per-worker RNG driving randomized victim selection while stealing.
        let mut rng = SmallRng::from_rng(&mut rand::rng());

        let backoff = Backoff::new();
        loop {
            if aborted.load(Ordering::Relaxed) {
                break;
            }

            let Some(state_ref) = find_task(&local_deque, &stealers, me, &mut rng, &mut stats) else {
                // No work found this sweep. Stop once every state has been
                // processed, otherwise keep trying: a busy peer may still
                // discover successors that become stealable.
                if pending.load(Ordering::Acquire) == 0 {
                    break;
                }
                stats.idle_sweeps += 1;
                backoff.snooze();
                continue;
            };
            backoff.reset();

            let result = (|| -> Result<(), MercError> {
                let found = discovered.get_into(state_ref, &mut state_buf);
                debug_assert!(found, "StateRef from work queue must be valid");
                let from = StateIndex::new(state_ref.index());
                log::trace!("explore worker {me}: state {from} = {state_buf:?}");
                let summands_to_explore = lps.prepare(&mut context, &state_buf);

                let info = lps.state_info(&state_buf, &context);
                on_state(&mut local, from, &info)?;

                let summands = lps.summands();
                for index in summands_to_explore {
                    summands[index].enumerate(&mut context, &state_buf, |label, next_state| {
                        let (target_ref, is_new) = discovered.insert_with(next_state, &mut forest_context);
                        let to = StateIndex::new(target_ref.index());
                        log::trace!(
                            "explore worker {me}: transition (summand {index}) {from} --{label:?}--> {to} = {next_state:?}{}",
                            if is_new { " (new)" } else { "" }
                        );
                        on_transition(&mut local, from, label, to)?;
                        if is_new {
                            successors.push(target_ref);
                        }
                        Ok(())
                    })?;
                }
                Ok(())
            })();

            match result {
                Ok(()) => {
                    // Apply this state's net effect on the outstanding-work
                    // count in a single atomic update.
                    let produced = successors.len();
                    if produced == 0 {
                        pending.fetch_sub(1, Ordering::AcqRel);
                    } else {
                        pending.fetch_add(produced - 1, Ordering::AcqRel);
                        for successor in successors.drain(..) {
                            local_deque.push(successor);
                        }
                    }
                }
                Err(err) => {
                    // Abort: the pending counter is left as-is since `aborted`
                    // now drives every worker to stop, independent of it.
                    first_error = Some(err);
                    aborted.store(true, Ordering::Relaxed);
                    break;
                }
            }
        }

        stats.log(me);

        (local, first_error)
    });

    // Surface the first error in worker-index order, otherwise return one
    // accumulator per worker.
    let mut first_error: Option<MercError> = None;
    let mut locals = Vec::with_capacity(results.len());
    for (local, error) in results {
        if first_error.is_none() {
            first_error = error;
        }
        locals.push(local);
    }
    if let Some(error) = first_error {
        return Err(error);
    }

    Ok((StateIndex::new(initial_ref.index()), locals))
}

/// Per-worker work-stealing counters, collected while a worker runs and logged
/// at debug level when it finishes. Used to diagnose load balancing: a healthy
/// run keeps the stolen fraction modest and idle sweeps low.
#[derive(Default)]
struct StealMetrics {
    /// Tasks taken from the worker's own deque.
    local_pops: u64,
    /// Tasks obtained by stealing a batch from a peer.
    steals: u64,
    /// `Steal::Retry` results observed while stealing (CAS contention with peers).
    steal_retries: u64,
    /// Sweeps that found no work anywhere, each followed by a snooze.
    idle_sweeps: u64,
}

impl StealMetrics {
    /// Emits this worker's counters at debug level.
    fn log(&self, worker: usize) {
        let processed = self.local_pops + self.steals;
        let stolen_pct = if processed == 0 {
            0.0
        } else {
            100.0 * self.steals as f64 / processed as f64
        };
        log::debug!(
            "explore worker {worker}: processed {processed} states ({} local, {} stolen = {stolen_pct:.1}%), \
             {} steal retries, {} idle sweeps",
            self.local_pops,
            self.steals,
            self.steal_retries,
            self.idle_sweeps,
        );
    }
}

/// Finds the next state for a worker to process: its own deque first, then a
/// batch stolen from a peer worker. Each attempt samples a random peer index
/// (`me` is skipped). Stops at the first peer that yields a task.
///
/// On a sweep with no successful steal the function returns `None` regardless of
/// whether peers were empty or merely contended (`Steal::Retry`). The caller's
/// loop then re-checks the `pending` counter and `aborted` flag before snoozing
/// and trying again, so retries are paced by the outer backoff and a worker can
/// never spin here in isolation while the run is shutting down.
fn find_task<T>(
    local: &Worker<T>,
    stealers: &[OnceLock<Stealer<T>>],
    me: usize,
    rng: &mut SmallRng,
    stats: &mut StealMetrics,
) -> Option<T> {
    if let Some(task) = local.pop() {
        stats.local_pops += 1;
        return Some(task);
    }

    // Try up to one random peer per worker; some peers may be sampled more
    // than once and others not at all, which is fine since a sweep that finds
    // nothing is retried by the caller.
    for _ in 0..stealers.len() {
        let index = rng.random_range(0..stealers.len());
        if index == me {
            continue;
        }

        let Some(stealer) = stealers[index].get() else {
            continue;
        };

        match stealer.steal_batch_and_pop(local) {
            Steal::Success(task) => {
                stats.steals += 1;
                return Some(task);
            }
            Steal::Retry => stats.steal_retries += 1,
            Steal::Empty => {}
        }
    }

    None
}
