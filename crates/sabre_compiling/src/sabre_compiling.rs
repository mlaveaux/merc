use std::ffi::c_void;
use std::path::Path;
use std::path::PathBuf;

use libloading::Library;
use libloading::Symbol;
use log::info;
use tempfile::TempDir;
use tempfile::tempdir;
use toml::Table;

use merc_data::DataExpression;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;
use merc_sabre_ffi::DataExpressionFFI;
use merc_sabre_ffi::DataExpressionRefFFI;
use merc_sabre_ffi::SabreRewriteVTable;
use merc_sabre_ffi::data_expression_ref_from_term;
use merc_sabre_ffi::into_data_expression;
use merc_sabre_ffi::rewrite_vtable;
use merc_utilities::MercError;

use crate::generate;
use crate::library::RuntimeLibrary;

pub struct SabreCompilingRewriter {
    /// Cached `rewrite` entry point of `library`. Resolved once in `new`; valid
    /// for as long as `library` stays loaded.
    rewrite_fn: extern "C-unwind" fn(&DataExpressionRefFFI<'_>) -> DataExpressionFFI,
    /// Keeps every term whose raw address is baked into the generated
    /// library protected.
    _spec: RewriteSpecification,
    /// The loaded library, keeps rewrite_fn alive.
    _library: Library,
    /// Keep the temporary directory alive so it is not removed while the
    /// library is still mapped. `None` when a caller-managed local directory is
    /// used instead.
    _temp_dir: Option<TempDir>,
}

impl RewriteEngine for SabreCompilingRewriter {
    fn rewrite(&mut self, term: &DataExpression) -> DataExpression {
        let result = (self.rewrite_fn)(&data_expression_ref_from_term(term));

        // SAFETY: `result` is an owned handle produced by the generated
        // `rewrite` entry point (always via the host `create`/`protect`), so it
        // wraps a live `Box<DataExpression>`.
        unsafe { into_data_expression(result) }
    }
}

impl SabreCompilingRewriter {
    /// Compiles a rewriter for `spec` into a fresh dynamic library and loads
    /// it into the current process.
    ///
    /// `use_local_workspace` links the generated crate against this
    /// repository's `sabre_ffi` by path instead of a pinned git revision, for
    /// local development. `use_local_tmp` builds under `./tmp` instead of a
    /// fresh system temporary directory, so the generated source survives for
    /// inspection.
    ///
    /// # Errors
    ///
    /// Returns an error if the generated crate fails to compile, or if the
    /// resulting library is missing the `initialise`/`rewrite` symbols
    /// emitted by the code generator.
    pub fn new(
        spec: &RewriteSpecification,
        use_local_workspace: bool,
        use_local_tmp: bool,
    ) -> Result<SabreCompilingRewriter, MercError> {
        // Only allocate a system temporary directory when one is needed; the
        // local-tmp path uses a fixed `./tmp` directory instead.
        let system_tmp_dir = if use_local_tmp { None } else { Some(tempdir()?) };
        let temp_dir = match &system_tmp_dir {
            Some(dir) => dir.path(),
            None => Path::new("./tmp"),
        };

        let compilation_toml = include_str!(concat!(env!("OUT_DIR"), "/Compilation.toml")).parse::<Table>()?;
        let sabrec = compilation_toml.get("sabrec").ok_or("Missing [sabrec] section")?;

        let mut dependencies = vec![];

        if use_local_workspace {
            let path = sabrec
                .get("path")
                .ok_or("Missing path entry")?
                .as_str()
                .ok_or("Not a string")?;

            info!("Using local dependency {path}");
            dependencies.push(format!(
                "merc_sabre-ffi = {{ path = '{}' }}",
                PathBuf::from(path)
                    .join("../../crates/sabre_compiling/sabre_ffi")
                    .to_string_lossy()
            ));
        } else {
            // Pin to the host's git commit so the loaded library's `#[repr(C)]`
            // vtable layout matches the host exactly.
            let repository = "https://github.com/MERCorg/merc.git";
            let commit = sabrec.get("commit").and_then(|c| c.as_str()).unwrap_or_default();

            if commit.is_empty() {
                info!("Using git dependency {repository} (unpinned; no commit recorded at build time)");
                dependencies.push(format!("merc_sabre-ffi = {{ git = '{repository}' }}"));
            } else {
                info!("Using git dependency {repository} pinned to {commit}");
                dependencies.push(format!("merc_sabre-ffi = {{ git = '{repository}', rev = '{commit}' }}"));
            }
        }

        let mut compilation_crate = RuntimeLibrary::new(temp_dir, dependencies)?;

        // Write the output source file(s).
        generate(spec, compilation_crate.source_dir())?;

        let library = compilation_crate.compile()?;

        // Install the host vtable into the loaded library exactly once and cache
        // the `rewrite` entry point. All term-pool access in the library is
        // routed back through the vtable, so the library never touches its own
        // (duplicated) term pool.
        //
        // SAFETY: `initialise` and `rewrite` have the signatures emitted by the
        // code generator. The vtable's function pointers live in the host binary
        // for the whole process, satisfying the contract of `set_rewrite_vtable`.
        let rewrite_fn = unsafe {
            let initialise: Symbol<extern "C-unwind" fn(*mut c_void)> = library.get(b"initialise")?;
            let vtable: SabreRewriteVTable = rewrite_vtable();
            initialise(std::ptr::addr_of!(vtable) as *mut c_void);

            let rewrite: Symbol<extern "C-unwind" fn(&DataExpressionRefFFI<'_>) -> DataExpressionFFI> =
                library.get(b"rewrite")?;
            *rewrite
        };

        Ok(SabreCompilingRewriter {
            rewrite_fn,
            _spec: spec.clone(),
            _library: library,
            _temp_dir: system_tmp_dir,
        })
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use merc_data::to_untyped_data_expression;
    use merc_rec_tests::load_rec_from_strings;
    use merc_sabre::RewriteEngine;

    use super::SabreCompilingRewriter;

    /// `innermost_codegen.rs` bakes the *raw pool address* of every symbol and
    /// machine-number literal reachable from a rule's right-hand side directly
    /// into the generated `lib.rs` (`DataExpressionRefFFI::from_ptr(<addr>)`).
    /// Those addresses are read once, at codegen time; nothing re-validates them
    /// when the compiled function runs later. The only thing standing between
    /// that and a dangling pointer is `SabreCompilingRewriter::_spec`, which is
    /// documented to keep every such term rooted for as long as the rewriter
    /// lives (`sabre_compiling.rs:30`-`32`).
    ///
    /// This test is the executable check of that invariant: it forces garbage
    /// collection, including collections triggered by ordinary allocation
    /// pressure from unrelated terms, both before and *between* calls into the
    /// compiled library, and confirms the compiled `rewrite` entry point still
    /// produces the correct result afterwards. If `_spec` (or the reachability
    /// argument behind it, see the comment on `TermStack::from_term`'s
    /// `is_data_machine_number` branch) ever stopped covering one of the
    /// embedded addresses, this is the kind of test that would start segfaulting
    /// or silently returning a term rebuilt from a reused, unrelated pool slot.
    #[test]
    #[cfg_attr(miri, ignore)] // Miri does not support FFI.
    fn test_sabre_compiling_survives_garbage_collection_between_calls() {
        let (spec, terms) = load_rec_from_strings(&[
            include_str!("../../../examples/REC/rec/factorial5.rec"),
            include_str!("../../../examples/REC/rec/factorial.rec"),
        ])
        .unwrap();

        let spec = spec.to_rewrite_spec();
        let mut rewriter = SabreCompilingRewriter::new(&spec, true, true).unwrap();

        // Force collection before the compiled library is used at all: every
        // address it embeds must already be reachable through `_spec` at this
        // point, not just transiently alive from the (now-dropped) codegen call.
        merc_aterm::storage::THREAD_TERM_POOL.with(|tp| {
            tp.force_collect_garbage();
            tp.force_collect_garbage();
        });

        for t in terms {
            let data_term = to_untyped_data_expression(t, None);

            // Generate unrelated allocation pressure and force another collection
            // between every call into the generated library, so a GC can run
            // while the compiled `rewrite`/`match_*` functions are on the stack
            // across separate top-level calls.
            for i in 0..64u64 {
                let _garbage: merc_data::DataExpression = merc_data::MachineNumber::new(i).into();
            }
            merc_aterm::storage::THREAD_TERM_POOL.with(|tp| tp.force_collect_garbage());

            let rewritten_term = rewriter.rewrite(&data_term);

            merc_aterm::storage::THREAD_TERM_POOL.with(|tp| tp.force_collect_garbage());

            assert_eq!(
                rewritten_term.to_string().chars().filter(|c| *c == 's').count(),
                120, // 5! = 120.
                "The rewritten result does not match the expected result after forced GC"
            );
        }
    }

    #[test]
    #[cfg_attr(miri, ignore)] // Miri does not support FFI.
    fn test_sabre_compiling_example() {
        let (spec, terms) = load_rec_from_strings(&[
            include_str!("../../../examples/REC/rec/factorial5.rec"),
            include_str!("../../../examples/REC/rec/factorial.rec"),
        ])
        .unwrap();

        let spec = spec.to_rewrite_spec();
        let mut rewriter = SabreCompilingRewriter::new(&spec, true, true).unwrap();

        for t in terms {
            let data_term = to_untyped_data_expression(t, None);
            let rewritten_term = rewriter.rewrite(&data_term);

            println!("Original term: {data_term}");
            println!("Rewritten term: {rewritten_term}");

            assert_eq!(
                rewritten_term.to_string().chars().filter(|c| *c == 's').count(),
                120, // 5! = 120.
                "The rewritten result does not match the expected result"
            );
        }
    }
}
