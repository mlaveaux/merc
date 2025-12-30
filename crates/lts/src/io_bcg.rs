//! Suppress various warnings from the generated bindings.

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(unused)]

use std::path::Path;

use merc_utilities::MercError;

use crate::LabelledTransitionSystem;
use crate::LTS;

#[cfg(not(feature = "merc_bcg_format"))]
mod inner {
    use super::*;

    /// This is a stub implementation used when BCG support is not compiled in.
    pub fn read_bcg(_path: &Path, _hidden_labels: Vec<String>) -> Result<LabelledTransitionSystem<String>, MercError> {
        Err("BCG format support not compiled in, see the 'merc_bcg_format' feature.".into())
    }

    /// This is a stub implementation used when BCG support is not compiled in.
    pub fn write_bcg(_lts: &impl LTS, _path: &Path) -> Result<(), MercError> {
        Err("BCG format support not compiled in, see the 'merc_bcg_format' feature.".into())
    }
}

#[cfg(feature = "merc_bcg_format")]
mod inner {
    use log::info;
    use merc_io::TimeProgress;

    use super::*;

    use core::num;
    use std::env;
    use std::ffi::CStr;
    use std::ffi::CString;
    use std::sync::Mutex;
    use std::sync::Once;

    use crate::LtsBuilder;
    use crate::StateIndex;

    /// Initialize the BCG library exactly once.
    static BCG_INITIALIZED: Once = Once::new();

    /// Mutex to ensure thread-safe access to BCG library functions.
    static BCG_LOCK: Mutex<()> = Mutex::new(());

    // Include the generated bindings for the BCG C library.
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

    /// Reads a labelled transition system in the prioprietary BCG format, from the [CADP](https://cadp.inria.fr/man/bcg.html) toolset.
    ///
    /// # Details
    ///
    /// This requires the `CADP` toolset to be installed for the target platform, and the `CADP` environment variable to be set.
    ///
    /// Note that the C library can only read files from disk; reading from in-memory buffers is not supported.
    pub fn read_bcg(path: &Path, hidden_labels: Vec<String>) -> Result<LabelledTransitionSystem<String>, MercError> {
        initialize_bcg()?;
        info!("Reading LTS in BCG format...");

        // Take the lock to ensure thread-safe access to BCG functions.
        let _guard = BCG_LOCK.lock().expect("Failed to acquire BCG lock");

        let mut bcg_object: BCG_TYPE_OBJECT_TRANSITION = std::ptr::null_mut();
        unsafe {
            BCG_OT_READ_BCG_BEGIN(
                CString::new(path.to_string_lossy().as_ref())?.into_raw(),
                &mut bcg_object,
                0, // No special flags
            );
        }

        // Read the labels.
        let num_of_labels = unsafe { BCG_OT_NB_LABELS(bcg_object) };

        let mut labels = Vec::with_capacity(num_of_labels as usize);
        for i in 0..num_of_labels {
            let labe = unsafe { BCG_OT_LABEL_STRING(bcg_object, i) };

            labels.push(unsafe { CStr::from_ptr(labe).to_string_lossy().into_owned() });
        }

        // Read the initial state.
        let initial_state = unsafe { BCG_OT_INITIAL_STATE(bcg_object) };

        let mut builder = LtsBuilder::new(labels.clone(), hidden_labels);

        // Read the transitions.
        let num_of_transitions = unsafe { BCG_OT_NB_EDGES(bcg_object) };

        // Default initialization
        let mut iterator: BCG_TYPE_OT_ITERATOR = BCG_TYPE_OT_ITERATOR {
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
        };

        unsafe {
            BCG_OT_START(&mut iterator, bcg_object, bcg_enum_edge_sort_BCG_UNDEFINED_SORT);
        };

        let mut progress = TimeProgress::new(
            move |transitions: usize| {
                info!(
                    "Read {} transitions ({}%)...",
                    transitions,
                    transitions * 100 / num_of_transitions as usize
                );
            },
            1,
        );

        while !iterator.bcg_edge_buffer.bcg_end {
            // These values are derived from the bcg_edge_sort.h file in the CADP source code, since we cannot use C macros.
            let source = iterator.bcg_edge_buffer.bcg_p;
            let label = iterator.bcg_edge_buffer.bcg_l;
            let target = iterator.bcg_edge_buffer.bcg_n;

            builder.add_transition(
                StateIndex::new(source as usize),
                &labels[label as usize],
                StateIndex::new(target as usize),
            );

            progress.print(builder.num_of_transitions());
            unsafe {
                BCG_OT_NEXT(&mut iterator);
            }
        }

        let lts = builder.finish(StateIndex::new(initial_state as usize));

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

        // Take the lock to ensure thread-safe access to BCG functions.
        let _guard = BCG_LOCK.lock().expect("Failed to acquire BCG lock");

        unsafe {
            // Equal to 2 if, in the forthcoming successive invocations of
            // function BCG_IO_WRITE_BCG_EDGE(), the sequence of actual values
            // given to the state1 argument of BCG_IO_WRITE_BCG_EDGE() will
            // increase monotonically
            BCG_IO_WRITE_BCG_BEGIN(
                CString::new(path.to_string_lossy().as_ref())?.into_raw(),
                lts.initial_state_index().value() as u64,
                2,
                CString::new("created by merc_lts")?.into_raw(),
                false,
            );
        }

        let num_of_transitions = lts.num_of_transitions();
        let mut progress = TimeProgress::new(
            move |transitions: usize| {
                info!(
                    "Wrote {} transitions ({}%)...",
                    transitions,
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
            }
        }

        unsafe {
            BCG_IO_WRITE_BCG_END();
        }

        unimplemented!()
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
                )
            }
        }

        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use std::path::Path;

        use crate::{read_bcg, LTS};

        #[test]
        fn test_read_bcg() {
            // Test reading a BCG file.
            let lts = read_bcg(Path::new("../../examples/lts/vasy_18_73.bcg"), Vec::new()).unwrap();

            assert_eq!(lts.num_of_states(), 2966);
            assert_eq!(lts.num_of_transitions(), 7393);
            assert_eq!(lts.num_of_labels(), 6);
        }
    }
}

pub use inner::read_bcg;
pub use inner::write_bcg;
