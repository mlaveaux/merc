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
#[derive(Clone, Default)]
pub struct Mcrl2DataSpecification {
    sorts: Vec<BasicSort>,
    aliases: Vec<SortAlias>,
    constructors: Vec<DataFunctionSymbol>,
    mappings: Vec<DataFunctionSymbol>,
    equations: Vec<DataEquation>,
}

impl Mcrl2DataSpecification {
    /// Builds a data specification from its type checking results.
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
