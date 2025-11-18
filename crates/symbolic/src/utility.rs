use merc_ldd::Ldd;
use merc_ldd::Storage;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;

fn ldd_to_bdd(storage: &mut Storage, _manager: BDDManagerRef, input: &Ldd, _bits: &Ldd, _first_variable: usize) -> BDDFunction {
    if storage.empty_set() == input {
        // return manager.::false_function();
    }

    unimplemented!();
}

#[cfg(test)]
mod tests {}
