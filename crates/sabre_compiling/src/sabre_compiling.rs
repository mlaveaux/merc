use std::path::Path;
use std::path::PathBuf;

use libloading::Library;
use libloading::Symbol;
use log::info;
use tempfile::tempdir;
use toml::Table;

use merc_aterm::ATermRef;
use merc_aterm::Term;
use merc_data::DataExpression;
use merc_sabre::RewriteEngine;
use merc_sabre::RewriteSpecification;
use merc_sabre_ffi::DataExpressionFFI;
use merc_sabre_ffi::DataExpressionRefFFI;
use merc_utilities::MercError;

use crate::generate;
use crate::library::RuntimeLibrary;

pub struct SabreCompilingRewriter {
    library: Library,
    //rewrite_func: Symbol<unsafe extern fn() -> u32>,
}

impl RewriteEngine for SabreCompilingRewriter {
    fn rewrite(&mut self, term: &DataExpression) -> DataExpression {
        // TODO: This ought to be stored somewhere for repeated calls.
        unsafe {
            let func: Symbol<extern "C" fn(&DataExpressionRefFFI) -> DataExpressionFFI> =
                self.library.get(b"rewrite").unwrap();

            let result = func(&DataExpressionRefFFI::from_index(term.shared()));
            ATermRef::from_index(result.index()).protect().into()
        }
    }
}

impl SabreCompilingRewriter {
    /// Creates a new compiling rewriter for the given specifications.
    ///
    /// - use_local_workspace: Use the development version of the toolset instead of referring to the github one.
    /// - use_local_tmp: Use a relative 'tmp' directory instead of using the system directory. Mostly used for debugging purposes.
    ///
    /// - [`RewriteEngine`]
    pub fn new(
        spec: &RewriteSpecification,
        use_local_workspace: bool,
        use_local_tmp: bool,
    ) -> Result<SabreCompilingRewriter, MercError> {
        let system_tmp_dir = tempdir()?;
        let temp_dir = if use_local_tmp {
            Path::new("./tmp")
        } else {
            system_tmp_dir.path()
        };

        let mut dependencies = vec![];

        if use_local_workspace {
            let compilation_toml = include_str!("../../../target/Compilation.toml").parse::<Table>()?;
            let path = compilation_toml
                .get("sabrec")
                .ok_or("Missing [sabre] section)")?
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
            info!("Using git dependency https://github.com/mlaveaux/merc.git");
            dependencies.push("merc_sabre-ffi = { git = 'https://github.com/mlaveaux/merc.git' }".to_string());
        }

        let mut compilation_crate = RuntimeLibrary::new(temp_dir, dependencies)?;

        // Write the output source file(s).
        generate(spec, compilation_crate.source_dir())?;

        let library = compilation_crate.compile()?;
        Ok(SabreCompilingRewriter { library })
    }
}

// #[cfg(test)]
// mod tests {
//     use test_log::test;

//     use merc_data::to_untyped_data_expression;
//     use merc_rec_tests::load_rec_from_strings;
//     use merc_sabre::RewriteEngine;

//     use super::SabreCompilingRewriter;

//     #[test]
//     fn test_compilation() {
//         //   plus : Nat Nat -> Nat   # addition
//         //   times : Nat Nat -> Nat  # product
//         //   fact : Nat -> Nat       # factorial
//         //   plus(d0, N) -> N
//         //   plus(s(N), M) -> s(plus(N, M))
//         //   fibb(d0) -> d0         # corrected by CONVECS
//         //   fibb(s(d0)) -> s(d0)
//         //   fibb(s(s(N))) -> plus(fibb(s(N)), fibb(N))
//         let (spec, terms) = load_rec_from_strings(&[
//             include_str!("../../../examples/REC/rec/factorial6.rec"),
//             include_str!("../../../examples/REC/rec/factorial.rec"),
//         ])
//         .unwrap();

//         let spec = spec.to_rewrite_spec();

//         let mut rewriter = SabreCompilingRewriter::new(&spec, true, true).unwrap();

//         for t in terms {
//             let data_term = to_untyped_data_expression(&t, None);
//             assert_eq!(
//                 rewriter.rewrite(&data_term),
//                 data_term,
//                 "The rewritten result does not match the expected result"
//             );
//         }
//     }
// }
