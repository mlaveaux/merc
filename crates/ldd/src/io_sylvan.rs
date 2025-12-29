use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::Read;

use crate::Ldd;
use crate::Storage;
use crate::Value;
use crate::compute_meta;

pub struct Transition {
    pub relation: Ldd,
    pub meta: Ldd,
}

/// Returns the (initial state, transitions) read from the file in Sylvan's format.
pub fn load_model(storage: &mut Storage, filename: &str) -> Result<(Ldd, Vec<Transition>), Box<dyn Error>> {
    let mut file = File::open(filename)?;
    let mut reader = SylvanReader::new();

    let _vector_length = read_u32(&mut file)?;
    //println!("Length of vector {}", vector_length);

    let _unused = read_u32(&mut file)?; // This is called 'k' in Sylvan's ldd2bdd.c, but unused.
    let initial_state = reader.read_ldd(storage, &mut file)?;

    let num_transitions: usize = read_u32(&mut file)? as usize;
    let mut transitions: Vec<Transition> = Vec::new();

    // Read all the transition groups.
    for _ in 0..num_transitions {
        let (read_proj, write_proj) = read_projection(&mut file)?;
        transitions.push(Transition {
            relation: storage.empty_set().clone(),
            meta: compute_meta(storage, &read_proj, &write_proj),
        });
    }

    for transition in transitions.iter_mut().take(num_transitions) {
        transition.relation = reader.read_ldd(storage, &mut file)?;
    }

    // Ignore the rest for now.
    Ok((initial_state, transitions))
}

struct SylvanReader {
    indexed_set: HashMap<u64, Ldd>, // Assigns LDDs to every index.
    last_index: u64,                // The index of the last LDD read from file.
}

impl SylvanReader {
    fn new() -> Self {
        Self {
            indexed_set: HashMap::new(),
            last_index: 2,
        }
    }

    /// Returns an LDD read from the given file in the Sylvan format.
    fn read_ldd(&mut self, storage: &mut Storage, file: &mut File) -> Result<Ldd, Box<dyn Error>> {
        let count = read_u64(file)?;
        //println!("node count = {}", count);

        for _ in 0..count {
            // Read a single MDD node. It has the following structure: u64 | u64
            // RmRR RRRR RRRR VVVV | VVVV DcDD DDDD DDDD (little endian)
            // Every character is 4 bits, V = value, D = down, R = right, m = marked, c = copy.
            let a = read_u64(file)?;
            let b = read_u64(file)?;
            //println!("{:064b} | {:064b}", a, b);

            let right = (a & 0x0000ffffffffffff) >> 1;
            let down = b >> 17;

            let mut bytes: [u8; 4] = Default::default();
            bytes[0..2].copy_from_slice(&a.to_le_bytes()[6..8]);
            bytes[2..4].copy_from_slice(&b.to_le_bytes()[0..2]);
            let value = u32::from_le_bytes(bytes);

            let copy = right & 0x10000;
            if copy != 0 {
                panic!("We do not yet deal with copy nodes.");
            }

            let down = self.node_from_index(storage, down);
            let right = self.node_from_index(storage, right);

            let ldd = storage.insert(value as Value, &down, &right);
            self.indexed_set.insert(self.last_index, ldd);

            self.last_index += 1;
        }

        let result = read_u64(file)?;
        Ok(self.node_from_index(storage, result))
    }

    /// Returns the LDD belonging to the given index.
    fn node_from_index(&self, storage: &mut Storage, index: u64) -> Ldd {
        if index == 0 {
            storage.empty_set().clone()
        } else if index == 1 {
            storage.empty_vector().clone()
        } else {
            self.indexed_set.get(&index).unwrap().clone()
        }
    }
}

/// Returns a single u32 read from the file.
fn read_u32(file: &mut File) -> Result<u32, Box<dyn Error>> {
    let mut buffer: [u8; 4] = Default::default();
    file.read_exact(&mut buffer)?;

    Ok(u32::from_le_bytes(buffer))
}

/// Returns a single u64 read from the file.
fn read_u64(file: &mut File) -> Result<u64, Box<dyn Error>> {
    let mut buffer: [u8; 8] = Default::default();
    file.read_exact(&mut buffer)?;

    Ok(u64::from_le_bytes(buffer))
}

/// Reads the read and write projections from the file.
fn read_projection(file: &mut File) -> Result<(Vec<Value>, Vec<Value>), Box<dyn Error>> {
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_load_anderson_4() {
        let mut storage = Storage::new();
        let (_, _) = load_model(&mut storage, "../../examples/ldd/anderson.4.ldd").expect("Loading should work correctly");
    }

    #[test]
    fn test_load_collision_4() {
        let mut storage = Storage::new();
        let (_, _) = load_model(&mut storage, "../../examples/ldd/collision.4.ldd").expect("Loading should work correctly");
    }
}
