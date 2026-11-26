//! Bidirectional conversion between the pure-Rust `merc_aterm` term pool and
//! the mCRL2 C++ `atermpp` term pool.
//!
//! Both pools represent first-order terms with the same abstract shape
//! (symbol name + arity + ordered arguments, plus a special integer term), but
//! they live in completely separate allocators and have different protection /
//! GC mechanisms.  These functions perform a structural copy — the result is a
//! maximally-shared term in the *target* pool with the same tree shape as the
//! source term.
//!
//! # Stack safety
//!
//! Both directions use an explicit work stack rather than recursion to avoid
//! blowing the system stack on deeply nested terms (e.g. large number
//! literals).

use mcrl2_sys::atermpp::ffi::mcrl2_aterm_int_value;

use super::THREAD_TERM_POOL as MCRL2_POOL;
use crate::ATerm as Mcrl2ATerm;
use crate::ATermRef as Mcrl2ATermRef;

/// Converts a pure-Rust [`merc_aterm::ATerm`] into a maximally-shared term in
/// the mCRL2 C++ aterm pool.
///
/// Integer terms are mapped to `aterm_int`; all other terms are mapped
/// structurally by (name, arity, arguments).
pub fn merc_aterm_to_mcrl2(root: &merc_aterm::ATermRef<'_>) -> Mcrl2ATerm {
    use merc_aterm::ATermIntRef;
    use merc_aterm::Symb as MercSymb;
    use merc_aterm::Term as MercTerm;
    use merc_aterm::is_int_term;

    enum Task {
        /// A merc term that needs to be converted.
        Process(merc_aterm::ATerm),
        /// After all `arity` arguments have been pushed to `result_stack`,
        /// create a C++ term with this symbol.
        Assemble(String, usize),
    }

    let mut todo: Vec<Task> = vec![Task::Process(root.protect())];
    let mut result_stack: Vec<Mcrl2ATerm> = Vec::new();

    while let Some(task) = todo.pop() {
        match task {
            Task::Process(term) => {
                if is_int_term(&term) {
                    let value = ATermIntRef::from(term.copy()).value() as u64;
                    let mcrl2_int = MCRL2_POOL.with_borrow(|tp| tp.create_int(value));
                    result_stack.push(mcrl2_int);
                } else {
                    let sym = MercTerm::get_head_symbol(&term);
                    let name = MercSymb::name(&sym).to_owned();
                    let arity = MercSymb::arity(&sym);

                    // Push the assembly task first so it fires *after* all args are done.
                    todo.push(Task::Assemble(name, arity));

                    // Push args right-to-left so the leftmost arg is popped (and
                    // converted) first, landing at the correct position in result_stack.
                    for i in (0..arity).rev() {
                        todo.push(Task::Process(MercTerm::arg(&term, i).protect()));
                    }
                }
            }
            Task::Assemble(name, arity) => {
                let start = result_stack.len() - arity;
                let args: Vec<Mcrl2ATerm> = result_stack.drain(start..).collect();

                let mcrl2_term = MCRL2_POOL.with_borrow(|tp| {
                    let sym = tp.create_symbol(&name, arity);
                    let arg_refs: Vec<Mcrl2ATermRef<'_>> = args.iter().map(|a| a.copy()).collect();
                    tp.create(&sym.copy(), &arg_refs)
                });
                result_stack.push(mcrl2_term);
            }
        }
    }

    debug_assert_eq!(result_stack.len(), 1, "conversion must yield exactly one term");
    result_stack.pop().expect("The conversion yields exactly one term")
}

/// Converts a mCRL2 C++ [`Mcrl2ATerm`] into a maximally-shared term in the
/// pure-Rust `merc_aterm` pool.
///
/// Integer terms are mapped to `merc_aterm::ATermInt`; all other terms are
/// mapped structurally by (name, arity, arguments).
pub fn mcrl2_aterm_to_merc(root: &Mcrl2ATermRef<'_>) -> merc_aterm::ATerm {
    use merc_aterm::Symbol as MercSymbol;
    use merc_aterm::storage::THREAD_TERM_POOL as MERC_POOL;

    // Every entry in `todo` is a protected `Mcrl2ATerm`; this keeps all
    // subterm pointers live regardless of whether C++ GC fires during the
    // construction of merc terms.
    enum Task {
        Process(Mcrl2ATerm),
        Assemble(String, usize),
    }

    let mut todo: Vec<Task> = vec![Task::Process(root.protect())];
    let mut result_stack: Vec<merc_aterm::ATerm> = Vec::new();

    while let Some(task) = todo.pop() {
        match task {
            Task::Process(term) => {
                if term.is_int() {
                    let value = mcrl2_aterm_int_value(term.get()) as usize;
                    let merc_int = MERC_POOL.with(|tp| tp.create_int(value));
                    result_stack.push(merc_int);
                } else {
                    let sym = term.get_head_symbol();
                    let name = sym.name().to_owned();
                    let arity = sym.arity();

                    todo.push(Task::Assemble(name, arity));

                    for i in (0..arity).rev() {
                        todo.push(Task::Process(term.arg(i).protect()));
                    }
                }
            }
            Task::Assemble(name, arity) => {
                let start = result_stack.len() - arity;
                let args: Vec<merc_aterm::ATerm> = result_stack.drain(start..).collect();

                let merc_term = MERC_POOL.with(|tp| {
                    let sym = MercSymbol::new(&name, arity);
                    tp.create_term_iter(&sym, args.iter())
                });
                result_stack.push(merc_term);
            }
        }
    }

    debug_assert_eq!(result_stack.len(), 1, "conversion must yield exactly one term");
    result_stack.pop().expect("The conversion yields exactly one term")
}

#[cfg(test)]
mod tests {
    use mcrl2_sys::atermpp::ffi::mcrl2_aterm_print;
    use merc_aterm::Term as MercTerm;

    use super::mcrl2_aterm_to_merc;
    use super::merc_aterm_to_mcrl2;
    use crate::atermpp::THREAD_TERM_POOL as MCRL2_POOL;

    /// Build a simple merc term `f(g(a), b)` and verify it round-trips through
    /// the merc → mcrl2 → merc pipeline without structural change.
    #[test]
    fn test_roundtrip_nested() {
        let a = merc_aterm::ATerm::constant(&merc_aterm::Symbol::new("a", 0));
        let b = merc_aterm::ATerm::constant(&merc_aterm::Symbol::new("b", 0));
        let g = merc_aterm::Symbol::new("g", 1);
        let ga = merc_aterm::ATerm::with_iter(&g, [MercTerm::copy(&a)]);
        let f = merc_aterm::Symbol::new("f", 2);
        let fgab = merc_aterm::ATerm::with_iter(&f, [MercTerm::copy(&ga), MercTerm::copy(&b)]);

        let as_mcrl2 = merc_aterm_to_mcrl2(&MercTerm::copy(&fgab));
        let back = mcrl2_aterm_to_merc(&as_mcrl2.copy());
        assert_eq!(format!("{fgab:?}"), format!("{back:?}"));
    }

    #[test]
    fn test_roundtrip_constant() {
        let c = merc_aterm::ATerm::constant(&merc_aterm::Symbol::new("Bool", 0));
        let as_mcrl2 = merc_aterm_to_mcrl2(&MercTerm::copy(&c));
        let back = mcrl2_aterm_to_merc(&as_mcrl2.copy());
        assert_eq!(format!("{c:?}"), format!("{back:?}"));
    }

    #[test]
    fn test_roundtrip_sort_id() {
        // SortId("Bool") — the shape merc_data::BasicSort uses.
        let bool_str = merc_aterm::ATerm::constant(&merc_aterm::Symbol::new("Bool", 0));
        let sort_id = merc_aterm::Symbol::new("SortId", 1);
        let bool_sort = merc_aterm::ATerm::with_iter(&sort_id, [MercTerm::copy(&bool_str)]);

        let as_mcrl2 = merc_aterm_to_mcrl2(&MercTerm::copy(&bool_sort));
        let back = mcrl2_aterm_to_merc(&as_mcrl2.copy());
        assert_eq!(format!("{bool_sort:?}"), format!("{back:?}"));
    }

    #[test]
    fn test_roundtrip_integer() {
        let int_term = merc_aterm::ATermInt::new(42);
        let int_aterm = merc_aterm::ATerm::from(int_term);
        let as_mcrl2 = merc_aterm_to_mcrl2(&MercTerm::copy(&int_aterm));
        // The C++ pool stores it as an aterm_int with value 42.
        assert_eq!(mcrl2_aterm_print(as_mcrl2.get()), "42");
    }

    /// Round-trips a term from the C++ aterm pool through mcrl2 → merc → mcrl2
    /// and asserts the printed form is unchanged.
    #[test]
    fn test_roundtrip_from_mcrl2() {
        let original = MCRL2_POOL
            .with_borrow(|tp| tp.create_from_string("f(g(a),b)"))
            .expect("C++ aterm parse failed");
        let as_merc = mcrl2_aterm_to_merc(&original.copy());
        let back = merc_aterm_to_mcrl2(&MercTerm::copy(&as_merc));
        assert_eq!(mcrl2_aterm_print(original.get()), mcrl2_aterm_print(back.get()));
    }
}
