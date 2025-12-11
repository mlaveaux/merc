use std::collections::HashSet;
use std::fmt;
use std::io;
use std::io::Write;

use itertools::Itertools;

use crate::Data;
use crate::Ldd;
use crate::Storage;
use crate::iterators::*;

/// Print the vector set represented by the LDD.
pub fn fmt_node<'a>(storage: &'a Storage, ldd: &Ldd) -> LddDisplay<'a> {
    LddDisplay {
        storage,
        ldd: ldd.clone(),
    }
}

/// Prints the given LDD is the 'dot' format, which can be converted into an image using 'GraphViz'.
pub fn print_dot(storage: &Storage, f: &mut impl Write, ldd: &Ldd) -> io::Result<()> {
    write!(
        f,
        r#"
digraph "DD" {{
graph [dpi = 300];
center = true;
edge [dir = forward];

"#
    )?;

    // Every node must be printed once, so keep track of already printed ones.
    #[allow(clippy::mutable_key_type)]
    let mut marked: HashSet<Ldd> = HashSet::new();

    // We don't show these nodes in the output since every right most node is 'false' and every bottom node is 'true'.
    // or in our terms empty_set and empty_vector. However, if the LDD itself is 'false' or 'true' we just show the single
    // node for clarity.
    if ldd == storage.empty_set() {
        writeln!(f, "0 [shape=record, label=\"False\"];")?;
    } else if ldd == storage.empty_vector() {
        writeln!(f, "1 [shape=record, label=\"True\"];")?;
    } else {
        print_node(storage, f, &mut marked, ldd)?;
    }

    writeln!(f, "}}")
}

pub struct LddDisplay<'a> {
    storage: &'a Storage,
    ldd: Ldd,
}

impl fmt::Display for LddDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{{")?;
        print(self.storage, &self.ldd, f)?;
        write!(f, "}}")
    }
}

/// Print the vector set represented by the LDD.
fn print(storage: &Storage, ldd: &Ldd, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    for vector in iter(storage, ldd) {
        writeln!(f, "[{}]", vector.iter().format(" "))?;
    }

    Ok(())
}

#[allow(clippy::mutable_key_type)]
fn print_node(storage: &Storage, f: &mut impl Write, marked: &mut HashSet<Ldd>, ldd: &Ldd) -> io::Result<()> {
    if marked.contains(ldd) || ldd == storage.empty_set() || ldd == storage.empty_vector() {
        Ok(())
    } else {
        // Print the node values
        write!(f, "{} [shape=record, label=\"", ldd.index())?;

        let mut first = true;
        for Data(value, _, _) in iter_right(storage, ldd) {
            if !first {
                write!(f, "|")?;
            }

            write!(f, "<{value}> {value}")?;
            first = false;
        }
        writeln!(f, "\"];")?;

        // Print the edges.
        for Data(value, down, _) in iter_right(storage, ldd) {
            if down != *storage.empty_set() && down != *storage.empty_vector() {
                writeln!(
                    f,
                    "{}:{} -> {}:{};",
                    ldd.index(),
                    value,
                    down.index(),
                    storage.get_ref(&down).0
                )?;
            }
        }

        // Print all nodes.
        for Data(_, down, _) in iter_right(storage, ldd) {
            print_node(storage, f, marked, &down)?;
        }

        Ok(())
    }
}
