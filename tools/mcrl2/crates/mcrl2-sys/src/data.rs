#[cxx::bridge(namespace = "mcrl2::data")]
pub mod ffi {
    unsafe extern "C++" {
        include!("mcrl2-sys/cpp/data.h");
        include!("mcrl2-sys/cpp/exception.h");

        /// Returns the variable in string form.
        fn mcrl2_variable_to_string(input: &aterm) -> String;

        fn mcrl2_variable_name(input: &aterm) -> UniquePtr<aterm>;

        fn mcrl2_variable_sort(input: &aterm) -> UniquePtr<aterm>;

        fn mcrl2_data_expression_to_string(input: &aterm) -> String;

        fn mcrl2_sort_to_string(input: &aterm) -> String;

        #[namespace = "atermpp"]
        type aterm = crate::atermpp::ffi::aterm;

        type data_specification;

        /// Creates a data specification from the given string.
        fn mcrl2_data_specification_from_string(input: &str) -> UniquePtr<data_specification>;

        #[namespace = "mcrl2::data::detail"]
        type RewriterJitty;

        #[cfg(feature = "mcrl2_jittyc")]
        #[namespace = "mcrl2::data::detail"]
        type RewriterCompilingJitty;

        /// Creates a jitty rewriter from the given data specification.
        fn mcrl2_create_rewriter_jitty(data_spec: &data_specification) -> UniquePtr<RewriterJitty>;

        /// Creates a compiling rewriter from the given data specification.
        #[cfg(feature = "mcrl2_jittyc")]
        fn mcrl2_create_rewriter_jittyc(data_spec: &data_specification) -> UniquePtr<RewriterCompilingJitty>;

        // Recognizers for the various variants of data expressions.
        fn mcrl2_data_expression_is_variable(input: &aterm) -> bool;
        fn mcrl2_data_expression_is_application(input: &aterm) -> bool;
        fn mcrl2_data_expression_is_abstraction(input: &aterm) -> bool;
        fn mcrl2_data_expression_is_function_symbol(input: &aterm) -> bool;
        fn mcrl2_data_expression_is_where_clause(input: &aterm) -> bool;
        fn mcrl2_data_expression_is_machine_number(input: &aterm) -> bool;
        fn mcrl2_data_expression_is_untyped_identifier(input: &aterm) -> bool;
        fn mcrl2_data_expression_is_data_expression(input: &aterm) -> bool;
    }
}
