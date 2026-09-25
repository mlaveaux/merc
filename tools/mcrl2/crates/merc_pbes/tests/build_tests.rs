//! Compile-fail regression guards. Previously `PbesSrfContext`/`PbesContext` carried
//! `unsafe impl Send`, letting safe code move one created on the main thread into a spawned
//! thread and crash the process (their `LearnSuccessorsContext`/`PbesRewriteContext` fields are
//! documented as thread-affine — see the phase-1b review report). Both impls were removed;
//! these fixtures assert that moving such a context across threads is now a compile error, not
//! a runtime crash.
#[test]
#[cfg_attr(miri, ignore)]
fn test_context_soundness() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/input/pbes_srf_context_not_send.rs");
    t.compile_fail("tests/input/pbes_context_not_send.rs");
}
