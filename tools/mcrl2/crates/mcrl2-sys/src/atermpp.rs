#[cxx::bridge(namespace = "atermpp")]
pub mod ffi {
    unsafe extern "C++" {
        include!("mcrl2-sys/cpp/atermpp.h");
        include!("mcrl2-sys/cpp/exception.h");

        type aterm;
        type function_symbol;
        type term_mark_stack;
        type tls_callback_container;

        #[namespace = "atermpp::detail"]
        type _aterm;
        #[namespace = "atermpp::detail"]
        type _function_symbol;

        // Functions for managing the aterm pool

        /// Enable automated garbage collection.
        ///
        /// # Warning
        /// This will deadlock when any Rust terms are created due to the
        /// interaction with the busy flags. Instead, call collect_garbage
        /// periodically to trigger garbage collection when needed.
        fn mcrl2_aterm_pool_enable_automatic_garbage_collection(enabled: bool);

        /// Returns the number of terms in the pool.
        fn mcrl2_aterm_pool_size() -> usize;

        /// Returns the capacity of the pool, for terms of all arities so this is slightly misleading.
        fn mcrl2_aterm_pool_capacity() -> usize;

        /// Trigger garbage collection.
        fn mcrl2_aterm_pool_collect_garbage();

        /// Triggers a garbage collection when internal heuristics have determined it to be necessasry.
        fn mcrl2_aterm_pool_test_garbage_collection();

        /// Locks and unlocks the global aterm pool for shared access.
        fn mcrl2_aterm_pool_lock_shared();

        /// Returns true iff the unlock was successful, otherwise the recursive count was non-zero.
        fn mcrl2_aterm_pool_unlock_shared() -> bool;

        /// Locks the global aterm pool for exclusive access.
        fn mcrl2_aterm_pool_lock_exclusive();

        /// Unlocks exclusive access to the global aterm pool.
        fn mcrl2_aterm_pool_unlock_exclusive();

        /// Register a function to be called during marking of the garbage
        /// collection
        ///
        /// Note that the resulting pointer can never be destroyed since the
        /// order of destruction for thread-local storage is not guaranteed.
        fn mcrl2_aterm_pool_register_mark_callback(
            callback_mark: fn(Pin<&mut term_mark_stack>) -> (),
            callback_size: fn() -> usize,
        ) -> UniquePtr<tls_callback_container>;

        /// Prints various metrics that are being tracked for terms.
        fn mcrl2_aterm_pool_print_metrics();

        // Functions for managing aterms

        /// Creates a term from the given function and arguments, must be
        /// protected before the busy flags are set to false.
        ///
        /// # Safety
        /// The function symbol and arguments will not be modified unless
        /// garbage collection marks the terms, which is done atomically.
        unsafe fn mcrl2_aterm_create(function: &_function_symbol, arguments: &[*const _aterm]) -> *const _aterm;

        /// Parses the given string and returns an aterm
        fn mcrl2_aterm_from_string(text: &str) -> Result<UniquePtr<aterm>>;

        /// Returns the pointer underlying the given term.
        fn mcrl2_aterm_get_address(term: &aterm) -> *const _aterm;

        /// Marks the aterm to prevent garbage collection.
        fn mcrl2_aterm_mark_address(term: &_aterm, todo: Pin<&mut term_mark_stack>);

        /// Returns true iff the term is an aterm_list.
        fn mcrl2_aterm_is_list(term: &_aterm) -> bool;

        /// Returns true iff the term is the empty aterm_list.
        fn mcrl2_aterm_is_empty_list(term: &_aterm) -> bool;

        /// Returns true iff the term is an aterm_int.
        fn mcrl2_aterm_is_int(term: &_aterm) -> bool;

        /// Converts an aterm to a string.
        fn mcrl2_aterm_print(term: &_aterm) -> String;

        /// Returns the ith argument of this term.
        fn mcrl2_aterm_get_argument(term: &_aterm, index: usize) -> *const _aterm;

        /// Returns the function symbol of an aterm.
        fn mcrl2_aterm_get_function_symbol(term: &_aterm) -> *const _function_symbol;

        // Functions for managing function symbols

        /// Creates a function symbol with the given name and arity, increases the reference counter by one.
        fn mcrl2_function_symbol_create(name: String, arity: usize) -> *const _function_symbol;

        /// Protects the given function symbol by incrementing the reference counter.
        fn mcrl2_function_symbol_protect(symbol: &_function_symbol);

        /// Decreases the reference counter of the function symbol by one.
        fn mcrl2_function_symbol_drop(symbol: &_function_symbol);

        /// Returns the function symbol name
        fn mcrl2_function_symbol_get_name<'a>(symbol: &_function_symbol) -> &'a str;

        /// Returns the function symbol arity
        fn mcrl2_function_symbol_get_arity(symbol: &_function_symbol) -> usize;

        /// Obtain the address of the given function symbol.
        fn mcrl2_function_symbol_get_address(symbol: &function_symbol) -> *const _function_symbol;

        // These functions are used to test whether the definitions used in the mCRL2 toolset are the same
        // as our FFI. It is inconvenient to have accessor function for all terms, i.e., head and tail for
        // lists. So instead we simply obtain the arg(0) and arg(1) directly in Rust. However, to ensure that
        // our assumptions are correct, we provide these functions to compare the results.
    }
}
