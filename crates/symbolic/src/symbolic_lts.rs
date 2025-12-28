use merc_aterm::ATerm;
use merc_data::DataSpecification;
use merc_ldd::Ldd;

/// Represents a symbolic LTS encoded by a disjunctive transition relation and a set of states.
pub struct SymbolicLts {
    data_specification: DataSpecification,

    states: Ldd,

    /// A singleton LDD representing the initial state.
    initial_state: Ldd,

    summand_groups: Vec<SummandGroup>,
}

impl SymbolicLts {
    /// Creates a new symbolic LTS.
    pub fn new(
        data_specification: DataSpecification,
        states: Ldd,
        initial_state: Ldd,
        summand_groups: Vec<SummandGroup>,
    ) -> Self {
        Self {
            data_specification,
            states,
            initial_state,
            summand_groups,
        }
    }

    /// Returns the data specification of the LTS.
    pub fn data_specification(&self) -> &DataSpecification {
        &self.data_specification
    }

    /// Returns the LDD representing the set of states.
    pub fn states(&self) -> &Ldd {
        &self.states
    }

    /// Returns the LDD representing the initial state.
    pub fn initial_state(&self) -> &Ldd {
        &self.initial_state
    }

    /// Returns an iterator over the summand groups.
    pub fn summand_groups(&self) -> &[SummandGroup] {
        &self.summand_groups
    }
}

/// Represents a short vector transition relation for a group of summands.
///
/// # Details
///
/// A short transition vector is part of a transition relation T -> U, where we
/// store T' -> U' with T' being the projection of T on the read parameters and
/// U' the projection of U on the write parameters, as a LDD. Formally,
///
/// (t, u) in (T -> U)  iff  (t', u') in (T' -> U') where t' and u' are the projections
///     of t and u on the read and write parameters respectively.
pub struct SummandGroup {
    read_parameters: Vec<ATerm>,
    write_parameters: Vec<ATerm>,

    /// The transition relation T' -> U' for this summand group.
    relation: Ldd,
}

impl SummandGroup {
    /// Creates a new summand group.
    pub fn new(read_parameters: Vec<ATerm>, write_parameters: Vec<ATerm>, relation: Ldd) -> Self {
        Self {
            read_parameters,
            write_parameters,
            relation,
        }
    }

    /// Returns the transition relation LDD for this summand group.
    pub fn relation(&self) -> &Ldd {
        &self.relation
    }

    /// Returns the read parameters for this summand group.
    pub fn read_parameters(&self) -> &[ATerm] {
        &self.read_parameters
    }

    /// Returns the write parameters for this summand group.
    pub fn write_parameters(&self) -> &[ATerm] {
        &self.write_parameters
    }
}
