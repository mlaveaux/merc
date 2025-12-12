#[cxx::bridge(namespace = "atermpp")]
pub mod ffi {
    unsafe extern "C++" {
        include!("mcrl2-sys/cpp/atermpp.h");
        include!("mcrl2-sys/cpp/exception.h");

        type aterm;

        #[namespace = "atermpp::detail"]
        type _aterm;

        #[namespace = "atermpp"]
        type term_mark_stack;

        /// Returns the `index` argument of the term.
        fn mcrl2_aterm_argument(input: &aterm, index: usize) -> UniquePtr<aterm>;

        /// Clones the given aterm.
        fn mcrl2_aterm_clone(input: &aterm) -> UniquePtr<aterm>;

        /// Compares two aterms for equality.
        fn mcrl2_aterm_are_equal(left: &aterm, right: &aterm) -> bool;

        /// Converts the given aterm to a string.
        fn mcrl2_aterm_to_string(input: &aterm) -> String;

        fn mcrl2_aterm_string_to_string(input: &aterm) -> String;

        /// Returns the size of the aterm list.
        fn mcrl2_aterm_list_front(input: &aterm) -> UniquePtr<aterm>;

        fn mcrl2_aterm_list_tail(input: &aterm) -> UniquePtr<aterm>;

        fn mcrl2_aterm_list_is_empty(input: &aterm) -> bool;

        /// Locks and unlocks the global aterm pool for shared access.
        fn mcrl2_lock_shared();

        /// Returns true iff the unlock was successful, otherwise the recursive count was non-zero.
        fn mcrl2_unlock_shared() -> bool;

        /// Locks and unlocks the global aterm pool for exclusive access.
        fn mcrl2_lock_exclusive();
        fn mcrl2_unlock_exclusive();
    }
}
