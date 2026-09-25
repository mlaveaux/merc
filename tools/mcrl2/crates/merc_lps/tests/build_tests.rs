//! Compile-fail regression guard. `ExplicitContext` previously carried `unsafe impl Send`,
//! letting safe code move one created on the main thread into a spawned thread and crash the
//! process (its `LearnSuccessorsContext` field is documented as thread-affine — see the
//! phase-1b review report). The impl was removed; this fixture asserts that moving such a
//! context across threads is now a compile error, not a runtime crash.
#[test]
#[cfg_attr(miri, ignore)]
fn test_context_soundness() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/input/explicit_context_not_send.rs");
}
