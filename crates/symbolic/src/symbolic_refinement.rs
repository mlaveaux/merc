use oxidd::bdd::{self};

use crate::SymbolicLts;

pub fn run(_lts: &SymbolicLts) {
    let mut _manager = bdd::new_manager(2048, 1024, 8);
}
