use std::fmt;
use std::ops::Index;

use merc_symbolic::minus_edge;
use merc_symbolic::FormatConfigSet;
use oxidd::bdd::BDDFunction;
use oxidd::bdd::BDDManagerRef;
use oxidd::BooleanFunction;
use oxidd::Function;
use oxidd::ManagerRef;
use oxidd_core::util::EdgeDropGuard;

use merc_utilities::MercError;

use crate::VertexIndex;

/// A mapping from vertices to configurations.
///
/// # Details
///
/// Internally this implementation uses the manager and the `edge` functions
/// directly for efficiency reasons. Every BDDFunction typically calls
/// `with_manager_shared` internally, which induces significant overhead for
/// many vertices/operations.
#[derive(Clone, PartialEq, Eq)]
pub struct Submap {
    /// The mapping from vertex indices to BDD functions.
    mapping: Vec<BDDFunction>,

    /// Invariant: counts the number of non-empty positions in the mapping.
    non_empty_count: usize,

    /// A cached reference to the false BDD function.
    false_bdd: BDDFunction,
}

impl Submap {
    /// Creates a new empty Submap for the given number of vertices.
    pub fn new(manager_ref: &BDDManagerRef, initial: BDDFunction, num_of_vertices: usize) -> Self {
        Self {
            mapping: vec![initial.clone(); num_of_vertices],
            non_empty_count: if initial.satisfiable() {
                num_of_vertices // If the initial function is satisfiable, all entries are non-empty.
            } else {
                0
            },
            false_bdd: manager_ref.with_manager_shared(|manager| BDDFunction::f(manager)),
        }
    }

    /// Returns an iterator over the vertices in the submap whose configuration is satisfiable.
    pub fn iter_vertices(&self) -> impl Iterator<Item = VertexIndex> + '_ {
        self.mapping.iter().enumerate().filter_map(|(i, func)| {
            if func.satisfiable() {
                Some(VertexIndex::new(i))
            } else {
                None
            }
        })
    }

    /// Returns the number of non-empty entries in the submap.
    pub fn number_of_non_empty(&self) -> usize {
        self.non_empty_count
    }

    /// Sets the function for the given vertex index.
    ///
    /// Takes an internal manager to avoid repeated calls to [oxidd:Manager::with_manager_shared].
    pub fn set<'id>(
        &mut self,
        manager: &<BDDFunction as Function>::Manager<'id>,
        index: VertexIndex,
        func: BDDFunction,
    ) {
        let was_empty = self.mapping[*index].as_edge(manager) == self.false_bdd.as_edge(manager);
        let is_empty = func.as_edge(manager) == self.false_bdd.as_edge(manager);

        self.mapping[*index] = func;

        // Update the non-empty count invariant.
        if was_empty && !is_empty {
            self.non_empty_count += 1;
        } else if !was_empty && is_empty {
            self.non_empty_count -= 1;
        }
    }

    /// Returns true iff the submap is empty.
    pub fn is_empty(&self) -> bool {
        self.non_empty_count == 0
    }

    /// Returns the number of entries in the submap.
    pub fn len(&self) -> usize {
        self.mapping.len()
    }

    /// Clears the submap, setting all entries to the empty function.
    pub fn clear(&mut self, manager_ref: &BDDManagerRef) -> Result<(), MercError> {
        manager_ref.with_manager_shared(|manager| {
            for func in self.mapping.iter_mut() {
                *func = BDDFunction::f(manager);
            }
            self.non_empty_count = 0;
        });

        Ok(())
    }

    /// Computes the difference between this submap and another submap.
    pub fn minus(mut self, manager_ref: &BDDManagerRef, other: &Submap) -> Result<Submap, MercError> {
        manager_ref.with_manager_shared(|manager| -> Result<(), MercError> {
            let f_edge = EdgeDropGuard::new(manager, BDDFunction::f_edge(manager));
            for (i, func) in self.mapping.iter_mut().enumerate() {
                let was_satisfiable = *func.as_edge(manager) != *f_edge;
                if was_satisfiable {
                    *func = BDDFunction::from_edge(
                        manager,
                        BDDFunction::imp_strict_edge(
                            manager,
                            &other.mapping[i].as_edge(manager),
                            func.as_edge(manager),
                        )?,
                    );
                    let is_satisfiable = *func.as_edge(manager) != *f_edge;

                    if was_satisfiable && !is_satisfiable {
                        self.non_empty_count -= 1;
                    }
                }
            }

            Ok(())
        })?;

        Ok(self)
    }

    /// Computes the union between this submap and another submap.
    pub fn or(mut self, manager_ref: &BDDManagerRef, other: &Submap) -> Result<Submap, MercError> {
        manager_ref.with_manager_shared(|manager| -> Result<(), MercError> {
            let f_edge = EdgeDropGuard::new(manager, BDDFunction::f_edge(manager));

            for (i, func) in self.mapping.iter_mut().enumerate() {
                let func_edge = func.as_edge(manager);

                let was_satisfiable = *func_edge != *f_edge;
                let new_func = BDDFunction::or_edge(manager, func_edge, other.mapping[i].as_edge(manager))?;
                let is_satisfiable = new_func != *f_edge;

                *func = BDDFunction::from_edge(manager, new_func);

                if !was_satisfiable && is_satisfiable {
                    self.non_empty_count += 1;
                }
            }

            Ok(())
        })?;

        Ok(self)
    }

    /// Computes the intersection between this submap and another function.
    pub fn and_function(
        mut self,
        manager_ref: &BDDManagerRef,
        configuration: &BDDFunction,
    ) -> Result<Submap, MercError> {
        manager_ref.with_manager_shared(|manager| -> Result<(), MercError> {
            let f_edge = EdgeDropGuard::new(manager, BDDFunction::f_edge(manager));

            for func in self.mapping.iter_mut() {
                let func_edge = func.as_edge(manager);

                let was_satisfiable = *func_edge != *f_edge;
                let new_func = BDDFunction::and_edge(manager, func_edge, configuration.as_edge(manager))?;
                let is_satisfiable = new_func != *f_edge;

                *func = BDDFunction::from_edge(manager, new_func);

                if was_satisfiable && !is_satisfiable {
                    self.non_empty_count -= 1;
                }
            }

            Ok(())
        })?;

        Ok(self)
    }

    /// Computes the difference between this submap and another function.
    pub fn minus_function(
        mut self,
        manager_ref: &BDDManagerRef,
        configuration: &BDDFunction,
    ) -> Result<Submap, MercError> {
        manager_ref.with_manager_shared(|manager| -> Result<(), MercError> {
            let f_edge = EdgeDropGuard::new(manager, BDDFunction::f_edge(manager));
            let conf_edge = configuration.as_edge(manager);

            for func in self.mapping.iter_mut() {
                let func_edge = func.as_edge(manager);

                let was_satisfiable = *func_edge != *f_edge;
                let new_func = minus_edge(manager, func_edge, conf_edge)?;
                let is_satisfiable = new_func != *f_edge;

                *func = BDDFunction::from_edge(manager, new_func);

                if was_satisfiable && !is_satisfiable {
                    self.non_empty_count -= 1;
                }
            }

            Ok(())
        })?;

        Ok(self)
    }

    /// Returns an iterator over all entries.
    pub fn iter(&self) -> impl Iterator<Item = (VertexIndex, &BDDFunction)> {
        self.mapping
            .iter()
            .enumerate()
            .map(|(i, func)| (VertexIndex::new(i), func))
    }
}

impl Index<VertexIndex> for Submap {
    type Output = BDDFunction;

    fn index(&self, index: VertexIndex) -> &Self::Output {
        &self.mapping[*index]
    }
}

impl fmt::Debug for Submap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, func) in self.mapping.iter().enumerate() {
            if func.satisfiable() {
                write!(f, " {} ({})", i, FormatConfigSet(func))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use merc_macros::merc_test;
    use oxidd::bdd::BDDFunction;
    use oxidd::util::AllocResult;
    use oxidd::BooleanFunction;
    use oxidd::Manager;
    use oxidd::ManagerRef;

    use crate::Submap;
    use crate::VertexIndex;

    #[merc_test]
    #[cfg_attr(miri, ignore)] // Oxidd does not work with miri
    fn test_submap() {
        let manager_ref = oxidd::bdd::new_manager(2048, 1024, 1);
        let vars: Vec<BDDFunction> = manager_ref
            .with_manager_exclusive(|manager| {
                AllocResult::from_iter(manager.add_vars(3).map(|i| BDDFunction::var(manager, i)))
            })
            .expect("Could not create variables");

        let false_bdd = manager_ref.with_manager_shared(|manager| BDDFunction::f(manager));
        let mut submap = Submap::new(&manager_ref, false_bdd.clone(), 3);

        assert_eq!(submap.len(), 3);
        assert_eq!(submap.non_empty_count, 0);

        manager_ref.with_manager_shared(|manager| {
            submap.set(manager, VertexIndex::new(0), vars[0].clone());
        });

        assert_eq!(submap.non_empty_count, 1);
    }
}
