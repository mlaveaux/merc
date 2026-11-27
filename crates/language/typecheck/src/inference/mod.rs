mod context;
#[allow(clippy::module_inception)]
mod inference;
mod resolved_sort;
mod typed_display;
mod unification;

#[cfg(feature = "internal")]
pub use context::TypeCheckContext;
#[cfg_attr(
    feature = "internal",
    expect(unused_imports, reason = "the `internal` API re-exports `TypeCheckContext` itself")
)]
pub(crate) use context::*;
#[cfg(feature = "internal")]
pub use inference::EquationTyping;
#[cfg(feature = "internal")]
pub use inference::ExprId;
#[cfg(feature = "internal")]
pub use inference::ExprTag;
pub use inference::InferenceError;
#[cfg(feature = "internal")]
pub use inference::NameTarget;
#[cfg(feature = "internal")]
pub use inference::for_each_expr_child;
pub(crate) use inference::*;
#[cfg(feature = "internal")]
pub use resolved_sort::ResolvedSort;
#[cfg(feature = "internal")]
pub use resolved_sort::ResolvedSortId;
#[cfg(feature = "internal")]
pub use resolved_sort::ResolvedSortTag;
#[cfg(feature = "internal")]
pub use resolved_sort::SortInterner;
#[cfg(feature = "internal")]
pub use resolved_sort::unreachable_not_a_value_sort;
pub(crate) use resolved_sort::*;
pub(crate) use typed_display::*;
pub(crate) use unification::*;
