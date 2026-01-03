use std::collections::HashMap;
use std::io::Read;

use merc_utilities::MercError;

use crate::Ldd;
use crate::Storage;
use crate::Value;

/// A reader for LDDs in the Sylvan .ldd format.
pub struct SylvanReader {
    indexed_set: HashMap<u64, Ldd>, // Assigns LDDs to every index.
    last_index: u64,                // The index of the last LDD read from file.
}

impl SylvanReader {
    pub fn new() -> Self {
        Self {
            indexed_set: HashMap::new(),
            last_index: 2,
        }
    }

    /// Returns an LDD read from the given stream in the Sylvan format.
    pub fn read_ldd(&mut self, storage: &mut Storage, stream: &mut impl Read) -> Result<Ldd, MercError> {
        let count = read_u64(stream)?;
        //println!("node count = {}", count);

        for _ in 0..count {
            // Read a single MDD node. It has the following structure: u64 | u64
            // RmRR RRRR RRRR VVVV | VVVV DcDD DDDD DDDD (little endian)
            // Every character is 4 bits, V = value, D = down, R = right, m = marked, c = copy.
            let a = read_u64(stream)?;
            let b = read_u64(stream)?;
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

        let result = read_u64(stream)?;
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

impl Default for SylvanReader {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns a single u32 read from the given stream.
pub fn read_u32(stream: &mut impl Read) -> Result<u32, MercError> {
    let mut buffer: [u8; 4] = Default::default();
    stream.read_exact(&mut buffer)?;

    Ok(u32::from_le_bytes(buffer))
}

/// Returns a single u64 read from the given stream.
pub fn read_u64(stream: &mut impl Read) -> Result<u64, MercError> {
    let mut buffer: [u8; 8] = Default::default();
    stream.read_exact(&mut buffer)?;

    Ok(u64::from_le_bytes(buffer))
}
