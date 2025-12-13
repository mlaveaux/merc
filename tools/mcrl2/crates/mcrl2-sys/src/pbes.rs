#[cxx::bridge(namespace = "mcrl2::pbes_system")]
pub mod ffi {
    /// A helper struct for std::pair<const local_control_flow_graph_vertex*, UniquePtr<CxxVector<usize>>>
    struct vertex_outgoing_edge {
        vertex: *const local_control_flow_graph_vertex,
        edges: UniquePtr<CxxVector<usize>>,
    }

    /// A helper struct for std::pair<pbes_expression, pbes_expression>>
    struct assignment_pair {
        pub lhs: *const aterm,
        pub rhs: *const aterm,
    }

    unsafe extern "C++" {
        include!("mcrl2-sys/cpp/pbes.h");
        include!("mcrl2-sys/cpp/exception.h");

        type pbes;

        type srf_summand;

        /// Loads a PBES from a file.
        fn mcrl2_load_pbes_from_pbes_file(filename: &str) -> Result<UniquePtr<pbes>>;

        fn mcrl2_load_pbes_from_text_file(filename: &str) -> Result<UniquePtr<pbes>>;

        /// Loads a PBES from a string.
        fn mcrl2_load_pbes_from_text(input: &str) -> Result<UniquePtr<pbes>>;

        #[namespace = "mcrl2::data"]
        type data_specification = crate::data::ffi::data_specification;

        fn mcrl2_pbes_data_specification(input: &pbes) -> UniquePtr<data_specification>;

        type stategraph_algorithm;

        /// Run the state graph algorithm and obtain the result.
        fn mcrl2_stategraph_local_algorithm_run(input: &pbes) -> Result<UniquePtr<stategraph_algorithm>>;

        #[namespace = "mcrl2::pbes_system::detail"]
        type local_control_flow_graph;

        /// Get the control flow graphs identified by the state graph algorithm.
        fn mcrl2_stategraph_local_algorithm_cfgs(
            result: Pin<&mut CxxVector<local_control_flow_graph>>,
            input: &stategraph_algorithm,
        );

        #[namespace = "mcrl2::pbes_system::detail"]
        type stategraph_equation;

        fn mcrl2_stategraph_local_algorithm_equations(
            result: Pin<&mut CxxVector<stategraph_equation>>,
            input: &stategraph_algorithm,
        );

        #[namespace = "mcrl2::pbes_system::detail"]
        type predicate_variable;

        /// Returns the predicate variables of a stategraph equation.
        fn mcrl2_stategraph_equation_predicate_variables(
            result: Pin<&mut CxxVector<predicate_variable>>,
            input: &stategraph_equation,
        );

        /// Returns the propositional variable of a pbes equation
        fn mcrl2_stategraph_equation_variable(equation: &stategraph_equation) -> UniquePtr<aterm>;

        /// Returns the used set of a predicate variable.
        fn mcrl2_predicate_variable_used(input: &predicate_variable) -> Vec<usize>;

        /// Returns the changed set of a predicate variable.
        fn mcrl2_predicate_variable_changed(input: &predicate_variable) -> Vec<usize>;

        #[namespace = "mcrl2::pbes_system::detail"]
        type local_control_flow_graph_vertex;

        /// Obtain the vertices of a cfg.
        fn mcrl2_local_control_flow_graph_vertices(
            result: Pin<&mut CxxVector<local_control_flow_graph_vertex>>,
            input: &local_control_flow_graph,
        );

        /// Obtain the index of the variable associated with the vertex.
        fn mcrl2_local_control_flow_graph_vertex_index(vertex: &local_control_flow_graph_vertex) -> usize;

        /// Obtain the name of the variable associated with the vertex.
        fn mcrl2_local_control_flow_graph_vertex_name(vertex: &local_control_flow_graph_vertex) -> UniquePtr<aterm>;

        /// Obtain the value of the variable associated with the vertex.
        fn mcrl2_local_control_flow_graph_vertex_value(vertex: &local_control_flow_graph_vertex) -> UniquePtr<aterm>;

        /// Obtain the outgoing edges of the vertex.
        fn mcrl2_local_control_flow_graph_vertex_outgoing_edges(
            result: Pin<&mut CxxVector<vertex_outgoing_edge>>,
            input: &local_control_flow_graph_vertex,
        );

        /// Obtain the outgoing edges of the vertex.
        fn mcrl2_local_control_flow_graph_vertex_incoming_edges(
            result: Pin<&mut CxxVector<vertex_outgoing_edge>>,
            input: &local_control_flow_graph_vertex,
        );

        type srf_pbes;

        type srf_equation;

        /// Convert a PBES to an SRF PBES.
        fn mcrl2_pbes_to_srf_pbes(input: &pbes) -> Result<UniquePtr<srf_pbes>>;

        /// Returns PBES as a string.
        fn mcrl2_pbes_to_string(input: &pbes) -> String;

        /// Convert a SRF PBES to a PBES.
        fn mcrl2_srf_pbes_to_pbes(input: &srf_pbes) -> UniquePtr<pbes>;

        /// Unify all parameters of the equations, optionally ignoring the equations
        /// related to counter example information. Finally, if reset is true, reset the
        /// newly introduced parameters to a default value.
        fn mcrl2_srf_pbes_unify_parameters(input: Pin<&mut srf_pbes>, ignore_ce_equations: bool, reset: bool);

        /// Returns the summands of the given srf_equation.
        fn mcrl2_srf_equations_summands(result: Pin<&mut CxxVector<srf_summand>>, input: &srf_equation);

        #[namespace = "atermpp"]
        type aterm = crate::atermpp::ffi::aterm;

        /// Returns the equations of the given srf_pbes.
        fn mcrl2_srf_pbes_equations(result: Pin<&mut CxxVector<srf_equation>>, input: &srf_pbes);

        /// Returns the variable of the given srf_equation.
        fn mcrl2_srf_pbes_equation_variable(input: &srf_equation) -> UniquePtr<aterm>;

        fn mcrl2_propositional_variable_name(input: &aterm) -> UniquePtr<aterm>;

        /// Returns an aterm_list<variable>
        fn mcrl2_propositional_variable_parameters(input: &aterm) -> UniquePtr<aterm>;

        fn mcrl2_propositional_variable_to_string(input: &aterm) -> String;

        fn mcrl2_propositional_variable_is(input: &aterm) -> bool;

        fn mcrl2_srf_summand_condition(summand: &srf_summand) -> UniquePtr<aterm>;

        fn mcrl2_srf_summand_variable(summand: &srf_summand) -> UniquePtr<aterm>;

        /// Replace data variables in a pbes expression according to the given substitutions.
        fn mcrl2_pbes_expression_replace_variables(
            expression: &aterm,
            substitutions: &Vec<assignment_pair>,
        ) -> UniquePtr<aterm>;

        /// Replace propositional variables in a pbes expression according to the given substitutions.
        fn mcrl2_pbes_expression_replace_propositional_variables(
            expression: &aterm,
            pi: &Vec<usize>,
        ) -> UniquePtr<aterm>;

        fn mcrl2_pbes_expression_to_string(expression: &aterm) -> String;
    }
}
