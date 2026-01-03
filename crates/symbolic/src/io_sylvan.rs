use std::io::Read;

use merc_ldd::Ldd;
use merc_ldd::Storage;
use merc_ldd::SylvanReader;
use merc_ldd::Value;
use merc_ldd::compute_meta;
use merc_ldd::read_u32;
use merc_utilities::MercError;

use crate::SymbolicLTS;
use crate::TransitionGroup;

/// Returns the (initial state, transitions) read from the file in Sylvan's format.
pub fn read_sylvan(storage: &mut Storage, stream: &mut impl Read) -> Result<SylvanLts, MercError> {
    let mut reader = SylvanReader::new();

    let _vector_length = read_u32(stream)?;
    //println!("Length of vector {}", vector_length);

    let _unused = read_u32(stream)?; // This is called 'k' in Sylvan's ldd2bdd.c, but unused.
    let initial_state = reader.read_ldd(storage, stream)?;
    let num_transitions: usize = read_u32(stream)? as usize;
    let mut groups: Vec<SylvanTransitionGroup> = Vec::new();

    // Read all the transition groups.
    for _ in 0..num_transitions {
        let (read_proj, write_proj) = read_projection(stream)?;
        groups.push(SylvanTransitionGroup::new(
            storage.empty_set().clone(),
            compute_meta(storage, &read_proj, &write_proj),
        ));
    }

    for transition in groups.iter_mut().take(num_transitions) {
        transition.relation = reader.read_ldd(storage, stream)?;
    }

    Ok(SylvanLts::new(storage.empty_set().clone(), initial_state, groups))
}

/// Reads the read and write projections from the given stream.
pub fn read_projection(file: &mut impl Read) -> Result<(Vec<Value>, Vec<Value>), MercError> {
    let num_read = read_u32(file)?;
    let num_write = read_u32(file)?;

    // Read num_read integers for the read parameters.
    let mut read_proj: Vec<Value> = Vec::new();
    for _ in 0..num_read {
        let value = read_u32(file)?;
        read_proj.push(value as Value);
    }

    // Read num_write integers for the write parameters.
    let mut write_proj: Vec<Value> = Vec::new();
    for _ in 0..num_write {
        let value = read_u32(file)?;
        write_proj.push(value as Value);
    }

    Ok((read_proj, write_proj))
}

/// A symbolic labelled transition system read from a Sylvan file.
pub struct SylvanLts {
    initial_state: merc_ldd::Ldd,

    transition_groups: Vec<SylvanTransitionGroup>, // (relation, meta)

    empty_set: Ldd,
}

impl SylvanLts {
    /// Creates a new Sylvan LTS.
    pub fn new(empty_set: Ldd, initial_state: Ldd, transition_groups: Vec<SylvanTransitionGroup>) -> Self {
        Self {
            initial_state,
            transition_groups,
            empty_set,
        }
    }
}

impl SymbolicLTS for SylvanLts {
    fn states(&self) -> &Ldd {
        &self.empty_set
    }

    fn initial_state(&self) -> &Ldd {
        &self.initial_state
    }

    fn transition_groups(&self) -> &[impl TransitionGroup] {
        &self.transition_groups
    }
}

/// A transition group read from a Sylvan file.
pub struct SylvanTransitionGroup {
    relation: Ldd,
    meta: Ldd,
}

impl SylvanTransitionGroup {
    /// Creates a new Sylvan transition group.
    pub fn new(relation: Ldd, meta: Ldd) -> Self {
        Self { relation, meta }
    }
}

impl TransitionGroup for SylvanTransitionGroup {
    fn relation(&self) -> &Ldd {
        &self.relation
    }

    fn meta(&self) -> &Ldd {
        &self.meta
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_load_anderson_4() {
        let mut storage = Storage::new();
        let bytes = include_bytes!("../../../examples/ldd/anderson.4.ldd");
        let lts = read_sylvan(&mut storage, &mut &bytes[..]).expect("Loading should work correctly");
    }

    #[test]
    fn test_load_collision_4() {
        let mut storage = Storage::new();
        let bytes = include_bytes!("../../../examples/ldd/collision.4.ldd");
        let lts = read_sylvan(&mut storage, &mut &bytes[..]).expect("Loading should work correctly");
    }
}
