use std::fmt;

/// Variant of variability solver to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum Solver {
    /// Zielonka's recursive algorithm.
    Zielonka,
    /// Priority promotion algorithm.
    PriorityPromotion,
    /// Experimental two-sided progress-measure lifting over instance-derived trees.
    TwoSidedLifting,
}

/// Variant of the parity game algorithm to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum VpgSolver {
    /// Product-based solver.
    Product,
    /// Standard family-based Zielonka algorithm.
    Family,
    /// Left-optimised family-based Zielonka variant.
    FamilyOptimisedLeft,
}

impl fmt::Display for Solver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Zielonka => write!(f, "zielonka"),
            Self::PriorityPromotion => write!(f, "priority-promotion"),
            Self::TwoSidedLifting => write!(f, "two-sided-lifting"),
        }
    }
}

impl fmt::Display for VpgSolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Product => write!(f, "product"),
            Self::Family => write!(f, "family"),
            Self::FamilyOptimisedLeft => write!(f, "family-optimised-left"),
        }
    }
}
