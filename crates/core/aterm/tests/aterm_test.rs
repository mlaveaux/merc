//! Behaviour of terms, term lists, strings, ordering and protected containers through the public API.

use std::sync::Arc;

use parking_lot::Mutex;

use merc_aterm::ATerm;
use merc_aterm::ATermInt;
use merc_aterm::ATermList;
use merc_aterm::ATermRef;
use merc_aterm::ATermSend;
use merc_aterm::ATermString;
use merc_aterm::Protected;
use merc_aterm::StructuralOrd;
use merc_aterm::Symb;
use merc_aterm::Symbol;
use merc_aterm::Term;
use merc_aterm::storage::THREAD_TERM_POOL;

fn term(text: &str) -> ATerm {
    ATerm::from_string(text).unwrap()
}

#[test]
#[should_panic(expected = "arg(1) is not defined")]
fn test_arg_out_of_bounds_panics() {
    // `arg` is safe, so an out-of-bounds index must panic in every build profile instead of
    // reading past the term's arguments.
    let t = term("f(a)");

    let _ = t.arg(1);
}

#[test]
fn test_debug_prints_arguments_left_to_right() {
    assert_eq!(format!("{:?}", term("f(a, b, g(b))")), "f(a, b, g(b))");
}

#[test]
#[cfg_attr(miri, ignore)] // Building 300k terms is too slow under miri.
fn test_debug_deeply_nested_term() {
    // Formatting must not recurse once per nesting level.
    let f = Symbol::new("f", 1);
    let mut t = ATerm::constant(&Symbol::new("a", 0));
    for _ in 0..300_000 {
        t = ATerm::with_args(&f, &[t]).protect();
    }

    let text = format!("{t:?}");
    assert!(text.starts_with("f(f("));
    assert!(text.ends_with("))"));
}

#[test]
fn test_send_term_outlives_creating_thread() {
    // An ATermSend must keep its term alive after the creating thread exits, even across garbage
    // collection.
    let symbol = Symbol::new("send_outlives", 0);
    let term = std::thread::spawn(|| ATermSend::from(ATerm::constant(&Symbol::new("send_outlives", 0))))
        .join()
        .expect("The thread does not panic");

    THREAD_TERM_POOL.with(|tp| {
        tp.force_collect_garbage();
        tp.force_collect_garbage();
    });

    assert_eq!(term.get_head_symbol(), symbol.copy());
}

#[test]
#[cfg_attr(miri, ignore)] // This test runs too slow under miri.
fn test_send_terms_from_two_threads() {
    let symbol = Symbol::new("a", 0);
    let term = Arc::new(Mutex::new(ATermSend::from(ATerm::constant(&symbol))));

    let thread = {
        let term = term.clone();

        std::thread::spawn(move || {
            let symbol = Symbol::new("a", 0);

            for _ in 0..100000 {
                *term.lock() = ATermSend::from(ATerm::constant(&symbol));
            }
        })
    };

    for _ in 0..100000 {
        *term.lock() = ATermSend::from(ATerm::constant(&symbol));
    }

    thread.join().expect("The thread does not panic");
    assert_eq!(term.lock().get_head_symbol(), symbol.copy());
}

#[test]
fn test_list_term() {
    let list = ATermList::from_double_iter(vec![ATermInt::new(1), ATermInt::new(2), ATermInt::new(3)].into_iter());

    assert_eq!(list.head().value(), 1);
    assert_eq!(list.tail().head().value(), 2);
    assert_eq!(list.tail().tail().head().value(), 3);
    assert!(list.tail().tail().tail().is_empty());
}

#[test]
fn test_string_term() {
    let s = ATermString::new("test");

    assert_eq!(s.value(), "test");
    assert_eq!(s.to_string(), "test");
}

#[test]
fn test_protected_container_is_the_only_root_across_gc() {
    let mut container = Protected::<Vec<ATermRef<'static>>>::new(vec![]);
    {
        let t = term("pc_root_f(pc_root_g(pc_root_a), pc_root_b)");
        container.write(|write| write.push(t.get()));
        // `t` is dropped here, so the container is the only root of the term.
    }

    // Create garbage before and after the collection, so a collected slot would be reused.
    for i in 0..1000 {
        let _ = term(&format!("pc_garbage_{i}(pc_x, pc_y)"));
    }
    THREAD_TERM_POOL.with(|tp| tp.force_collect_garbage());
    for i in 0..1000 {
        let _ = term(&format!("pc_other_{i}(pc_x, pc_y)"));
    }

    let read = container.read();
    let entry = &read[0];
    assert_eq!(entry.get_head_symbol().name(), "pc_root_f");
    assert_eq!(entry.arg(0).get_head_symbol().name(), "pc_root_g");
    assert_eq!(entry.arg(1).get_head_symbol().name(), "pc_root_b");
}

#[test]
fn test_structural_order() {
    let ordered = [
        "a",
        "b",
        "f(a)",
        "f(b)",
        "f(a, b)",
        "f(a, g(a))",
        "f(a, g(b))",
        "f(b, a)",
        "g(a)",
    ];

    for (i, lhs) in ordered.iter().enumerate() {
        for (j, rhs) in ordered.iter().enumerate() {
            assert_eq!(
                StructuralOrd(term(lhs)).cmp(&StructuralOrd(term(rhs))),
                i.cmp(&j),
                "comparing {lhs} and {rhs}"
            );
        }
    }
}

#[test]
fn test_structural_order_integers() {
    assert!(StructuralOrd(ATermInt::new(2)) < StructuralOrd(ATermInt::new(10)));
    assert!(StructuralOrd(ATerm::from(ATermInt::new(10))) < StructuralOrd(term("a")));
}
