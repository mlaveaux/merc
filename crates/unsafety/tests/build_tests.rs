#[test]
#[cfg_attr(miri, ignore)]
fn test_stable_pointer_ptr_requires_unsafe() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/input/stable_pointer_ptr_requires_unsafe.rs");
}
