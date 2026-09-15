#![doc = include_str!("../README.md")]
#![debugger_visualizer(gdb_script_file = "gdb_pretty_printers.py")]

mod aterm;
mod aterm_binary_stream;
mod aterm_builder;
mod aterm_int;
mod aterm_list;
mod aterm_string;
mod markable;
mod parse_term;
mod protected;
mod random_term;
mod symbol;
mod transmutable;

pub mod storage;

pub(crate) use aterm_int::*;
pub(crate) use aterm_list::*;
pub(crate) use parse_term::*;
#[cfg(test)]
pub(crate) use random_term::*;

// Public API re-exports.
pub use aterm::ATerm;
pub use aterm::ATermArgs;
pub use aterm::ATermIndex;
pub use aterm::ATermRef;
pub use aterm::ATermSend;
pub use aterm::Return;
pub use aterm::Term;
pub use aterm::TermIterator;
pub use aterm_binary_stream::ATermRead;
pub use aterm_binary_stream::ATermStreamable;
pub use aterm_binary_stream::ATermWrite;
pub use aterm_binary_stream::BinaryATermReader;
pub use aterm_binary_stream::BinaryATermWriter;
pub use aterm_builder::ArgStack;
pub use aterm_builder::TermBuilder;
pub use aterm_builder::Yield;
pub use aterm_builder::apply;
pub use aterm_int::ATermInt;
pub use aterm_int::ATermIntRef;
pub use aterm_int::is_int_term;
pub use aterm_list::ATermList;
pub use aterm_list::ATermListIter;
pub use aterm_list::is_list_term;
pub use aterm_string::ATermString;
pub use aterm_string::ATermStringRef;
pub use markable::Markable;
pub use protected::Protected;
pub use protected::ProtectedReadGuard;
pub use protected::ProtectedSend;
pub use protected::ProtectedWriteGuard;
pub use symbol::Symb;
pub use symbol::Symbol;
pub use symbol::SymbolIndex;
pub use symbol::SymbolRef;
pub use transmutable::Transmutable;
