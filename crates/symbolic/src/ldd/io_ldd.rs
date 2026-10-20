use std::cell::RefCell;

use oxidd::ManagerRef;
use oxidd::ldd::LDDFunction;
use oxidd::ldd::LDDManagerRef;

use merc_aterm::ATerm;
use merc_aterm::ATermRead;
use merc_aterm::ATermWrite;
use merc_collections::IndexedSet;
use merc_io::BitStreamRead;
use merc_io::BitStreamWrite;
use merc_number::bits_for_value;
use merc_utilities::MercError;

use crate::Data;
use crate::iter_nodes;

///  The magic value for a binary LDD format stream.
const BLF_MAGIC: u64 = 0x8baf;
const BLF_VERSION: u64 = 0x8306;

/// Writes LDDs in a streamable binary format to an output stream.
///
/// Every LDD is traversed in order and assigned a unique number. Whenever traversal
/// encounters an LDD for which all children have been visited it is written to the stream
/// as `0:[value, down_index, right_index]`. Every output LDD is written as `1:index`, and
/// will be returned by [BinaryLddReader::read_ldd].
pub struct BinaryLddWriter<W: BitStreamWrite> {
    writer: W,
    nodes: RefCell<IndexedSet<LDDFunction>>,
}

impl<W: BitStreamWrite> BinaryLddWriter<W> {
    pub fn new(manager: &LDDManagerRef, mut writer: W) -> Result<Self, MercError> {
        // Write the header of the binary LDD format.
        writer.write_bits(BLF_MAGIC, 16)?;
        writer.write_bits(BLF_VERSION, 16)?;

        // Add the true and false constants
        let mut nodes = IndexedSet::new();
        manager.with_manager_shared(|m| -> Result<(), MercError> {
            nodes.insert(LDDFunction::empty_set(m)?);
            nodes.insert(LDDFunction::empty_vector(m)?);
            Ok(())
        })?;

        Ok(Self {
            writer,
            nodes: RefCell::new(nodes),
        })
    }

    /// Writes an LDD to the stream.
    pub fn write_ldd(&mut self, ldd: &LDDFunction) -> Result<(), MercError> {
        for (node, Data(value, down, right)) in iter_nodes(ldd, |node| {
            // Skip any LDD that we have already inserted in the stream
            !self.nodes.borrow().contains(node)
        }) {
            let mut nodes = self.nodes.borrow_mut();
            let (index, inserted) = nodes.insert(node.clone());
            if inserted {
                // New LDD that must be written to stream
                self.writer.write_bits(0, 1)?;
                self.writer.write_integer(value as u64)?;
                self.writer.write_bits(
                    *nodes
                        .index(&down)
                        .expect("The down node must have already been written") as u64,
                    Self::ldd_index_width(&nodes),
                )?;
                self.writer.write_bits(
                    *nodes
                        .index(&right)
                        .expect("The right node must have already been written") as u64,
                    Self::ldd_index_width(&nodes),
                )?;
            }

            if node == *ldd {
                // Write output LDD
                self.writer.write_bits(1, 1)?;
                self.writer.write_bits(*index as u64, Self::ldd_index_width(&nodes))?;
            }
        }

        // iter_nodes yields nothing for terminals; emit the output marker directly.
        if ldd.node().is_none() {
            let nodes = self.nodes.borrow();
            let index = nodes.index(ldd).expect("terminals are pre-inserted in new()");
            self.writer.write_bits(1, 1)?;
            self.writer.write_bits(*index as u64, Self::ldd_index_width(&nodes))?;
        }

        Ok(())
    }

    /// Returns the number of bits required to represent an LDD index.
    /// This matches the reader: when writing a node definition we've already
    /// inserted the current node, so we use the current number of nodes.
    fn ldd_index_width(nodes: &IndexedSet<LDDFunction>) -> u8 {
        bits_for_value(nodes.len())
    }
}

impl<W: BitStreamWrite + ATermWrite> ATermWrite for BinaryLddWriter<W> {
    delegate::delegate! {
        to self.writer {
            fn write_aterm(&mut self, term: &ATerm) -> Result<(), MercError>;
            fn write_aterm_int(&mut self, value: usize) -> Result<(), MercError>;
            fn write_aterm_iter<I>(&mut self, iter: I) -> Result<(), MercError>
            where
                I: ExactSizeIterator<Item = ATerm>;
        }
    }

    fn flush(&mut self) -> Result<(), MercError> {
        ATermWrite::flush(&mut self.writer)
    }
}

impl<W: BitStreamWrite> BitStreamWrite for BinaryLddWriter<W> {
    delegate::delegate! {
        to self.writer {
            fn write_bits(&mut self, value: u64, number_of_bits: u8) -> Result<(), MercError>;
            fn write_string(&mut self, s: &str) -> Result<(), MercError>;
            fn write_integer(&mut self, value: u64) -> Result<(), MercError>;
            fn flush(&mut self) -> Result<(), MercError>;
        }
    }
}

pub struct BinaryLddReader<R: BitStreamRead> {
    reader: R,
    nodes: Vec<LDDFunction>,
}

impl<R: BitStreamRead> BinaryLddReader<R> {
    /// Inserts the header into the stream and initializes the reader.
    pub fn new(storage: &LDDManagerRef, mut reader: R) -> Result<Self, MercError> {
        // Read and verify the header of the binary LDD format.
        let magic = reader.read_bits(16)?;
        if magic != BLF_MAGIC {
            return Err("Invalid magic number in binary LDD stream".into());
        }

        let version = reader.read_bits(16)?;
        if version != BLF_VERSION {
            return Err(format!("The BLF version ({version}) of the input file is incompatible with the version ({BLF_VERSION}) of this tool. The input file must be regenerated.").into());
        }

        // Add the true and false constants
        let nodes = storage.with_manager_shared(|m| -> Result<_, MercError> {
            Ok(vec![LDDFunction::empty_set(m)?, LDDFunction::empty_vector(m)?])
        })?;
        Ok(Self { reader, nodes })
    }

    /// Reads an LDD from the stream.
    pub fn read_ldd(&mut self, storage: &LDDManagerRef) -> Result<LDDFunction, MercError> {
        loop {
            let is_output = self.reader.read_bits(1)? == 1;

            if is_output {
                // The output is simply an index of the LDD
                let index = self.reader.read_bits(self.ldd_index_width(false))? as usize;
                return Ok(self
                    .nodes
                    .get(index)
                    .ok_or(format!("Read invalid ldd index {index}, length {}", self.nodes.len()))?
                    .clone());
            }

            let value = self.reader.read_integer()?;
            let down_index = self.reader.read_bits(self.ldd_index_width(true))? as usize;
            let right_index = self.reader.read_bits(self.ldd_index_width(true))? as usize;
            let down = self
                .nodes
                .get(down_index)
                .ok_or(format!(
                    "Read invalid down ldd index {down_index}, length {}",
                    self.nodes.len()
                ))?
                .clone();
            let right = self
                .nodes
                .get(right_index)
                .ok_or(format!(
                    "Read invalid right ldd index {right_index}, length {}",
                    self.nodes.len()
                ))?
                .clone();
            let ldd = storage.with_manager_shared(|m| LDDFunction::make_node(m, value as u32, &down, &right))?;
            self.nodes.push(ldd);
        }
    }

    /// Returns the number of bits required to represent an LDD index.
    fn ldd_index_width(&self, input: bool) -> u8 {
        bits_for_value(self.nodes.len() + input as usize) // Assume that size is one larger to contain the input ldd.
    }
}

impl<R: BitStreamRead + ATermRead> ATermRead for BinaryLddReader<R> {
    delegate::delegate! {
        to self.reader {
            fn read_aterm(&mut self) -> Result<Option<ATerm>, MercError>;
            fn read_aterm_int(&mut self) -> Result<usize, MercError>;
            fn read_aterm_iter(&mut self) -> Result<impl ExactSizeIterator<Item = Result<ATerm, MercError>>, MercError>;
        }
    }
}

impl<R: BitStreamRead> BitStreamRead for BinaryLddReader<R> {
    delegate::delegate! {
        to self.reader {
            fn read_bits(&mut self, num_bits: u8) -> Result<u64, MercError>;
            fn read_integer(&mut self) -> Result<u64, MercError>;
            fn read_string(&mut self) -> Result<String, MercError>;
        }
    }
}

#[cfg(test)]
mod tests {
    use merc_io::BitStreamReader;
    use merc_io::BitStreamWriter;
    use merc_utilities::random_test;

    use crate::LDD_CACHE_CAPACITY;
    use crate::LDD_NODE_CAPACITY;
    use crate::from_iter;
    use crate::random_vector_set;

    use super::*;

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_binary_ldd_stream_terminals() {
        let manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);

        let terminals = manager.with_manager_shared(|m| {
            [
                LDDFunction::empty_set(m).unwrap(),
                LDDFunction::empty_vector(m).unwrap(),
            ]
        });
        for term in terminals {
            let mut vector: Vec<u8> = Vec::new();
            let stream = BitStreamWriter::new(&mut vector);
            let mut output_stream = BinaryLddWriter::new(&manager, stream).unwrap();
            output_stream.write_ldd(&term).unwrap();
            drop(output_stream);

            let mut input_stream = BinaryLddReader::new(&manager, BitStreamReader::new(&vector[..])).unwrap();
            assert!(
                term == input_stream.read_ldd(&manager).unwrap(),
                "terminal round-trip failed"
            );
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn test_binary_ldd_stream() {
        random_test(100, |rng| {
            let manager = oxidd::ldd::new_manager(LDD_NODE_CAPACITY, LDD_CACHE_CAPACITY, 1);

            let input: Vec<_> = (0..20)
                .map(|_| {
                    let input = random_vector_set(rng, 32, 10, 10);
                    from_iter(&manager, input.iter())
                })
                .collect();

            let mut vector: Vec<u8> = Vec::new();
            let stream = BitStreamWriter::new(&mut vector);

            let mut output_stream = BinaryLddWriter::new(&manager, stream).unwrap();
            for term in &input {
                output_stream.write_ldd(term).unwrap();
            }
            drop(output_stream); // Explicitly drop to release the mutable borrow

            let mut input_stream = BinaryLddReader::new(&manager, BitStreamReader::new(&vector[..])).unwrap();
            for term in &input {
                assert!(
                    *term == input_stream.read_ldd(&manager).unwrap(),
                    "The read LDD must match the LDD that we have written"
                );
            }
        });
    }
}
