//! Iterator over cubes in a BDD.

use std::marker::PhantomData;

use merc_utilities::MercError;
use oxidd::BooleanFunction;
use oxidd::bdd::BDDFunction;
use oxidd::util::OptBool;

use crate::minus;

/// Iterator over all cubes (satisfying assignments) in a BDD.
///
/// The returned cubes contain don't care values (OptBool::None) for variables
/// that can be either true or false without affecting the satisfaction of the
/// BDD.
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
    type Item = Vec<OptBool>;

    fn next(&mut self) -> Option<Self::Item> {
        let cube = self.bdd.pick_cube_dd(|_, _, _| true).unwrap();

        self.bdd = minus(&self.bdd, &cube).expect("Failed to compute BDD difference");

        cube.pick_cube(|_, _, _| true)
    }
}

/// The same as [CubeIter], but iterates over all satisfying assignments without
/// considering don't care values. For the universe BDD, the [CubeIter] yields only
/// one cube with all don't cares, while this iterator yields all possible cubes.
pub struct CubeIterAll<'a> {
    bdd: &'a BDDFunction,
    // The variables used in the BDD.
    variables: &'a Vec<BDDFunction>,
    // The last cube generated.
    cube: Vec<OptBool>,
    // Whether to stop the iteration.
    done: bool,
}

impl<'a> CubeIterAll<'a> {
    /// Creates a new cube iterator that iterates over the single cube
    pub fn new(variables: &'a Vec<BDDFunction>, bdd: &'a BDDFunction) -> CubeIterAll<'a> {
        let cube = Vec::from_iter((0..variables.len()).map(|_| OptBool::False));
        Self {
            bdd,
            cube,
            variables,
            done: false,
        }
    }
}

impl Iterator for CubeIterAll<'_> {
    type Item = Result<(Vec<OptBool>, BDDFunction), MercError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        loop {
            let mut tmp = self.bdd.clone();
            for (index, value) in self.cube.iter().enumerate() {
                if *value == OptBool::True {
                    tmp = match tmp.and(&self.variables[index]) {
                        Ok(val) => val,
                        Err(e) => return Some(Err(e.into())),
                    };
                } else {
                    let not_var = match self.variables[index].not() {
                        Ok(val) => val,
                        Err(e) => return Some(Err(e.into())),
                    };
                    tmp = match tmp.and(&not_var) {
                        Ok(val) => val,
                        Err(e) => return Some(Err(e.into())),
                    };
                }

                if !tmp.satisfiable() {
                    // This cube is not satisfying, try the next one, or quit if overflow
                    if !increment(&mut self.cube) {
                        return None;
                    }
                    break;
                }
            }

            if tmp.satisfiable() {
                let result = self.cube.clone();
                // The next iteration overflows, we are done
                self.done = !increment(&mut self.cube);
                return Some(Ok((result, tmp)));
            }
        }
    }
}

/// Perform the binary increment, returns false if overflow occurs.
fn increment(cube: &mut [OptBool]) -> bool {
    for value in cube.iter_mut() {
        // Set each variable to true until we find one that is false
        if *value == OptBool::False {
            *value = OptBool::True;
            return true;
        }

        *value = OptBool::False;
    }

    // All variables were true, overflow
    false
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use itertools::Itertools;

    use merc_utilities::MercError;
    use merc_utilities::random_test;
    use oxidd::bdd::BDDFunction;
    use oxidd::util::OptBool;

    use crate::CubeIter;
    use crate::CubeIterAll;
    use crate::FormatConfig;
    use crate::create_variables;
    use crate::from_iter;
    use crate::random_bitvectors;

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_cube_iter_all() {
        random_test(100, |rng| {
            let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);
            let set = random_bitvectors(rng, 5, 20);
            println!("Set: {:?}", set.iter().format_with(", ", |v, f| f(&FormatConfig(v))));

            let variables = create_variables(&manager_ref, 5).unwrap();

            let bdd = from_iter(&manager_ref, &variables, set.iter()).unwrap();

            // Check that the cube iterator yields all the expected cubes
            let result: Result<Vec<(Vec<OptBool>, BDDFunction)>, MercError> =
                CubeIterAll::new(&variables, &bdd).collect();
            let cubes: Vec<(Vec<OptBool>, BDDFunction)> = result.unwrap();
            let mut seen = HashSet::new();
            for (bits, _) in &cubes {
                println!("Cube: {}", FormatConfig(&bits));
                assert!(set.contains(&bits), "Cube {} not in expected set", FormatConfig(&bits));
                assert!(
                    seen.insert(bits.clone()),
                    "Duplicate cube found: {}",
                    FormatConfig(&bits)
                );
            }

            for cube in &set {
                let found = cubes.iter().find(|(bits, _)| bits == cube);
                assert!(found.is_some(), "Expected cube {} not found", FormatConfig(cube));
            }
        })
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_cube_iter() {
        random_test(100, |rng| {
            let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);
            let set = random_bitvectors(rng, 5, 20);
            println!("Set: {:?}", set.iter().format_with(", ", |v, f| f(&FormatConfig(v))));

            let variables = create_variables(&manager_ref, 5).unwrap();

            let bdd = from_iter(&manager_ref, &variables, set.iter()).unwrap();

            // Check that it does not yield duplicates.
            let mut seen = HashSet::new();
            for cube in CubeIter::new(&bdd) {
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
