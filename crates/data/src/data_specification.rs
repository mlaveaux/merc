use mcrl3_aterm::ATerm;
use mcrl3_aterm::ATermRead;
use mcrl3_aterm::ATermStreamable;
use mcrl3_aterm::ATermWrite;
use mcrl3_utilities::MCRL3Error;

/// TODO: Not yet useful, but can be used to read the data specification from a binary stream.
pub struct DataSpecification {}

impl ATermStreamable for DataSpecification {
    fn write<W: ATermWrite>(&self, _writer: &mut W) -> Result<(), MCRL3Error> {
        unimplemented!()
    }

    fn read<R: ATermRead>(reader: &mut R) -> Result<Self, MCRL3Error>
    where
        Self: Sized,
    {
        let _sorts: Result<Vec<ATerm>, MCRL3Error> = reader.read_aterm_iter()?.collect();
        let _aliases: Result<Vec<ATerm>, MCRL3Error> = reader.read_aterm_iter()?.collect();
        let _constructors: Result<Vec<ATerm>, MCRL3Error> = reader.read_aterm_iter()?.collect();
        let _user_defined_mappings: Result<Vec<ATerm>, MCRL3Error> = reader.read_aterm_iter()?.collect();
        let _user_defined_equations: Result<Vec<ATerm>, MCRL3Error> = reader.read_aterm_iter()?.collect();

        // Ignore results for now.
        Ok(DataSpecification {})
    }
}
