use std::fmt;

/// Variant of variability solver to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum Solver {
    /// Zielonka's recursive algorithm.
    Zielonka,
    /// Priority promotion algorithm.
    PriorityPromotion,
    /// The hybrid strategy improvement algorithm H-B on the Vöge-Jurdziński valuation.
    HybridSi,
    /// Switch-all strategy improvement of Vöge and Jurdziński.
    VjSi,
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
            Self::HybridSi => write!(f, "hybrid-si"),
            Self::VjSi => write!(f, "vj-si"),
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
