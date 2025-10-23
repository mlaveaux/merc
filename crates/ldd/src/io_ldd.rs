use std::cell::RefCell;

use mcrl3_aterm::ATerm;
use mcrl3_aterm::ATermRead;
use mcrl3_io::BitStreamRead;
use mcrl3_io::BitStreamWrite;
use mcrl3_utilities::IndexedSet;
use mcrl3_utilities::MCRL3Error;

use crate::Data;
use crate::Ldd;
use crate::Storage;
use crate::iterators::iter_nodes;

///  The magic value for a binary LDD format stream.
const BLF_MAGIC: u64 = 0x8baf;
const BLF_VERSION: u64 = 0x8306;

/// \brief Writes ldds in a streamable binary format to an output stream.
/// \details The streamable ldd format:
///
/// Every LDD is traversed in order and assigned a unique number.
/// Whenever traversal encounters an LDD of which all children have
/// been visited it is written to the stream as 0:[value, down_index,
/// right_index]. An output LDD (as returned by
/// binary_ldd_istream::get()) is written as 1:index.
pub struct BinaryLddWriter<W: BitStreamWrite> {
    writer: W,
    nodes: RefCell<IndexedSet<Ldd>>,
}

impl<W: BitStreamWrite> BinaryLddWriter<W> {
    pub fn new(mut writer: W, storage: &mut Storage) -> Result<Self, MCRL3Error> {
        // Write the header of the binary LDD format.
        writer.write_bits(BLF_MAGIC, 16)?;
        writer.write_bits(BLF_VERSION, 16)?;

        // Add the true and false constants
        let mut nodes = IndexedSet::new();
        nodes.insert(storage.empty_vector().clone());
        nodes.insert(storage.empty_set().clone());

        Ok(Self {
            writer,
            nodes: RefCell::new(nodes),
        })
    }

    /// Writes an LDD to the stream.
    pub fn write_ldd(&mut self, ldd: &Ldd, storage: &Storage) -> Result<(), MCRL3Error> {
        for (node, Data(value, down, right)) in iter_nodes(storage, ldd, |node| {
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

        Ok(())
    }

    /// Returns the number of bits required to represent an LDD index.
    fn ldd_index_width(nodes: &IndexedSet<Ldd>) -> u8 {
        (nodes.len().ilog2() + 1) as u8 // Assume that size is one larger to contain the input ldd.
    }
}

pub struct BinaryLddReader<R: BitStreamRead> {
    reader: R,
    nodes: Vec<Ldd>,
}

impl<R: BitStreamRead> BinaryLddReader<R> {
    /// Inserts the header into the stream and initializes the reader.
    pub fn new(mut reader: R) -> Result<Self, MCRL3Error> {
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
        let mut nodes = Vec::new();
        nodes.push(Storage::default().empty_vector().clone());
        nodes.push(Storage::default().empty_set().clone());

        Ok(Self { reader, nodes })
    }

    /// Reads an LDD from the stream.
    pub fn read_ldd(&mut self, storage: &mut Storage) -> Result<Ldd, MCRL3Error> {
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
            let ldd = storage.insert(
                value as u32,
                self.nodes.get(down_index).ok_or(format!(
                    "Read invalid down ldd index {down_index}, length {}",
                    self.nodes.len()
                ))?,
                self.nodes.get(right_index).ok_or(format!(
                    "Read invalid right lddindex {right_index}, length {}",
                    self.nodes.len()
                ))?,
            );
            self.nodes.push(ldd);
        }
    }

    /// Returns the number of bits required to represent an LDD index.
    fn ldd_index_width(&self, input: bool) -> u8 {
        ((self.nodes.len() + input as usize).ilog2() + 1) as u8 // Assume that size is one larger to contain the input ldd.
    }
}

impl<R: BitStreamRead + ATermRead> ATermRead for BinaryLddReader<R> {
    delegate::delegate! {
        to self.reader {
            fn read_aterm(&mut self) -> Result<Option<ATerm>, MCRL3Error>;
            fn read_aterm_iter(&mut self) -> Result<impl ExactSizeIterator<Item = Result<ATerm, MCRL3Error>>, MCRL3Error>;
        }
    }
}

#[cfg(test)]
mod tests {
    use mcrl3_io::BitStreamReader;
    use mcrl3_io::BitStreamWriter;
    use mcrl3_utilities::random_test;

    use crate::test_utility::from_iter;
    use crate::test_utility::random_vector_set;

    use super::*;

    #[test]
    fn test_binary_ldd_stream() {
        random_test(1, |rng| {
            let mut storage = Storage::new();

            let input: Vec<_> = (0..20)
                .map(|_| {
                    let input = random_vector_set(rng, 32, 10, 10);
                    from_iter(&mut storage, input.iter())
                })
                .collect();

            let mut vector: Vec<u8> = Vec::new();
            let stream = BitStreamWriter::new(&mut vector);

            let mut output_stream = BinaryLddWriter::new(stream, &mut storage).unwrap();
            for term in &input {
                output_stream.write_ldd(term, &storage).unwrap();
            }
            drop(output_stream); // Explicitly drop to release the mutable borrow

            let mut input_stream = BinaryLddReader::new(BitStreamReader::new(&vector[..])).unwrap();
            for term in &input {
                debug_assert_eq!(
                    *term,
                    input_stream.read_ldd(&mut storage).unwrap(),
                    "The read LDD must match the LDD that we have written"
                );
            }
        });
    }
}
