#[cxx::bridge(namespace = "mcrl2::data")]
pub mod ffi {
    unsafe extern "C++" {
        include!("mcrl2-sys/cpp/data.h");

        type variable;

        /// Returns the variable in string form.
        fn mcrl2_variable_to_string(input: &variable) -> Result<String>;

        #[namespace = "atermpp"]
        type aterm = crate::atermpp::ffi::aterm;

        /// Returns true if the given term is correct.
        fn mcrl2_data_is_variable(input: &aterm) -> bool;
    }
}
