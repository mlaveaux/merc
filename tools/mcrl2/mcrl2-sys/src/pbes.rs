use cxx::CxxVector;

#[cxx::bridge(namespace = "mcrl2::pbes_system")]
pub mod ffi {
    unsafe extern "C++" {
        include!("mcrl2-sys/cpp/pbes.h");

        type pbes;

        /// Loads a PBES from a file.
        fn mcrl2_load_pbes_from_file(filename: &str) -> Result<UniquePtr<pbes>>;

        type stategraph_algorithm;

        #[namespace = "mcrl2::pbes_system::detail"]
        type local_control_flow_graph;

        /// Run the state graph algorithm and obtain the result.
        fn mcrl2_pbes_stategraph_local_algorithm_run(input: &pbes) -> Result<UniquePtr<stategraph_algorithm>>;

        /// Get the control flow graphs identified by the state graph algorithm.
        fn mcrl2_pbes_stategraph_local_algorithm_cfgs_size(input: &stategraph_algorithm) -> Result<usize>;

        type srf_pbes;

        /// Convert a PBES to an SRF PBES.
        fn mcrl2_pbes_to_srf_pbes(input: &pbes) -> Result<UniquePtr<srf_pbes>>;

        /// Convert a SRF PBES to a PBES.
        fn mcrl2_srf_pbes_to_pbes(input: &srf_pbes) -> Result<UniquePtr<pbes>>;

        /// Unify all parameters of the equations, optionally ignoring the equations
        /// related to counter example information. Finally, if reset is true, reset the
        /// newly introduced parameters to a default value.
        fn mcrl2_unify_parameters(input: Pin<&mut srf_pbes>, ignore_ce_equations: bool, reset: bool) -> Result<()>;
    }
}
