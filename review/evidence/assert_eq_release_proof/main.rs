// Standalone, dependency-free proof for the fix in
// review/phase-1b-mcrl2-ffi-atermpp.md, finding 1: that `assert_eq!` panics
// unconditionally in an optimized ("release"-equivalent) build, while
// `debug_assert_eq!` (the guard the fix replaces) is compiled out.
//
// Run:
//   rustc -O --edition 2021 main.rs -o main_release
//   ./main_release debug_only   # exits 0, no panic (compiled out)
//   ./main_release always       # exits 101, panics with the fix's message
fn debug_only(a: usize, b: usize) {
    debug_assert_eq!(a, b, "Number of arguments does not match arity");
}

fn always(a: usize, b: usize) {
    assert_eq!(a, b, "Number of arguments does not match arity");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("debug_only") => {
            debug_only(4, 1);
            println!("debug_only: no panic (compiled out) — cfg(debug_assertions)={}", cfg!(debug_assertions));
        }
        Some("always") => {
            always(4, 1);
            println!("always: no panic (unexpected!)");
        }
        _ => println!("usage: main [debug_only|always]"),
    }
}
