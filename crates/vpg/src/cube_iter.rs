use oxidd::{bdd::BDDFunction, util::OptBool};
use oxidd::BooleanFunction;

/// Iterator over all cubes (satisfying assignments) in a BDD.
pub struct CubeIter<'a> {
    /// The BDD to iterate over.
    bdd: &'a BDDFunction,
    /// The current choices for each variable.
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

        let cube = self.bdd.pick_cube(|_manager, _edge, index| {
            // Ensure that the choices vector is large enough, initialize with don't care
            let mut resized = false;
            if index as usize >= self.choices.len() {
                resized = true;
                self.choices.resize(index as usize + 1, OptBool::None);
            }

            // If we have skipped levels then the intermediate variables should be don't care
            for i in (self.last_index as usize + 1)..(index as usize) {
                self.choices[i] = OptBool::None;
            }

            if index <= self.last_index {
                // Set all ones to zero, and initialize the next index on true
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
    
        // Check if all choices are None, then we are done
        if self.choices.iter().all(|x| *x == OptBool::None) {
            self.done = true;
        }

        cube
    }
}
