#[cxx::bridge(namespace = "mcrl2::pbes_system")]
pub mod ffi {

    unsafe extern "C++" {
        include!("mcrl2-sys/cpp/pbes/pbes.h");

        type pbes;
        
        type cliques_algorithm;

        fn load_pbes_from_file(filename: &str) -> Result<UniquePtr<pbes>>;

        fn run_stategraph_local_algorithm(input: &pbes) -> Result<UniquePtr<cliques_algorithm>>;
    }
}
