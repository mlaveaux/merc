# merc_lps

Native support for mCRL2's Linear Process Specification (`.lps`) binary
format: a reader for the format (`LinearProcessSpecification::read_file`),
and a `merc_explore::LPS`/`Summand` implementation that instantiates `sum`
variables with `merc_enumerate::Enumerator` instead of the mCRL2 FFI
enumerator used by `tools/mcrl2/crates/merc_lps`.

See `docs/enumeration-crate-plan.md` (Phase 3, section 7) in the workspace
root for the design this crate implements.
