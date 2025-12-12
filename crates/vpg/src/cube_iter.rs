//! Iterator over cubes in a BDD.

use merc_utilities::MercError;
use oxidd::BooleanFunction;
use oxidd::bdd::BDDFunction;
use oxidd::util::OptBool;

/// Iterator over all cubes (satisfying assignments) in a BDD.
///
/// The returned cubes contain don't care values (OptBool::None) for variables
/// that can be either true or false without affecting the satisfaction of the
/// BDD.
pub struct CubeIter<'a> {
    /// The BDD to iterate over.
    bdd: &'a BDDFunction,
    /// The current choices for each variable, OptBool::None means don't care.
    choices: Vec<OptBool>,
    /// Keeps track of the last index visited by pick_cube
    last_index: u32,
    /// Whether all cubes have been iterated over.
    done: bool,
}

impl<'a> CubeIter<'a> {
    /// Creates a new cube iterator for the given BDD.
    pub fn new(bdd: &'a BDDFunction) -> Self {
        Self {
            bdd,
            choices: Vec::new(),
            last_index: 0,
            done: false,
        }
    }
}

impl Iterator for CubeIter<'_> {
    type Item = Vec<OptBool>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        // We essentially perform addition on binary sequences (where 1 = true, 0 =
        // false, and don't care is skipped). Whenever index <= last_index is
        // encountered, we flip all 1s to 0s until we find a 0 to flip to 1.
        // Furthermore, if we skip indices, we set the intermediate indices to
        // don't care. Finally, when they are all one then we are done.
        let cube = self.bdd.pick_cube(|_manager, _edge, index| {
            // Ensure that the choices vector is large enough, initialize with don't care
            let mut resized = false;
            if index as usize >= self.choices.len() {
                resized = true;
                self.choices.resize(index as usize + 1, OptBool::None);
            }

            // If we have skipped levels then the intermediate variables should be don't cares.
            for i in (self.last_index as usize + 1)..(index as usize) {
                self.choices[i] = OptBool::None;
            }

            if index <= self.last_index {
                // Set all ones to zero, and initialize the next index to true
                let mut had_false = false;
                for i in 0..self.choices.len() {
                    if self.choices[i] == OptBool::True {
                        self.choices[i] = OptBool::False;
                    } else if self.choices[i] == OptBool::False {
                        self.choices[i] = OptBool::True;
                        had_false = true;
                        break; // Skip updating further indices
                    }
                }

                if !had_false && !resized {
                    // All choices with 1 have been taken, so abort.
                    self.done = true;
                }
            }

            // Update the choice for the current index
            self.last_index = index;

            if self.choices[index as usize] == OptBool::None {
                // First time setting this index, it should be false
                self.choices[index as usize] = OptBool::False;
            }

            match self.choices[index as usize] {
                OptBool::False => true,
                OptBool::True => false,
                OptBool::None => unreachable!("Proper choice should have been set"),
            }
        });

        // Check if all choices are None, then we are also done (since the
        // choice function is not called and the set is the universe) we must
        // deal with it here.
        if self.choices.iter().all(|x| *x == OptBool::None) {
            self.done = true;
        }

        cube
    }
}

/// The same as [CubeIter], but iterates over all satisfying assignments without
/// considering don't care values. For the universe BDD, the [CubeIter] yields only
/// one cube with all don't cares, while this iterator yields all possible cubes.
pub struct CubeIterAll<'a> {
    bdd: &'a BDDFunction,

    cube: Vec<OptBool>,

    stop: bool,

    variables: &'a Vec<BDDFunction>,
}

impl<'a> CubeIterAll<'a> {
    /// Creates a new cube iterator that iterates over the single cube
    pub fn new(variables: &'a Vec<BDDFunction>, bdd: &'a BDDFunction) -> CubeIterAll<'a> {
        let cube = Vec::from_iter((0..variables.len()).map(|_| OptBool::False));
        Self { bdd, cube, variables, stop: false }
    }
}

impl Iterator for CubeIterAll<'_> {
    type Item = Result<(Vec<OptBool>, BDDFunction), MercError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.stop {
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
                self.stop = !increment(&mut self.cube);
                return Some(Ok((result, tmp)));
            }
        }
    }
}

/// Perform the binary increment, returns false if overflow occurs.
fn increment(cube: &mut Vec<OptBool>) -> bool {
    for value in cube.iter_mut() {
        // Set each variable to true until we find one that is false
        if *value == OptBool::False {
            *value = OptBool::True;
            return true;
        }

        *value = OptBool::False;
    }

    // ALl variables were true, overflow
    false
}

#[cfg(test)]
mod tests {
    use itertools::Itertools;

    use merc_utilities::MercError;
    use merc_utilities::random_test;
    use oxidd::bdd::BDDFunction;
    use oxidd::util::OptBool;

    use crate::CubeIterAll;
    use crate::FormatConfig;
    use crate::create_variables;
    use crate::from_iter;
    use crate::random_bitvectors;

    #[test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_random_cube_iter() {
        random_test(100, |rng| {
            let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);
            let set = random_bitvectors(rng, 5, 20);
            println!("Set: {:?}", set.iter().format_with(", ", |v, f| f(&FormatConfig(v))));

            let variables = create_variables(&manager_ref, 5).unwrap();

            let bdd = from_iter(&manager_ref, &variables, set.iter()).unwrap();

            // Check that the cube iterator yields all the expected cubes
            let result: Result<Vec<(Vec<OptBool>, BDDFunction)>, MercError> = CubeIterAll::new(&variables, &bdd).collect();
            let cubes: Vec<(Vec<OptBool>, BDDFunction)> = result.unwrap();
            for (bits, _) in &cubes{
                assert!(set.contains(&bits), "Cube {} not in expected set", FormatConfig(&bits));
            }

            for cube in &set {
                let found = cubes.iter().find(|(bits, _)| bits == cube);
                assert!(found.is_some(), "Expected cube {} not found", FormatConfig(cube));
            }
        })
    }
}
