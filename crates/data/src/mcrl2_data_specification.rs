use std::fmt;

use merc_aterm::ATerm;
use merc_aterm::ATermRead;
use merc_aterm::ATermStreamable;
use merc_aterm::ATermWrite;
use merc_utilities::MercError;

use crate::BasicSort;
use crate::DataEquation;
use crate::DataFunctionSymbol;
use crate::SortAlias;

/// The five sections of an mCRL2 data specification, holding the fully typed,
/// lowered terms. This is the binary serialization format also used by the
/// mCRL2 toolset.
#[derive(Default)]
pub struct Mcrl2DataSpecification {
    sorts: Vec<BasicSort>,
    aliases: Vec<SortAlias>,
    constructors: Vec<DataFunctionSymbol>,
    mappings: Vec<DataFunctionSymbol>,
    equations: Vec<DataEquation>,
}

impl Mcrl2DataSpecification {
    /// Builds a data specification from its typed sections, e.g. the output of
    /// `merc_typecheck`'s Phase-4 lowering.
    pub fn new(
        sorts: Vec<BasicSort>,
        aliases: Vec<SortAlias>,
        constructors: Vec<DataFunctionSymbol>,
        mappings: Vec<DataFunctionSymbol>,
        equations: Vec<DataEquation>,
    ) -> Self {
        Mcrl2DataSpecification {
            sorts,
            aliases,
            constructors,
            mappings,
            equations,
        }
    }

    /// Returns the user-declared sorts.
    pub fn sorts(&self) -> &[BasicSort] {
        &self.sorts
    }

    /// Returns the sort aliases.
    pub fn aliases(&self) -> &[SortAlias] {
        &self.aliases
    }

    /// Returns the constructor functions.
    pub fn constructors(&self) -> &[DataFunctionSymbol] {
        &self.constructors
    }

    /// Returns the (non-constructor) mapping functions.
    pub fn mappings(&self) -> &[DataFunctionSymbol] {
        &self.mappings
    }

    /// Returns the equations.
    pub fn equations(&self) -> &[DataEquation] {
        &self.equations
    }

    /// Returns a new specification holding `self`'s sections followed by
    /// `other`'s, section by section (sorts, then aliases, constructors,
    /// mappings, equations). Neither side is deduplicated against the
    /// other, so callers merging two specifications that may already share
    /// declarations (e.g. both derived from the same standard prelude) are
    /// responsible for avoiding that overlap themselves.
    pub fn merge(&self, other: &Mcrl2DataSpecification) -> Mcrl2DataSpecification {
        let mut sorts = self.sorts.clone();
        sorts.extend(other.sorts.iter().cloned());

        let mut aliases = self.aliases.clone();
        aliases.extend(other.aliases.iter().cloned());

        let mut constructors = self.constructors.clone();
        constructors.extend(other.constructors.iter().cloned());

        let mut mappings = self.mappings.clone();
        mappings.extend(other.mappings.iter().cloned());

        let mut equations = self.equations.clone();
        equations.extend(other.equations.iter().cloned());

        Mcrl2DataSpecification {
            sorts,
            aliases,
            constructors,
            mappings,
            equations,
        }
    }
}

impl ATermStreamable for Mcrl2DataSpecification {
    fn write<W: ATermWrite>(&self, writer: &mut W) -> Result<(), MercError> {
        writer.write_aterm_iter(self.sorts.iter().cloned().map(ATerm::from))?;
        writer.write_aterm_iter(self.aliases.iter().cloned().map(ATerm::from))?;
        writer.write_aterm_iter(self.constructors.iter().cloned().map(ATerm::from))?;
        writer.write_aterm_iter(self.mappings.iter().cloned().map(ATerm::from))?;
        writer.write_aterm_iter(self.equations.iter().cloned().map(ATerm::from))?;
        Ok(())
    }

    fn read<R: ATermRead>(reader: &mut R) -> Result<Self, MercError>
    where
        Self: Sized,
    {
        let sorts = reader
            .read_aterm_iter()?
            .map(|t| t.map(BasicSort::from))
            .collect::<Result<Vec<_>, _>>()?;
        let aliases = reader
            .read_aterm_iter()?
            .map(|t| t.map(SortAlias::from))
            .collect::<Result<Vec<_>, _>>()?;
        let constructors = reader
            .read_aterm_iter()?
            .map(|t| t.map(DataFunctionSymbol::from))
            .collect::<Result<Vec<_>, _>>()?;
        let mappings = reader
            .read_aterm_iter()?
            .map(|t| t.map(DataFunctionSymbol::from))
            .collect::<Result<Vec<_>, _>>()?;
        let equations = reader
            .read_aterm_iter()?
            .map(|t| t.map(DataEquation::from))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Mcrl2DataSpecification {
            sorts,
            aliases,
            constructors,
            mappings,
            equations,
        })
    }
}

impl fmt::Display for Mcrl2DataSpecification {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.sorts().is_empty() {
            writeln!(f, "sort")?;
            for sort in self.sorts() {
                writeln!(f, "    {sort};")?;
            }
            writeln!(f)?;
        }

        if !self.aliases().is_empty() {
            writeln!(f, "sort")?;
            for alias in self.aliases() {
                writeln!(f, "    {alias};")?;
            }
            writeln!(f)?;
        }

        if !self.constructors().is_empty() {
            writeln!(f, "cons")?;
            for cons in self.constructors() {
                writeln!(f, "    {}: {};", cons.name(), cons.sort())?;
            }
            writeln!(f)?;
        }

        if !self.mappings().is_empty() {
            writeln!(f, "map")?;
            for map in self.mappings() {
                writeln!(f, "    {}: {};", map.name(), map.sort())?;
            }

            writeln!(f)?;
        }

        if !self.equations().is_empty() {
            writeln!(f, "eqn")?;
            for eqn in self.equations() {
                let variables = eqn.variables();
                if !variables.is_empty() {
                    let vars = variables
                        .iter()
                        .map(|v| format!("{}: {}", v.name(), v.sort()))
                        .collect::<Vec<_>>()
                        .join(", ");
                    writeln!(f, "    % var {vars}")?;
                }

                writeln!(f, "    {eqn};")?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use merc_aterm::ATermStreamable;
    use merc_aterm::ATermWrite;
    use merc_aterm::BinaryATermReader;
    use merc_aterm::BinaryATermWriter;

    use super::Mcrl2DataSpecification;
    use crate::BasicSort;
    use crate::DataEquation;
    use crate::DataFunctionSymbol;
    use crate::DataVariable;
    use crate::SortAlias;
    use crate::SortExpression;

    #[test]
    fn test_mcrl2_data_specification_roundtrip() {
        let nat: SortExpression = BasicSort::new("Nat").into();
        let x = DataVariable::with_sort("x", nat.copy());

        // Populate every section, including an empty one, to exercise the length-prefixed framing.
        let spec = Mcrl2DataSpecification::new(
            vec![BasicSort::new("D")],
            Vec::new(),
            vec![
                DataFunctionSymbol::with_sort("c", nat.copy()),
                DataFunctionSymbol::with_sort("d", nat.copy()),
            ],
            vec![DataFunctionSymbol::with_sort("f", nat.copy())],
            vec![DataEquation::new(
                std::slice::from_ref(&x),
                None,
                x.clone().into(),
                x.clone().into(),
            )],
        );

        let mut stream: Vec<u8> = Vec::new();
        let mut writer = BinaryATermWriter::new(&mut stream).unwrap();
        spec.write(&mut writer).unwrap();
        ATermWrite::flush(&mut writer).unwrap();
        drop(writer); // Release the mutable borrow on the stream.

        let mut reader = BinaryATermReader::new(&stream[..]).unwrap();
        let read = Mcrl2DataSpecification::read(&mut reader).unwrap();

        assert_eq!(read.sorts, spec.sorts);
        assert_eq!(read.aliases, spec.aliases);
        assert_eq!(read.constructors, spec.constructors);
        assert_eq!(read.mappings, spec.mappings);
        assert_eq!(read.equations, spec.equations);
    }

    #[test]
    fn test_mcrl2_data_specification_alias_roundtrip() {
        let spec = Mcrl2DataSpecification::new(
            Vec::new(),
            vec![SortAlias::new(BasicSort::new("D"), BasicSort::new("Nat").into())],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        let mut stream: Vec<u8> = Vec::new();
        let mut writer = BinaryATermWriter::new(&mut stream).unwrap();
        spec.write(&mut writer).unwrap();
        ATermWrite::flush(&mut writer).unwrap();
        drop(writer);

        let mut reader = BinaryATermReader::new(&stream[..]).unwrap();
        let read = Mcrl2DataSpecification::read(&mut reader).unwrap();

        assert_eq!(read.aliases, spec.aliases);
    }
}
