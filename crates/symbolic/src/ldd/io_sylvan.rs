use std::collections::HashMap;
use std::fmt;
use std::io::Read;

use log::info;
use merc_utilities::MercError;
use oxidd::ManagerRef;
use oxidd::ldd::LDDFunction;
use oxidd::ldd::LDDManagerRef;
use oxidd::ldd::Value;

use crate::SymbolicLPS;
use crate::TransitionGroup;

/// Returns the (initial state, transitions) read from the file in Sylvan's format.
pub fn read_sylvan<R: Read>(manager: &LDDManagerRef, stream: &mut R) -> Result<SylvanLts, MercError> {
    info!("Reading symbolic LTS in Sylvan format...");
    let mut reader = SylvanReader::new();

    let _vector_length = read_u32(stream)?;

    let _unused = read_u32(stream)?; // This is called 'k' in Sylvan's ldd2bdd.c, but unused.
    let initial_state = reader.read_ldd(manager, stream)?;
    let num_transitions: usize = read_u32(stream)? as usize;
    let mut groups: Vec<SylvanTransitionGroup> = Vec::new();

    // Read all the transition groups.
    for _ in 0..num_transitions {
        let (read_proj, write_proj) = read_projection(stream)?;

        let (meta, empty) = manager.with_manager_shared(|m| -> Result<_, MercError> {
            let meta = LDDFunction::relation_product_meta(m, &read_proj, &write_proj)?.meta;
            Ok((meta, LDDFunction::empty_set(m)?))
        })?;
        groups.push(SylvanTransitionGroup::new(empty, meta, read_proj, write_proj));
    }

    for transition in groups.iter_mut().take(num_transitions) {
        transition.relation = reader.read_ldd(manager, stream)?;
    }

    Ok(SylvanLts::new(initial_state, groups))
}

/// Reads the read and write projections from the given stream.
pub fn read_projection<R: Read>(file: &mut R) -> Result<(Vec<Value>, Vec<Value>), MercError> {
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
    initial_state: LDDFunction,
    transition_groups: Vec<SylvanTransitionGroup>,
}

impl SylvanLts {
    /// Creates a new Sylvan LTS.
    pub fn new(initial_state: LDDFunction, transition_groups: Vec<SylvanTransitionGroup>) -> Self {
        Self {
            initial_state,
            transition_groups,
        }
    }
}

impl SymbolicLPS for SylvanLts {
    type Group = SylvanTransitionGroup;

    fn initial_state(&self) -> &LDDFunction {
        &self.initial_state
    }

    fn transition_groups(&self) -> &[Self::Group] {
        &self.transition_groups
    }

    fn transition_groups_mut(&mut self) -> &mut [Self::Group] {
        &mut self.transition_groups
    }

    fn create_context(&self) {}
}

/// A transition group read from a Sylvan file.
pub struct SylvanTransitionGroup {
    relation: LDDFunction,
    meta: LDDFunction,
    read_proj: Vec<u32>,
    write_proj: Vec<u32>,
}

impl SylvanTransitionGroup {
    /// Creates a new Sylvan transition group.
    pub fn new(relation: LDDFunction, meta: LDDFunction, read_proj: Vec<u32>, write_proj: Vec<u32>) -> Self {
        Self {
            relation,
            meta,
            read_proj,
            write_proj,
        }
    }
}

impl TransitionGroup for SylvanTransitionGroup {
    type Context = ();

    fn relation(&self) -> &LDDFunction {
        &self.relation
    }

    fn read_indices(&self) -> &[u32] {
        &self.read_proj
    }

    fn write_indices(&self) -> &[u32] {
        &self.write_proj
    }

    fn action_label_index(&self) -> Option<usize> {
        None
    }

    fn meta(&self) -> &LDDFunction {
        &self.meta
    }

    fn learn_successors(
        &mut self,
        _context: &mut (),
        _manager: &LDDManagerRef,
        _todo: &LDDFunction,
        _cached: bool,
    ) -> Result<(), MercError> {
        // All states are already explored.
        Ok(())
    }
}

impl fmt::Debug for SylvanTransitionGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SylvanTransitionGroup")
            .field("read_proj", &self.read_proj)
            .field("write_proj", &self.write_proj)
            .finish()
    }
}

/// A reader for LDDs in the Sylvan .ldd format.
pub struct SylvanReader {
    indexed_set: HashMap<u64, LDDFunction>, // Assigns LDDs to every index.
    last_index: u64,                        // The index of the last LDD read from file.
}

impl SylvanReader {
    pub fn new() -> Self {
        Self {
            indexed_set: HashMap::new(),
            last_index: 2,
        }
    }

    /// Returns an LDD read from the given stream in the Sylvan format.
    pub fn read_ldd<R: Read>(&mut self, manager: &LDDManagerRef, stream: &mut R) -> Result<LDDFunction, MercError> {
        let count = read_u64(stream)?;

        for _ in 0..count {
            // Read a single MDD node. It has the following structure: u64 | u64
            // RmRR RRRR RRRR VVVV | VVVV DcDD DDDD DDDD (little endian)
            // Every character is 4 bits, V = value, D = down, R = right, m = marked, c = copy.
            let a = read_u64(stream)?;
            let b = read_u64(stream)?;
            let right = (a & 0x0000ffffffffffff) >> 1;
            let down = b >> 17;

            let mut bytes: [u8; 4] = Default::default();
            bytes[0..2].copy_from_slice(&a.to_le_bytes()[6..8]);
            bytes[2..4].copy_from_slice(&b.to_le_bytes()[0..2]);
            let value = u32::from_le_bytes(bytes);

            let copy = right & 0x10000;
            if copy != 0 {
                return Err("Sylvan copy nodes are not supported".into());
            }

            let down = self.node_from_index(manager, down)?;
            let right = self.node_from_index(manager, right)?;

            let ldd = manager.with_manager_shared(|m| LDDFunction::make_node(m, value as Value, &down, &right))?;
            self.indexed_set.insert(self.last_index, ldd);

            self.last_index += 1;
        }

        let result = read_u64(stream)?;
        self.node_from_index(manager, result)
    }

    /// Returns the LDD belonging to the given index.
    fn node_from_index(&self, manager: &LDDManagerRef, index: u64) -> Result<LDDFunction, MercError> {
        if index == 0 {
            Ok(manager.with_manager_shared(|m| LDDFunction::empty_set(m))?)
        } else if index == 1 {
            Ok(manager.with_manager_shared(|m| LDDFunction::empty_vector(m))?)
        } else {
            Ok(self
                .indexed_set
                .get(&index)
                .ok_or_else(|| {
                    format!(
                        "LDD index {index} not found in table of {} entries",
                        self.indexed_set.len()
                    )
                })?
                .clone())
        }
    }
}

impl Default for SylvanReader {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns a single u32 read from the given stream.
pub fn read_u32<R: Read>(stream: &mut R) -> Result<u32, MercError> {
    let mut buffer: [u8; 4] = Default::default();
    stream.read_exact(&mut buffer)?;

    Ok(u32::from_le_bytes(buffer))
}

/// Returns a single u64 read from the given stream.
pub fn read_u64<R: Read>(stream: &mut R) -> Result<u64, MercError> {
    let mut buffer: [u8; 8] = Default::default();
    stream.read_exact(&mut buffer)?;

    Ok(u64::from_le_bytes(buffer))
}

#[cfg(test)]
mod test {
    use merc_utilities::Timing;

    use crate::LDD_CACHE_CAPACITY;
    use crate::LDD_NODE_CAPACITY;
    use crate::reachability;

    use super::read_sylvan;

    #[test]
    #[cfg_attr(miri, ignore)] // Miri is too slow
    fn test_load_anderson_4() {
        let ldd_manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);
        let bytes = include_bytes!("../../../../examples/ldd/anderson.4.ldd");
        let mut lts = read_sylvan(&ldd_manager, &mut &bytes[..]).expect("Loading should work correctly");
        reachability(&ldd_manager, &mut lts, &Timing::new()).expect("Reachability should work correctly");
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Miri is too slow
    #[cfg_attr(windows, ignore)] // collision.4.ldd's reachability recursion overflows Windows' smaller default stack
    #[cfg(not(debug_assertions))]
    fn test_load_collision_4() {
        let ldd_manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY * 20, LDD_CACHE_CAPACITY * 20, 1);
        let bytes = include_bytes!("../../../../examples/ldd/collision.4.ldd");
        let mut lts = read_sylvan(&ldd_manager, &mut &bytes[..]).expect("Loading should work correctly");
        reachability(&ldd_manager, &mut lts, &Timing::new()).expect("Reachability should work correctly");
    }
}
