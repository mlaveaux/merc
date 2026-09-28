use merc_unsafety::StablePointerSet;

fn main() {
    let set: StablePointerSet<i32> = StablePointerSet::new();
    let (ptr, _) = set.insert(42);

    // `ptr()` can read the pointee (for a `T` whose `Erasable::unerase` does),
    // so it must require `unsafe` -- this must fail to compile.
    let _raw = ptr.ptr();
}
