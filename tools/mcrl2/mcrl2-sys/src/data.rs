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

        #[namespace = "atermpp"]
        type aterm = crate::atermpp::ffi::aterm;
    }
}
