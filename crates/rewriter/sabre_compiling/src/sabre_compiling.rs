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
                PathBuf::from(path).join("./sabre_ffi").to_string_lossy()
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
