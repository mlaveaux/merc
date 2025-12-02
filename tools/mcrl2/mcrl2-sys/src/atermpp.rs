#[cxx::bridge(namespace = "atermpp")]
pub mod ffi {
    unsafe extern "C++" {
        include!("mcrl2-sys/cpp/atermpp.h");
        include!("mcrl2-sys/cpp/exception.h");

        type aterm;

        /// Returns the size of the aterm list.
        fn mcrl2_aterm_list_size(input: &aterm) -> usize;

        /// Returns the `index` argument of the term.
        fn mcrl2_aterm_argument(input: &aterm, index: usize) -> UniquePtr<aterm>;
    }
}
