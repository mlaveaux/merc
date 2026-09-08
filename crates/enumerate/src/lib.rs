#![doc = include_str!("../README.md")]

// `enumerator` is the one module allowed `unsafe` (for `Protected`'s
// container-registration API); every other module forbids it individually
// (see each file's own `#![forbid(unsafe_code)]`), since a crate-level
// `forbid` here would apply to `enumerator` too and cannot be downgraded by
// a child module.
mod binding;
mod enumeration_plan;
mod enumerator;
mod fresh;
mod naive_enumerator;
mod one_point;
mod ordering;
mod remaining;

pub use enumeration_plan::ConstructorPlan;
pub use enumeration_plan::EnumerationPlan;
pub use enumeration_plan::EnumerationPlanId;
pub use enumeration_plan::EnumerationPlans;
pub use enumeration_plan::NotEnumerableReason;
pub use enumeration_plan::SortEnumerability;
pub use enumerator::EnumerationLimits;
pub use enumerator::Enumerator;
pub use enumerator::Outcome;
pub use enumerator::QuantifierKind;
pub use enumerator::Solution;
pub use enumerator::WitnessOutcome;
pub use fresh::FreshVariableGenerator;
pub use naive_enumerator::NaiveEnumerator;
