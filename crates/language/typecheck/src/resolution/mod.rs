mod alias;
mod anonymous_struct;
mod name_resolution;
mod non_empty;
mod normalize;
mod variable_resolution;

pub(crate) use alias::*;
#[cfg_attr(
    not(feature = "internal"),
    expect(unused_imports, reason = "only the `internal` API uses it")
)]
pub(crate) use anonymous_struct::*;
pub(crate) use name_resolution::*;
pub(crate) use non_empty::*;
pub(crate) use normalize::*;
pub(crate) use variable_resolution::*;

#[cfg(feature = "internal")]
pub use variable_resolution::NameStack;
#[cfg(feature = "internal")]
pub use variable_resolution::Scope;
#[cfg(feature = "internal")]
pub use variable_resolution::resolve_in_data_expr;
#[cfg(feature = "internal")]
pub use variable_resolution::resolve_in_prop_var_inst;
