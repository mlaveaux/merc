//! Small, deterministic checks of the term pool's unsafe paths — construction, shared-pointer
//! identity, argument access, protection across garbage collection, and the `Send` wrapper.
//!
//! Unlike the randomized stress tests (which are `#[cfg_attr(miri, ignore)]` because they are far
//! too slow), these are cheap enough to run under miri, so they exercise the pointer/transmute and
//! protection-set code with Stacked/Tree Borrows checking.

use std::collections::VecDeque;

use merc_aterm::ATerm;
use merc_aterm::ATermRead;
use merc_aterm::ATermRef;
use merc_aterm::ATermSend;
use merc_aterm::ATermWrite;
use merc_aterm::BinaryATermReader;
use merc_aterm::BinaryATermWriter;
use merc_aterm::ProtectedSend;
use merc_aterm::Symb;
use merc_aterm::Symbol;
use merc_aterm::SymbolRef;
use merc_aterm::Term;
use merc_aterm::Transmutable;
use merc_aterm::storage::THREAD_TERM_POOL;

/// Builds the term `f(a, g(a))` from freshly created symbols on every call.
fn build_sample() -> ATerm {
    let a = ATerm::constant(&Symbol::new("a", 0));
    let g = ATerm::with_args(&Symbol::new("g", 1), &[a.copy()]).protect();
    ATerm::with_args(&Symbol::new("f", 2), &[a.copy(), g.copy()]).protect()
}

#[test]
fn test_miri_maximal_sharing() {
    // Two structurally equal terms must resolve to the same shared node: structural equality
    // implies pointer (index) equality.
    let first = build_sample();
    let second = build_sample();
    assert_eq!(first, second);
    assert_eq!(
        first.index(),
        second.index(),
        "maximal sharing should reuse the same node"
    );

    // A structurally different term is a distinct node.
    let other = ATerm::constant(&Symbol::new("a", 0));
    assert_ne!(first.index(), other.index());
}

#[test]
fn test_miri_term_arguments() {
    let term = build_sample();

    assert_eq!(term.get_head_symbol().name(), "f");
    assert_eq!(term.get_head_symbol().arity(), 2);
    assert_eq!(term.arguments().len(), 2);

    // arg(0) is the constant `a`.
    let arg0 = term.arg(0);
    assert_eq!(arg0.get_head_symbol().name(), "a");
    assert_eq!(arg0.arguments().len(), 0);

    // arg(1) is `g(a)`, whose only argument is again `a`, shared with arg(0).
    let arg1 = term.arg(1);
    assert_eq!(arg1.get_head_symbol().name(), "g");
    let nested = arg1.arg(0);
    assert_eq!(nested.get_head_symbol().name(), "a");
    assert_eq!(
        nested.index(),
        arg0.index(),
        "the inner `a` is the same shared node as arg(0)"
    );
}

#[test]
fn test_miri_protection_survives_gc() {
    // A protected term must survive forced garbage collection with its structure intact.
    let term = build_sample();

    THREAD_TERM_POOL.with(|tp| {
        tp.force_collect_garbage();
        tp.force_collect_garbage();
    });

    assert_eq!(term.get_head_symbol().name(), "f");
    assert_eq!(term.arg(1).arg(0).get_head_symbol().name(), "a");

    // Rebuilding the same term after GC still yields the same shared node.
    assert_eq!(term.index(), build_sample().index());
}

#[test]
fn test_miri_send_roundtrip() {
    // The `Send` wrapper keeps its term alive across GC, even on a single thread, and converting
    // back to a protected `ATerm` yields the original shared node.
    let term = build_sample();
    let send = ATermSend::from(build_sample());

    THREAD_TERM_POOL.with(|tp| tp.force_collect_garbage());

    assert_eq!(send.get_head_symbol().name(), "f");
    assert_eq!(send.protect().index(), term.index());
}

#[test]
fn test_miri_term_iterator() {
    // The subterm iterator traverses every edge without deduplicating shared nodes, so `f(a, g(a))`
    // visits f, a, g(a) and the shared `a` again: four nodes in total.
    let term = build_sample();
    assert_eq!(term.iter().count(), 4);
}

#[test]
fn test_miri_binary_writer_survives_gc() {
    // `BinaryATermWriter`'s internal state (`function_symbols`/`terms`/`stack`) is
    // `GlobalProtected`, so it must survive garbage collection just as a plain `Protected`
    // container did before it.
    let mut buffer: Vec<u8> = Vec::new();
    let mut writer = BinaryATermWriter::new(&mut buffer).unwrap();
    writer.write_aterm(&build_sample()).unwrap();

    THREAD_TERM_POOL.with(|tp| tp.force_collect_garbage());

    writer.write_aterm(&build_sample()).unwrap();
    ATermWrite::flush(&mut writer).unwrap();
    drop(writer);

    let mut reader = BinaryATermReader::new(&buffer[..]).unwrap();
    let first = reader.read_aterm().unwrap().expect("first term must be present");
    let second = reader.read_aterm().unwrap().expect("second term must be present");
    assert_eq!(first, build_sample());
    assert_eq!(second, build_sample());
}

#[test]
fn test_miri_global_protected_send_across_threads() {
    // A `GlobalProtected` created on one thread must be usable -- read, written, and dropped --
    // from a different thread, unlike a plain `Protected` container.
    let mut protected = ProtectedSend::<Vec<SymbolRef<'static>>>::new(Vec::new());
    let symbol = Symbol::new("global_protected_send", 0);
    protected.write().push(symbol.copy());

    let protected = std::thread::spawn(move || {
        THREAD_TERM_POOL.with(|tp| tp.force_collect_garbage());
        assert_eq!(
            protected.read().len(),
            1,
            "the pushed symbol must survive the move and a GC"
        );
        protected
    })
    .join()
    .unwrap();

    assert_eq!(protected.read()[0].name(), "global_protected_send");
    // Dropped here, on yet another "thread" (still the joining one) than the one it was created
    // on -- exercising `GlobalProtected::drop` without a `THREAD_TERM_POOL` lookup.
}

/// `ATermArgs` previously did not override `size_hint`, so its lower bound was 0
/// and there was no upper bound. Adapters such as `Skip` and `zip` use `size_hint`
/// to implement their own `ExactSizeIterator::len`, so without the fix they would
/// return 0 regardless of the actual remaining count.
#[test]
fn test_aterm_args_size_hint_is_exact() {
    // `h(a, b, c)` has arity 3, giving a three-element `ATermArgs` iterator.
    let a = ATerm::constant(&Symbol::new("a_sh", 0));
    let b = ATerm::constant(&Symbol::new("b_sh", 0));
    let c = ATerm::constant(&Symbol::new("c_sh", 0));
    let term = ATerm::with_args(&Symbol::new("h_sh", 3), &[a.copy(), b.copy(), c.copy()]).protect();

    // Fresh iterator: all 3 arguments remain.
    let mut iter = term.arguments();
    assert_eq!(iter.size_hint(), (3, Some(3)), "size_hint before advancing");
    assert_eq!(iter.len(), 3, "ExactSizeIterator::len before advancing");

    // Consume one argument; 2 remain.
    iter.next();
    assert_eq!(iter.size_hint(), (2, Some(2)), "size_hint after one next()");
    assert_eq!(iter.len(), 2, "ExactSizeIterator::len after one next()");

    // `skip` is built on top of `size_hint`; with the fix it must still report the
    // correct remaining length.
    let after_skip = term.arguments().skip(1);
    assert_eq!(
        after_skip.len(),
        2,
        "skip(1) on a 3-argument iterator must report len 2"
    );
}

/// Empty boundary: an arity-0 term's argument iterator must be empty in both directions with
/// no underflow in `ATermArgs::next_back` (`self.arity -= 1` is only reached after `self.index
/// < self.arity` is checked, so `arity == 0` must short-circuit before that subtraction).
#[test]
fn test_boundary_zero_arity_term_has_no_arguments() {
    let c = ATerm::constant(&Symbol::new("boundary_zero_arity", 0));
    let mut args = c.arguments();
    assert!(args.is_empty());
    assert_eq!(args.len(), 0);
    assert_eq!(args.next(), None);
    assert_eq!(args.next_back(), None);
}

/// Boundary between the fixed-arity storage tables and the dynamically sized fallback:
/// `MAX_FIXED_ARITY == 7`, so arity 7 is the last symbol handled by `insert_fixed_iter`
/// (`terms_7`) and arity 8 is the first to fall through to `ATermStorage::insert`'s
/// `SliceDst`-based `terms` table. Both must construct and read back correctly.
#[test]
fn test_boundary_max_fixed_arity_and_first_dynamic_arity() {
    let leaf = ATerm::constant(&Symbol::new("boundary_leaf", 0));

    let seven_args: Vec<ATerm> = (0..7).map(|_| leaf.copy().protect()).collect();
    let seven = ATerm::with_args(&Symbol::new("boundary_seven", 7), &seven_args).protect();
    assert_eq!(seven.get_head_symbol().arity(), 7);
    assert_eq!(seven.arguments().len(), 7);
    for arg in seven.arguments() {
        assert_eq!(arg.index(), leaf.index());
    }

    let eight_args: Vec<ATerm> = (0..8).map(|_| leaf.copy().protect()).collect();
    let eight = ATerm::with_args(&Symbol::new("boundary_eight", 8), &eight_args).protect();
    assert_eq!(eight.get_head_symbol().arity(), 8);
    assert_eq!(eight.arguments().len(), 8);
    for arg in eight.arguments() {
        assert_eq!(arg.index(), leaf.index());
    }
}

/// Aliasing boundary: in `f(a, a)`, `arg(0)` and `arg(1)` are the very same interned node
/// (maximal sharing), so reading through both `ATermRef` handles at once is reading the same
/// address through two independently reconstructed shared borrows -- sound under Stacked/Tree
/// Borrows only because both accesses are read-only (there is no `&mut` anywhere in this path).
#[test]
fn test_boundary_shared_argument_aliasing() {
    let a = ATerm::constant(&Symbol::new("boundary_alias_a", 0));
    let f = ATerm::with_args(&Symbol::new("boundary_alias_f", 2), &[a.copy(), a.copy()]).protect();

    let arg0 = f.arg(0);
    let arg1 = f.arg(1);
    assert_eq!(arg0.index(), arg1.index(), "both arguments are the same shared node");

    // Read through both aliases "at once" (interleaved, not just sequentially dropped).
    let name0 = arg0.get_head_symbol().name();
    let name1 = arg1.get_head_symbol().name();
    assert_eq!(name0, name1);
    assert_eq!(name0, "boundary_alias_a");
}

/// Exact end of the valid region: `arg(arity - 1)` is the last valid index and must succeed.
#[test]
fn test_boundary_arg_at_last_valid_index_succeeds() {
    let a = ATerm::constant(&Symbol::new("boundary_last_arg_a", 0));
    let b = ATerm::constant(&Symbol::new("boundary_last_arg_b", 0));
    let term = ATerm::with_args(&Symbol::new("boundary_last_arg_f", 2), &[a.copy(), b.copy()]).protect();

    assert_eq!(term.arg(1).get_head_symbol().name(), "boundary_last_arg_b");
}

/// One past the end of the valid region: `arg(arity)` must panic via the ordinary checked slice
/// index in `Term::arg`, never silently read past the arguments array.
#[test]
#[should_panic]
fn test_boundary_arg_one_past_last_valid_index_panics() {
    let a = ATerm::constant(&Symbol::new("boundary_past_arg_a", 0));
    let b = ATerm::constant(&Symbol::new("boundary_past_arg_b", 0));
    let term = ATerm::with_args(&Symbol::new("boundary_past_arg_f", 2), &[a.copy(), b.copy()]).protect();

    let _ = term.arg(2);
}

/// Empty-container boundary for [`Transmutable`]: shrinking the lifetime of an empty `Vec`,
/// `VecDeque`, `Option::None` and empty slice must not read or touch any element (there are
/// none to touch) while still returning a validly typed empty view.
#[test]
fn test_boundary_transmutable_empty_containers() {
    let empty_vec: Vec<ATermRef<'static>> = Vec::new();
    // SAFETY: the transmuted lifetime does not outlive `empty_vec`.
    let viewed_vec: &Vec<ATermRef<'_>> = unsafe { empty_vec.transmute_lifetime() };
    assert!(viewed_vec.is_empty());

    let empty_deque: VecDeque<ATermRef<'static>> = VecDeque::new();
    // SAFETY: see above.
    let viewed_deque: &VecDeque<ATermRef<'_>> = unsafe { empty_deque.transmute_lifetime() };
    assert!(viewed_deque.is_empty());

    let none: Option<ATermRef<'static>> = None;
    // SAFETY: see above.
    let viewed_none: &Option<ATermRef<'_>> = unsafe { none.transmute_lifetime() };
    assert!(viewed_none.is_none());

    let empty_slice: &[ATermRef<'static>] = &[];
    // SAFETY: see above.
    let viewed_slice: &[ATermRef<'_>] = unsafe { empty_slice.transmute_lifetime() };
    assert!(viewed_slice.is_empty());
}

/// Single-element boundary for [`Transmutable`]: a one-element `Vec` must preserve the
/// identity (pointer/index) of its one element across the lifetime shrink, exercising the same
/// transmute the empty case above cannot (there is nothing to compare identity against there).
#[test]
fn test_boundary_transmutable_single_element_preserves_identity() {
    let leaf = ATerm::constant(&Symbol::new("boundary_transmute_leaf", 0));

    // `Transmutable` is only implemented for `ATermRef<'static>` (see `transmutable.rs`), so
    // building the fixture needs the same "shrink a real borrow to `'static`, then only ever
    // observe it for no longer than the source lives" trick `ProtectedWriteGuard::protect` uses
    // internally.
    // SAFETY: the `'static`-labeled copy is only read below, strictly before `leaf` (and hence
    // `v`) goes out of scope at the end of this function.
    let v: Vec<ATermRef<'static>> = vec![unsafe { std::mem::transmute::<ATermRef<'_>, ATermRef<'static>>(leaf.copy()) }];

    // SAFETY: the transmuted lifetime does not outlive `v` (which itself does not outlive
    // `leaf`, the term it borrows from).
    let viewed: &Vec<ATermRef<'_>> = unsafe { v.transmute_lifetime() };
    assert_eq!(viewed.len(), 1);
    assert_eq!(viewed[0].index(), leaf.index());
}
