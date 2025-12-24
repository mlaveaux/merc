use merc_utilities::MercError;
use oxidd::BooleanFunction;
use oxidd::Manager;
use oxidd::ManagerRef;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;
use oxidd::util::OptBool;
use rand::Rng;

/// Generate `num_vectors` random bitvectors of length `num_vars`.
pub fn random_bitvectors(rng: &mut impl Rng, num_vars: usize, num_vectors: usize) -> Vec<Vec<OptBool>> {
    let mut vectors = Vec::new();
    for _ in 0..rng.random_range(0..num_vectors) {
        let mut vec = Vec::new();
        for _ in 0..num_vars {
            vec.push(if rng.random_bool(0.5) {
                OptBool::True
            } else {
                OptBool::False
            });
        }
        vectors.push(vec);
    }
    vectors
}

/// Create a BDD from the given bitvector.
pub fn from_iter<'a>(
    manager_ref: &BDDManagerRef,
    variables: &[BDDFunction],
    vectors: impl Iterator<Item = &'a Vec<OptBool>>,
) -> Result<BDDFunction, MercError> {
    let mut bdd = manager_ref.with_manager_shared(|manager| BDDFunction::f(manager));
    for bits in vectors {
        let mut cube = manager_ref.with_manager_shared(|manager| BDDFunction::t(manager));
        // Create a cube for this bitvector
        for (i, bit) in bits.iter().enumerate() {
            let var = variables[i].clone();
            let literal = match *bit {
                OptBool::True => var,
                OptBool::False => var.not()?,
                OptBool::None => continue,
            };
            cube = cube.and(&literal)?;
        }

        bdd = bdd.or(&cube)?;
    }

    Ok(bdd)
}

/// Create a random BDD over the given variables with the given number of cubes.
pub fn random_bdd(
    manager_ref: &BDDManagerRef,
    rng: &mut impl Rng,
    variables: &[BDDFunction],
) -> Result<BDDFunction, MercError> {
    let bitvectors = random_bitvectors(rng, variables.len(), 100);
    from_iter(manager_ref, variables, bitvectors.iter())
}

/// Create the given number of variables in the BDD manager.
pub fn create_variables(manager_ref: &BDDManagerRef, num_vars: u32) -> Result<Vec<BDDFunction>, MercError> {
    Ok(manager_ref.with_manager_exclusive(|manager| {
        manager
            .add_vars(num_vars)
            .map(|i| BDDFunction::var(manager, i))
            .collect::<Result<Vec<_>, _>>()
    })?)
}
