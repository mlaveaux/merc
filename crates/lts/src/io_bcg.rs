//! Suppress various warnings from the generated bindings.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(unused)]

use std::path::Path;

use merc_utilities::MercError;

use crate::LTS;
use crate::LabelledTransitionSystem;

#[cfg(not(feature = "cadp"))]
mod inner {
    use super::*;

    /// This is a stub implementation used when BCG support is not compiled in.
    pub fn read_bcg(_path: &Path, _hidden_labels: Vec<String>) -> Result<LabelledTransitionSystem<String>, MercError> {
        Err("BCG format support not compiled in, see the 'cadp' feature.".into())
    }

    /// This is a stub implementation used when BCG support is not compiled in.
    pub fn write_bcg(_lts: &impl LTS, _path: &Path) -> Result<(), MercError> {
        Err("BCG format support not compiled in, see the 'cadp' feature.".into())
    }
}

#[cfg(feature = "cadp")]
mod inner {
    use log::info;

    use super::*;

    use core::num;
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::env;
    use std::ffi::CStr;
    use std::ffi::CString;
    use std::pin::Pin;
    use std::sync::Mutex;
    use std::sync::Once;

    use merc_io::LargeFormatter;
    use merc_io::TimeProgress;

    use crate::LabelIndex;
    use crate::LtsBuilder;
    use crate::StateIndex;
    use crate::TransitionLabel;

    /// Initialize the BCG library exactly once.
    static BCG_INITIALIZED: Once = Once::new();

    /// Mutex to ensure thread-safe access to BCG library functions.
    static BCG_LOCK: Mutex<()> = Mutex::new(());

    // Include the generated bindings for the BCG C library.
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

    /// Reads a labelled transition system in the BCG format, from the
    /// [CADP](https://cadp.inria.fr/man/bcg.html) toolset.
    ///
    /// # Details
    ///
    /// This requires the `CADP` toolset to be installed for the target
    /// platform, and the `CADP` environment variable to be set.
    ///
    /// Note that the C library can only read files from disk; reading from
    /// in-memory buffers is not supported.
    pub fn read_bcg(path: &Path, hidden_labels: Vec<String>) -> Result<LabelledTransitionSystem<String>, MercError> {
        initialize_bcg()?;
        info!("Reading LTS in BCG format...");

        // Take the lock to ensure thread-safe access to BCG functions.
        let _guard = BCG_LOCK.lock().expect("Failed to acquire BCG lock");

        let mut bcg_object: BCG_TYPE_OBJECT_TRANSITION = std::ptr::null_mut();
        let filename = CString::new(path.to_string_lossy().as_ref())?;

        #[repr(u32)]
        enum AccessMode {
            Edges = 0,
            Succ = 1,
            Pred = 2,
            SuccPred = 3,
            SuccPredAlt = 4,
        }

        // SAFETY: The function will not modify the string.
        unsafe {
            BCG_OT_READ_BCG_BEGIN(
                filename.as_ptr() as *mut i8,
                &mut bcg_object,
                AccessMode::Succ as u32, // With successors we can efficiently create the LTS.
            );
        }

        // Read the labels.
        let num_of_labels = unsafe { BCG_OT_NB_LABELS(bcg_object) };

        let mut labels = Vec::with_capacity(num_of_labels as usize);
        labels.push(String::tau_label());

        let mut label_index = HashMap::new();
        for i in 0..num_of_labels {
            let label = unsafe { BCG_OT_LABEL_STRING(bcg_object, i) };

            let is_visible = unsafe { BCG_OT_LABEL_VISIBLE(bcg_object, i) };

            let label = unsafe { CStr::from_ptr(label).to_string_lossy().into_owned() };
            if is_visible {
                label_index.insert(i as usize, labels.len()); // Map to new index.
                labels.push(label.clone());
            } else {
                label_index.insert(i as usize, 0); // Map to the internal action.
            }
        }

        // Read the initial state.
        let initial_state = unsafe { BCG_OT_INITIAL_STATE(bcg_object) };

        let num_of_transitions = unsafe { BCG_OT_NB_EDGES(bcg_object) };
        let mut progress = TimeProgress::new(
            move |transitions: usize| {
                info!(
                    "Read {} transitions ({}%)...",
                    LargeFormatter(transitions),
                    transitions * 100 / num_of_transitions as usize
                );
            },
            1,
        );

        // Read the successors for every state.
        let num_of_states = unsafe { BCG_OT_NB_STATES(bcg_object) };
        let mut num_of_transitions = Cell::new(0usize);
        let lts = LabelledTransitionSystem::with_successors(
            StateIndex::new(initial_state as usize),
            num_of_states as usize,
            labels,
            |state| {
                unsafe { SuccessorIter::new(bcg_object, state.value() as u64) }.map(|edge| {
                    num_of_transitions.set(num_of_transitions.get() + 1);
                    progress.print(num_of_transitions.get());
                    (LabelIndex::new(label_index[&edge.label]), StateIndex::new(edge.target))
                })
            },
        );

        // Clean up
        unsafe {
            BCG_OT_READ_BCG_END(&mut bcg_object);
        }

        info!("Finished reading LTS.");
        Ok(lts)
    }

    /// Writes the given labelled transition system to a file in the BCG format, see [read_bcg].
    ///
    /// # Details
    ///
    /// We require the label to be convertible into a `String`.
    pub fn write_bcg<L: LTS>(lts: &L, path: &Path) -> Result<(), MercError>
    where
        String: From<L::Label>,
    {
        initialize_bcg()?;
        info!("Writing LTS in BCG format...");

        // Take the lock to ensure thread-safe access to BCG functions.
        let _guard = BCG_LOCK.lock().expect("Failed to acquire BCG lock");

        let filename = CString::new(path.to_string_lossy().as_ref())?;
        let comment = CString::new("created by merc_lts")?;

        #[repr(u32)]
        enum WriteMode {
            // In the forthcoming successive invocations of
            // function BCG_IO_WRITE_BCG_EDGE(), the sequence of actual values
            // given to the state1 argument of BCG_IO_WRITE_BCG_EDGE() will
            // increase monotonically
            MonotonicStates = 2,
        }

        // SAFETY: The C call will not modify the string.
        unsafe {
            BCG_IO_WRITE_BCG_BEGIN(
                filename.as_ptr() as *mut i8,
                lts.initial_state_index().value() as u64,
                WriteMode::MonotonicStates as u32,
                comment.as_ptr() as *mut i8,
                false,
            );
        }

        let num_of_transitions = lts.num_of_transitions();
        let mut progress = TimeProgress::new(
            move |transitions: usize| {
                info!(
                    "Wrote {} transitions ({}%)...",
                    LargeFormatter(transitions),
                    transitions * 100 / num_of_transitions
                );
            },
            1,
        );

        let labels = lts
            .labels()
            .iter()
            .map(|label| CString::new::<String>(label.clone().into()))
            .collect::<Result<Vec<_>, _>>()?;

        let mut number_of_transitions = 0;
        for state in lts.iter_states() {
            for transition in lts.outgoing_transitions(state) {
                // SAFETY: The state label is not mutated by the C function.
                unsafe {
                    BCG_IO_WRITE_BCG_EDGE(
                        state.value() as u64,
                        labels[transition.label.value() as usize].as_ptr() as *mut i8,
                        transition.to.value() as u64,
                    );
                }

                progress.print(number_of_transitions);
                number_of_transitions += 1;
            }
        }

        unsafe {
            BCG_IO_WRITE_BCG_END();
        }

        info!("Finished writing LTS.");
        Ok(())
    }

    /// Initialize the BCG library.
    fn initialize_bcg() -> Result<(), MercError> {
        BCG_INITIALIZED.call_once(|| {
            // SAFETY: Initialize the BCG library only once.
            unsafe { BCG_INIT() };
            info!("BCG library initialized.");
        });

        match env::var("CADP") {
            Ok(cadp_path) => {
                if Path::new(&cadp_path).exists() {
                    info!("Found CADP installation at: {}", cadp_path);
                } else {
                    return Err(format!("The CADP environment variable is set to '{}', but this path does not exist; the CADP toolset must be installed to read BCG files.", cadp_path).into());
                }
            }
            Err(_) => {
                return Err(
                    "The CADP environment variable is not set; the CADP toolset must be installed to read BCG files."
                        .into(),
                );
            }
        }

        Ok(())
    }

    /// Represents an edge in the BCG file.
    struct BcgEdge {
        source: usize,
        label: usize,
        target: usize,
    }

    // Iterator over all edges in the BCG fil, `BCG_OT_ITERATE_PLN`.
    struct EdgeIter {
        inner: BcgOtIterator,
    }

    impl EdgeIter {
        /// Create a new BCG OT iterator.
        unsafe fn new(bcg_object: BCG_TYPE_OBJECT_TRANSITION) -> Self {
            let mut inner = unsafe { BcgOtIterator::new() };

            unsafe { BCG_OT_START(inner.inner.as_mut().get_unchecked_mut(), bcg_object, bcg_enum_edge_sort_BCG_UNDEFINED_SORT) };
            Self { inner }
        }
    }

    impl Iterator for EdgeIter {
        type Item = BcgEdge;

        fn next(&mut self) -> Option<Self::Item> {
            // If we've reached the end, signal iteration end.
            if self.inner.end() {
                return None;
            }

            let edge = self.inner.edge();

            // Advance the underlying C iterator for the next call.
            unsafe {
                self.inner.next();
            }

            Some(edge)
        }
    }

    /// Iterator for the successors of a specific state, `BCG_OT_ITERATE_P_LN`.
    struct SuccessorIter {
        inner: BcgOtIterator,
        state: u64,
    }

    impl SuccessorIter {
        /// Constructs a new BCG OT iterator for a specific `state`.
        pub unsafe fn new(bcg_object: BCG_TYPE_OBJECT_TRANSITION, state: u64) -> Self {
            let mut inner = unsafe { BcgOtIterator::new() };

            unsafe { BCG_OT_START_P(inner.inner.as_mut().get_unchecked_mut(), bcg_object, bcg_enum_edge_sort_BCG_P_SORT, state) };
            Self { inner, state }
        }
    }

    impl Iterator for SuccessorIter {
        type Item = BcgEdge;

        fn next(&mut self) -> Option<Self::Item> {
            // If we've reached the end, or the state has changed, signal iteration end.
            if self.inner.end() || self.inner.p() != self.state {
                return None;
            }

            let edge = self.inner.edge();

            unsafe {
                self.inner.next();
            }

            Some(edge)
        }
    }

    /// Wrapper around the BCG OT iterator.
    struct BcgOtIterator {
        inner: Pin<Box<BCG_TYPE_OT_ITERATOR>>,
    }

    impl BcgOtIterator {
        /// Constructs a new BCG OT iterator
        pub unsafe fn new() -> Self {
            Self {
                inner: Box::pin(BCG_TYPE_OT_ITERATOR {
                    bcg_object_transition: std::ptr::null_mut(),
                    bcg_bcg_file_iterator: bcg_body_bcg_file_iterator { bcg_nb_edges: 0 },
                    bcg_et1_iterator: BCG_TYPE_ET1_ITERATOR {
                        bcg_edge_table: std::ptr::null_mut(),
                        bcg_current_state: 0,
                        bcg_last_edge_of_state: 0,
                        bcg_edge_number: 0,
                        bcg_edge_buffer: std::ptr::null_mut(),
                        bcg_given_state: false,
                    },
                    bcg_et2_iterator: BCG_TYPE_ET2_ITERATOR {
                        bcg_edge_table: std::ptr::null_mut(),
                        bcg_edge_number: 0,
                        bcg_index_number: 0,
                        bcg_edge_buffer: std::ptr::null_mut(),
                    },
                    bcg_edge_buffer: BCG_TYPE_EDGE {
                        bcg_end: false,
                        bcg_i: 0,
                        bcg_p: 0,
                        bcg_l: 0,
                        bcg_n: 0,
                    },
                }),
            }
        }

        /// Returns true if the iterator has reached the end, `BCG_OT_END`.
        fn end(&self) -> bool {
            self.inner.bcg_edge_buffer.bcg_end
        }

        /// Returns the current source state, `BCG_OT_P`.
        fn p(&self) -> u64 {
            self.inner.bcg_edge_buffer.bcg_p as u64
        }

        /// Returns the current edge.
        fn edge(&self) -> BcgEdge {
            BcgEdge {
                source: self.inner.bcg_edge_buffer.bcg_p as usize,
                label: self.inner.bcg_edge_buffer.bcg_l as usize,
                target: self.inner.bcg_edge_buffer.bcg_n as usize,
            }
        }

        /// Advance the underlying C iterator for the next call, `BCG_OT_NEXT`.
        unsafe fn next(&mut self) {
            unsafe {
                BCG_OT_NEXT(self.inner.as_mut().get_unchecked_mut());
            }
        }
    }

    impl Drop for BcgOtIterator {
        fn drop(&mut self) {
            unsafe {
                // The same as BCG_OT_END_ITERATE.
                BCG_OT_STOP(self.inner.as_mut().get_unchecked_mut());
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use std::env::temp_dir;
        use std::path::Path;

        use merc_utilities::random_test;

        use crate::LTS;
        use crate::random_lts_monolithic;
        use crate::read_bcg;
        use crate::write_bcg;

        #[test]
        fn test_read_bcg() {
            // Test reading a BCG file.
            let lts = read_bcg(Path::new("../../examples/lts/abp.bcg"), Vec::new()).unwrap();

            assert_eq!(lts.num_of_states(), 74);
            assert_eq!(lts.num_of_transitions(), 92);
            assert_eq!(lts.num_of_labels(), 19);
        }

        #[test]
        #[cfg_attr(miri, ignore)] // Too slow with miri
        fn test_random_bcg_io() {
            random_test(100, |rng| {
                let lts = random_lts_monolithic::<String>(rng, 100, 3, 20);

                // Use a temporary file in the target directory.
                let tmp = temp_dir();

                let file = tmp.join("test_random_bcg_io.bcg");
                write_bcg(&lts, &file).unwrap();   

                let result_lts = read_bcg(&file, Vec::new()).unwrap();

                crate::check_equivalent(&lts, &result_lts);
            });
        }
    }
}

pub use inner::read_bcg;
pub use inner::write_bcg;
