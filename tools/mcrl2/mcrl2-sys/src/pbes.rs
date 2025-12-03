#[cxx::bridge(namespace = "mcrl2::pbes_system")]
pub mod ffi {

    unsafe extern "C++" {
        include!("mcrl2-sys/cpp/pbes.h");
        include!("mcrl2-sys/cpp/exception.h");

        type pbes;

        /// Loads a PBES from a file.
        fn mcrl2_load_pbes_from_file(filename: &str) -> Result<UniquePtr<pbes>>;

        type stategraph_algorithm;

        /// Run the state graph algorithm and obtain the result.
        fn mcrl2_pbes_stategraph_local_algorithm_run(input: &pbes) -> Result<UniquePtr<stategraph_algorithm>>;

        #[namespace = "mcrl2::pbes_system::detail"]
        type local_control_flow_graph;

        /// Get the control flow graphs identified by the state graph algorithm.
        fn mcrl2_pbes_stategraph_local_algorithm_cfgs(
            result: Pin<&mut CxxVector<local_control_flow_graph>>,
            input: &stategraph_algorithm,
        ) -> Result<()>;

        #[namespace = "mcrl2::pbes_system::detail"]
        type local_control_flow_graph_vertex;

        /// Obtain the vertices of a cfg.
        fn mcrl2_pbes_local_control_flow_graph_vertices(
            result: Pin<&mut CxxVector<local_control_flow_graph_vertex>>,
            input: &local_control_flow_graph,
        ) -> Result<()>;

        type srf_pbes;

        type srf_equation;

        /// Convert a PBES to an SRF PBES.
        fn mcrl2_pbes_to_srf_pbes(input: &pbes) -> Result<UniquePtr<srf_pbes>>;

        /// Returns PBES as a string.
        fn mcrl2_pbes_to_string(input: &pbes) -> Result<String>;

        /// Convert a SRF PBES to a PBES.
        fn mcrl2_srf_pbes_to_pbes(input: &srf_pbes) -> Result<UniquePtr<pbes>>;

        /// Unify all parameters of the equations, optionally ignoring the equations
        /// related to counter example information. Finally, if reset is true, reset the
        /// newly introduced parameters to a default value.
        fn mcrl2_unify_parameters(input: Pin<&mut srf_pbes>, ignore_ce_equations: bool, reset: bool) -> Result<()>;

        #[namespace = "atermpp"]
        type aterm = crate::atermpp::ffi::aterm;

        type propositional_variable;

        /// Returns the equations of the given srf_pbes.
        fn mcrl2_srf_pbes_equations(result: Pin<&mut CxxVector<srf_equation>>, input: &srf_pbes) -> Result<()>;

        /// Returns the variable of the given srf_equation.
        unsafe fn mcrl2_srf_pbes_equation_variable(
            input: *const srf_equation,
        ) -> Result<UniquePtr<propositional_variable>>;

        #[namespace = "mcrl2::data"]
        type variable = crate::data::ffi::variable;

        /// Returns an aterm_list<variable>
        fn mcrl2_propositional_variable_parameters(input: &propositional_variable) -> Result<UniquePtr<aterm>>;

        fn mcrl2_propositional_variable_to_string(input: &propositional_variable) -> Result<String>;
    }
}
