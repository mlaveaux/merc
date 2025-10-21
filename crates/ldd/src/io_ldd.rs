use std::cell::RefCell;
use std::io::Read;
use std::io::Write;

use mcrl3_io::BitStreamReader;
use mcrl3_io::BitStreamWriter;
use mcrl3_utilities::IndexedSet;
use mcrl3_utilities::MCRL3Error;

use crate::Data;
use crate::Ldd;
use crate::Storage;
use crate::iterators::iter_node;

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
pub struct BinaryLddWriter<W: Write> {
    writer: BitStreamWriter<W>,
    nodes: RefCell<IndexedSet<Ldd>>,
}

impl<W: Write> BinaryLddWriter<W> {
    pub fn new(writer: W, storage: &mut Storage) -> Result<Self, MCRL3Error> {
        // Write the header of the binary LDD format.
        let mut writer = BitStreamWriter::new(writer);
        writer.write_bits(BLF_MAGIC, 16)?;
        writer.write_bits(BLF_VERSION, 16)?;

        // Add the true and false constants
        let mut nodes = IndexedSet::new();
        nodes.insert(storage.empty_set().clone());
        nodes.insert(storage.empty_vector().clone());

        Ok(Self {
            writer,
            nodes: RefCell::new(nodes),
        })
    }

    /// Writes an LDD to the stream.
    pub fn write(&mut self, ldd: &Ldd, storage: &Storage) -> Result<(), MCRL3Error> {
        for (node, Data(value, down, right)) in iter_node(storage, ldd, |node| {
            // Skip any LDD that we have already inserted in the stream
            self.nodes.borrow().contains(node)
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
                    self.ldd_index_width(),
                )?;
                self.writer.write_bits(
                    *nodes
                        .index(&right)
                        .expect("The right node must have already been written") as u64,
                    self.ldd_index_width(),
                )?;
            }

            if node == *ldd {
                // Write output LDD
                self.writer.write_bits(1, 1)?;
                self.writer.write_bits(*index as u64, self.ldd_index_width())?;
            }
        }

        Ok(())
    }

    /// Returns the number of bits required to represent an LDD index.
    fn ldd_index_width(&self) -> u8 {
        (self.nodes.borrow().len().ilog2() + 1) as u8 // Assume that size is one larger to contain the input ldd.
    }
}

pub struct BinaryLddReader<R: Read> {
    reader: BitStreamReader<R>,
    nodes: Vec<Ldd>,
}

impl<R: Read> BinaryLddReader<R> {
    /// Inserts the header into the stream and initializes the reader.
    pub fn new(reader: R) -> Result<Self, MCRL3Error> {
        let mut reader = BitStreamReader::new(reader);

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
        nodes.push(Storage::default().empty_set().clone());
        nodes.push(Storage::default().empty_vector().clone());

        Ok(Self { reader, nodes })
    }

    /// Reads an LDD from the stream.
    pub fn read(&mut self, storage: &mut Storage) -> Result<Ldd, MCRL3Error> {
        loop {
            let is_output = self.reader.read_bits(1)? == 1;

            if is_output {
                // The output is simply an index of the LDD
                let index = self.reader.read_bits(self.ldd_index_width(false))? as usize;
                return Ok(self.nodes[index].clone());
            }

            let value = self.reader.read_integer()?;
            let down_index = self.reader.read_bits(self.ldd_index_width(true))? as usize;
            let right_index = self.reader.read_bits(self.ldd_index_width(true))? as usize;
            let ldd = storage.insert(value as u32, &self.nodes[down_index], &self.nodes[right_index]);
            self.nodes.push(ldd);
        }
    }

    /// Returns the number of bits required to represent an LDD index.
    fn ldd_index_width(&self, input: bool) -> u8 {
        ((self.nodes.len() + input as usize).ilog2() + 1) as u8 // Assume that size is one larger to contain the input ldd.
    }
}

#[cfg(test)]
mod tests {
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

            let mut stream: Vec<u8> = Vec::new();

            let mut output_stream = BinaryLddWriter::new(&mut stream, &mut storage).unwrap();
            for term in &input {
                output_stream.write(term, &storage).unwrap();
            }
            drop(output_stream); // Explicitly drop to release the mutable borrow

            let mut input_stream = BinaryLddReader::new(&stream[..]).unwrap();
            for term in &input {
                debug_assert_eq!(
                    *term,
                    input_stream.read(&mut storage).unwrap(),
                    "The read LDD must match the LDD that we have written"
                );
            }
        });
    }
}
