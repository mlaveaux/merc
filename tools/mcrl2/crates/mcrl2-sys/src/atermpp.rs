#[cxx::bridge(namespace = "atermpp")]
pub mod ffi {
    unsafe extern "C++" {
        include!("mcrl2-sys/cpp/atermpp.h");
        include!("mcrl2-sys/cpp/exception.h");

        type aterm;
        type term_mark_stack;
        type function_symbol;

        #[namespace = "atermpp::detail"]
        type _aterm;
        #[namespace = "atermpp::detail"]
        type _function_symbol;
        
        /// Enable automated garbage collection.
        ///
        /// # Warning
        /// This will deadlock when any Rust terms are created due to the
        /// interaction with the busy flags. Instead, call collect_garbage
        /// periodically to trigger garbage collection when needed.
        fn enable_automatic_garbage_collection(enabled: bool);

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
        
        /// Returns the function symbol name
        unsafe fn mcrl2_function_symbol_name<'a>(symbol: *const _function_symbol) -> &'a str;

        /// Returns the function symbol arity
        unsafe fn mcrl2_function_symbol_arity(symbol: *const _function_symbol) -> usize;

        /// Protects the given function symbol by incrementing the reference counter.
        unsafe fn mcrl2_protect_function_symbol(symbol: *const _function_symbol);

        /// Decreases the reference counter of the function symbol by one.
        unsafe fn mcrl2_drop_function_symbol(symbol: *const _function_symbol);
    }
}
