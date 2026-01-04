use log::info;
use merc_io::TimeProgress;
use merc_ldd::Ldd;
use merc_ldd::Storage;
use merc_ldd::len;
use merc_ldd::minus;
use merc_ldd::relational_product;
use merc_ldd::union;
use merc_utilities::MercError;

/// A generic trait representing a symbolic LTS
pub trait SymbolicLTS {
    /// Returns the LDD representing the set of states.
    fn states(&self) -> &merc_ldd::Ldd;

    /// Returns the LDD representing the initial state.
    fn initial_state(&self) -> &merc_ldd::Ldd;

    /// Returns an iterator over the summand groups.
    fn transition_groups(&self) -> &[impl TransitionGroup];
}

pub trait TransitionGroup {
    /// Returns the transition relation T' -> U' for this summand group.
    fn relation(&self) -> &Ldd;

    /// Returns the meta information for this summand group.
    fn meta(&self) -> &Ldd;
}

/// Performs reachability analysis using the given initial state and transitions read from a Sylvan file.
pub fn reachability(storage: &mut Storage, lts: &impl SymbolicLTS) -> Result<usize, MercError> {
    let mut todo = lts.initial_state().clone();
    let mut states = lts.initial_state().clone(); // The state space.
    let mut iteration = 0;

    let progress = TimeProgress::new(
        |iteration: usize| {
            info!("Iteration {}", iteration);
        },
        1,
    );

    while todo != *storage.empty_set() {
        let mut todo1 = storage.empty_set().clone();
        for transition in lts.transition_groups() {
            let result = relational_product(storage, &todo, transition.relation(), transition.meta());
            todo1 = union(storage, &todo1, &result);
        }

        todo = minus(storage, &todo1, &states);
        states = union(storage, &states, &todo);
        progress.print(iteration);
        iteration += 1;
    }

    Ok(len(storage, &states))
}
