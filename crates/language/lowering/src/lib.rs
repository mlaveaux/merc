#![doc = include_str!("../README.md")]

mod lowering;
mod system_symbols;

pub use lowering::lower_data_specification;
pub use lowering::lower_expression;
