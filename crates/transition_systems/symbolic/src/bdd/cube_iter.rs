use std::marker::PhantomData;

use oxidd::BooleanFunction;
use oxidd::Function;
use oxidd::ManagerRef;
use oxidd::VarNo;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;
use oxidd::util::AllocResult;
use oxidd::util::OptBool;
use oxidd::util::OutOfMemory;
use oxidd::util::Ref;
use oxidd_core::function::EdgeOfFunc;
use oxidd_core::function::OwnEdgeOfFunc;

/// Returns the boolean set difference of two BDD functions: lhs \ rhs.
/// Implemented as lhs AND (NOT rhs).
pub fn minus(lhs: &BDDFunction, rhs: &BDDFunction) -> AllocResult<BDDFunction> {
    rhs.imp_strict(lhs)
}

/// Variant of [minus] that works on edges.
pub fn minus_edge<'id>(
    manager: &<BDDFunction as Function>::Manager<'id>,
    lhs: Ref<'_, EdgeOfFunc<'id, BDDFunction>>,
    rhs: Ref<'_, EdgeOfFunc<'id, BDDFunction>>,
) -> AllocResult<OwnEdgeOfFunc<'id, BDDFunction>> {
    BDDFunction::imp_strict_edge(manager, rhs, lhs)
}

/// Iterator over all cubes (satisfying assignments) in a BDD.
///
/// The returned cubes contain don't care values (OptBool::None) for variables
/// that can be either true or false without affecting the satisfaction of the
/// BDD.
///
/// Returns Result to handle out of memory errors.
pub struct CubeIter<'a> {
    /// The BDD to iterate over.
    bdd: BDDFunction,

    _marker: PhantomData<&'a ()>,
}

impl<'a> CubeIter<'a> {
    /// Creates a new cube iterator for the given BDD.
    pub fn new(bdd: &'a BDDFunction) -> Self {
        Self {
            bdd: bdd.clone(),
            _marker: PhantomData,
        }
    }
}

impl Iterator for CubeIter<'_> {
    type Item = Result<(Vec<OptBool>, BDDFunction), OutOfMemory>;

    fn next(&mut self) -> Option<Self::Item> {
        // We need to turn Err into Some(Err) to satisfy the Iterator trait.
        let cube = match self.bdd.pick_cube_dd(|_, _, _| true) {
            Ok(cube) => cube,
            Err(e) => return Some(Err(e)),
        };

        self.bdd = match minus(&self.bdd, &cube) {
            Ok(bdd) => bdd,
            Err(e) => return Some(Err(e)),
        };

        cube.pick_cube(|_, _, _| true).map(|bits| Ok((bits, cube)))
    }
}

/// Iterates over all satisfying assignments without considering don't care
/// values.
///
/// # Details
///
/// For the universe BDD, the `CubeIter` yields only one cube with all don't
/// cares, while this iterator yields all possible cubes. When
/// `variable_indices` is provided, returned assignments are projected to only
/// those variables.
///
/// Note that with `variable_indices` set, there can be multiple (identical)
/// cubes returned if the original cube has relevant outside of the projected
/// variables. This can be avoided by first restricting the input BDD to only
/// the desired `variable_indices`.
pub struct CubeIterAll<'a> {
    /// Iterator over the cubes with don't cares.
    iter: CubeIter<'a>,

    /// The current cube returned from CubeIter.
    cube: Option<Vec<OptBool>>,

    /// The cube that is currently being iterated over.
    current_cube: Vec<OptBool>,

    /// Optional set of variable indices to project the output onto.
    variable_indices: Option<&'a [VarNo]>,
}

impl<'a> CubeIterAll<'a> {
    /// Creates a new cube iterator that defers initialization to the first `next` call.
    pub fn new(bdd: &'a BDDFunction) -> Self {
        Self {
            iter: CubeIter::new(bdd),
            cube: None,
            current_cube: Vec::new(),
            variable_indices: None,
        }
    }

    /// Creates a new cube iterator that returns assignments only for the given variable indices.
    pub fn with_variables(bdd: &'a BDDFunction, variable_indices: &'a [VarNo]) -> Self {
        Self {
            iter: CubeIter::new(bdd),
            cube: None,
            current_cube: Vec::new(),
            variable_indices: Some(variable_indices),
        }
    }

    /// Initialize the `current` cube: project onto the selected variables (if requested)
    /// and replace the remaining don't cares with false to form the first assignment.
    fn initialize_cube(&mut self) {
        if let Some(cube) = &mut self.cube {
            // Project to the selected variable indices first, so the concretized assignment
            // below is over the projected space and never contains a don't care.
            if let Some(indices) = &self.variable_indices {
                *cube = project_bits(cube, indices);
            }

            self.current_cube = cube
                .iter()
                .map(|element| {
                    if *element == OptBool::None {
                        OptBool::False
                    } else {
                        *element
                    }
                })
                .collect();
        }
    }
}

impl Iterator for CubeIterAll<'_> {
    type Item = Result<Vec<OptBool>, OutOfMemory>;

    fn next(&mut self) -> Option<Self::Item> {
        // Lazy initialization on first call.
        if self.cube.is_none() {
            self.cube = match self.iter.next() {
                Some(Ok((bits, _))) => Some(bits),
                Some(Err(e)) => return Some(Err(e)),
                None => return None,
            };

            self.initialize_cube();
        }

        let cube = self
            .cube
            .as_ref()
            .expect("The cube is Some, otherwise the iterator would have returned None above");
        let result = self.current_cube.clone();

        // Advance to the next assignment for this cube. If overflow,
        // move to the next cube and initialize its current assignment.
        if !increment(&mut self.current_cube, cube) {
            self.cube = match self.iter.next() {
                Some(Ok((bits, _))) => Some(bits), // We cannot use the cube since we are going to fill in the don't cares.
                Some(Err(e)) => return Some(Err(e)),
                None => None,
            };

            self.initialize_cube();
        }

        Some(Ok(result))
    }
}

/// Constructs a BDD representing the given vector of (optional) bits.
pub fn bits_to_bdd(
    manager_ref: &BDDManagerRef,
    variables: &[BDDFunction],
    cube: &[OptBool],
) -> AllocResult<BDDFunction> {
    manager_ref.with_manager_shared(|manager| {
        let mut result = BDDFunction::t(manager);

        for (var, value) in variables.iter().zip(cube.iter()) {
            match value {
                OptBool::True => {
                    result = result.and(var)?;
                }
                OptBool::False => {
                    let not_var = var.not()?;
                    result = result.and(&not_var)?;
                }
                OptBool::None => {
                    // Don't care, skip
                }
            }
        }

        Ok(result)
    })
}

/// Perform the binary increment, returns false if overflow occurs.
///
/// Only considers bits for which the `cube` has don't care values, since these
/// are the only ones that can be changed.
fn increment(current_cube: &mut [OptBool], cube: &[OptBool]) -> bool {
    // Propagate carry across don't-care positions, resetting carried bits to false.
    let mut carry = true;
    for i in 0..current_cube.len() {
        if cube[i] != OptBool::None {
            continue;
        }
        match current_cube[i] {
            // Treat None as False for robustness.
            OptBool::None | OptBool::False => {
                // Found a bit to flip; set it to True and stop carrying.
                current_cube[i] = OptBool::True;
                carry = false;
                break;
            }
            OptBool::True => {
                // Reset this don't-care bit and continue carrying to the next.
                current_cube[i] = OptBool::False;
            }
        }
    }

    // All relevant variables were true (or there were none), overflow.
    !carry
}

/// Project bits onto the given variable indices.
fn project_bits(bits: &[OptBool], variable_indices: &[VarNo]) -> Vec<OptBool> {
    variable_indices
        .iter()
        .map(|&i| bits.get(i as usize).copied().expect("Projecting out of bounds"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use itertools::Itertools;

    use merc_utilities::MercError;
    use merc_utilities::random_test;
    use oxidd::BooleanFunction;
    use oxidd::Manager;
    use oxidd::ManagerRef;
    use oxidd::bdd::BDDFunction;
    use oxidd::util::OptBool;

    use crate::BDD_CACHE_CAPACITY;
    use crate::BDD_NODE_CAPACITY;
    use crate::CubeIter;
    use crate::CubeIterAll;
    use crate::FormatConfig;
    use crate::bdd_from_iter;
    use crate::random_bitvectors;

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_cube_iter_all() {
        random_test(100, |rng| {
            let manager_ref = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);
            let set = random_bitvectors(rng, 5, 20);
            println!("Set: {:?}", set.iter().format_with(", ", |v, f| f(&FormatConfig(v))));

            let variables = manager_ref
                .with_manager_exclusive(|manager| -> Result<Vec<BDDFunction>, MercError> {
                    Ok(manager
                        .add_vars(5)
                        .map(|i| BDDFunction::var(manager, i))
                        .collect::<Result<Vec<BDDFunction>, _>>()?)
                })
                .expect("Failed to create variables");

            let bdd = bdd_from_iter(&manager_ref, &variables, set.iter()).unwrap();

            // Check that the cube iterator yields all the expected cubes
            let mut seen = HashSet::new();
            for bits in CubeIterAll::new(&bdd) {
                let bits = bits.expect("Failed to iterate cubes");

                println!("Cube: {}", FormatConfig(&bits));
                assert!(!bits.contains(&OptBool::None), "Cube has no don't cares:");
                assert!(set.contains(&bits), "Cube {} not in expected set", FormatConfig(&bits));
                assert!(
                    seen.insert(bits.clone()),
                    "Duplicate cube found: {}",
                    FormatConfig(&bits)
                );
            }

            let cubes: Vec<Vec<OptBool>> = CubeIterAll::new(&bdd)
                .collect::<Result<Vec<_>, _>>()
                .expect("Failed to iterate cubes");
            for cube in &set {
                let found = cubes.iter().find(|bits| *bits == cube);
                assert!(found.is_some(), "Expected cube {} not found", FormatConfig(cube));
            }
        })
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_cube_iter() {
        random_test(100, |rng| {
            let manager_ref = oxidd::bdd::new_manager(BDD_NODE_CAPACITY, BDD_CACHE_CAPACITY, 1);
            let set = random_bitvectors(rng, 5, 20);
            println!("Set: {:?}", set.iter().format_with(", ", |v, f| f(&FormatConfig(v))));

            let variables = manager_ref
                .with_manager_exclusive(|manager| -> Result<Vec<BDDFunction>, MercError> {
                    Ok(manager
                        .add_vars(5)
                        .map(|i| BDDFunction::var(manager, i))
                        .collect::<Result<Vec<BDDFunction>, _>>()?)
                })
                .expect("Failed to create variables");

            let bdd = bdd_from_iter(&manager_ref, &variables, set.iter()).unwrap();

            // Check that it does not yield duplicates.
            let mut seen = HashSet::new();
            for cube in CubeIter::new(&bdd) {
                let (cube, _) = cube.expect("Failed to iterate cubes");
                println!("Cube: {}", FormatConfig(&cube));

                assert!(
                    seen.insert(cube.clone()),
                    "Duplicate cube found: {}",
                    FormatConfig(&cube)
                );
            }
        })
    }
}
