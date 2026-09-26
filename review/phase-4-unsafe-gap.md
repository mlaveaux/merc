# Phase 4 review: `merc_derive_terms` transmute unsafety + Windows console FFI

Scope: `crates/macros/src/merc_derive_terms.rs` (the generated
`unsafe impl Transmutable for #name_ref` / `transmute_lifetime` /
`transmute_lifetime_mut`), and `crates/tools/src/console.rs` (Windows console
FFI: `AllocConsole`/`AttachConsole`/`FreeConsole`/`GetConsoleWindow`). This
worktree's `review/` directory did not carry the prior phases' history (its
branch point predates that work landing on `skills`); prior findings were
read from `/home/user/merc/review/README.md` and the sharedmutex report
(`phase-1-foundation-unsafe-sharedmutex-unsafety.md`) referenced by the task,
directly off the primary checkout, to avoid duplicating them.

**Verdict**: `merc_derive_terms`'s generated `Transmutable` impl is sound for
every struct it is actually applied to today; that soundness property is now
backed by a passing Kani proof (new coverage, not previously present).
`crates/tools/src/console.rs`'s FFI signatures and HANDLE-free `Console`
guard are correct on inspection, but `init_console` has one real, plausible
correctness defect distinct from memory-unsafety: it unconditionally calls
`AttachConsole(ATTACH_PARENT_PROCESS)` whenever the process has no console
*window*, which is also true of a process whose stdout/stderr were already
validly redirected to a file or pipe by its caller — and per documented
Windows behaviour, that call silently discards the caller's redirection.
This cannot be executed in this Linux sandbox (no Windows runtime or Wine
available), so it is reported as PLAUSIBLE, backed by cross-compiled/
type-checked evidence and independent confirmation that other real-world
Rust/C++ projects hit and fixed the identical pattern.

## Finding 1 — `init_console` clobbers a caller's redirected stdout/stderr — PLAUSIBLE, Medium (fix applied, awaiting Windows CI confirmation)

**Status update (implementor pass).** A fix is applied at
`crates/tools/src/console.rs`: `init_console` now also checks
`GetStdHandle(STD_OUTPUT_HANDLE)`/`GetStdHandle(STD_ERROR_HANDLE)` and skips
`AttachConsole`/`AllocConsole` whenever either already holds a real (non-null,
non-`INVALID_HANDLE_VALUE`) handle, per the "direction of a fix" below. The
decision itself was pulled out into a small platform-independent function,
`should_attach_or_allocate_console(has_console_window, stdout_valid,
stderr_valid) -> bool`, so it has native unit tests
(`crates/tools/src/console.rs`, `mod tests`) covering all the relevant
boolean combinations, including the reviewer's exact failure scenario
(`has_console_window = false`, `stdout_valid = true` ⇒ must not attach), and
those pass in this sandbox. The fix also compiles clean, lint-clean, and
format-clean for the real Windows target
(`cargo check -p merc_tools --tests --target x86_64-pc-windows-gnu`,
`cargo clippy -p merc_tools --all-targets --target x86_64-pc-windows-gnu`,
`cargo +nightly fmt -p merc_tools -- --check`), and a manual trace of the
existing `crates/tools/tests/console_windows.rs` regression test against the
new code confirms it takes the "leave stdio alone" branch. This status is
kept at **PLAUSIBLE** rather than raised to FIXED because none of that
exercises the real Win32 APIs (`GetStdHandle`, `SetStdHandle`,
`AttachConsole`) at runtime — this sandbox still has no Windows runtime or
Wine — so `console_windows.rs` itself has still never actually been run.
Confirming FIXED requires a real Windows run of
`cargo test -p merc_tools --test console_windows`.

`crates/tools/src/console.rs:29-64` (`init_console`)

```rust
if GetConsoleWindow().is_null() {
    if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
        if AllocConsole() != 0 { Ok(Console { attached: false }) } else { Err(...) }
    } else {
        Ok(Console { attached: true })
    }
} else {
    Ok(Console { attached: true })
}
```

**Failure scenario.** `ltsgraph` is a GUI-subsystem binary that is also a CLI
tool (`Cli::parse()`, `--version`, `-v`/verbosity flags, `eprintln!` for
version output — `tools/gui/ltsgraph/src/main.rs:143-156`), and it calls
`init_console()` unconditionally as its first action. A GUI-subsystem process
has no console *window* whether or not its standard handles were redirected
by its caller (e.g. `ltsgraph.exe --version > out.txt` or
`ltsgraph.exe --version | findstr foo` from a script or another program).
`GetConsoleWindow().is_null()` is true in exactly that case too, so
`init_console` proceeds to call `AttachConsole(ATTACH_PARENT_PROCESS)`.
Per Microsoft's documented console-handle behaviour, `AttachConsole` (like
`AllocConsole`) re-initialises the process's `STD_OUTPUT_HANDLE`/
`STD_ERROR_HANDLE` to the console screen buffer of the console it attaches
to, discarding whatever the caller had already set those handles to (the
redirected file/pipe). The result: the user's `> out.txt` redirection is
silently broken — `--version` output and any `println!`/log output goes to
the attaching console (if one exists) instead of `out.txt`, or is lost
entirely if no parent console exists (then `AllocConsole` also clobbers the
same handles with a freshly allocated, invisible console).

This is not a hypothetical reading of the docs: it is the exact, named
failure mode of a real, independently-discovered and fixed bug in another
Rust project doing the same `AttachConsole(ATTACH_PARENT_PROCESS)` dance for
a GUI-subsystem CLI tool — see
[firezone/firezone#15071](https://github.com/firezone/firezone/pull/15071)
("When the GUI client is started with a redirected stdout, ... AttachConsole
replaced the inherited pipe with the console and the redirection broke" —
fixed by skipping the attach when stdout is already a valid, non-console
destination) and the equivalent report in
[tomyhara/Mochi#15](https://github.com/tomyhara/Mochi/issues/15) ("console
attach/alloc is decided by argument count, and may clobber redirected
handles"). `console.rs` has no equivalent guard: it decides purely from
`GetConsoleWindow()`, never checking whether stdout/stderr already point at
something usable (e.g. via `GetFileType`) before attaching.

**Why this is PLAUSIBLE, not CONFIRMED.** This sandbox is Linux-only, with no
Windows runtime and no Wine, so the scenario cannot be executed end-to-end
here. What was verified instead:
- The FFI declarations used (`AllocConsole`, `AttachConsole`, `FreeConsole`,
  `GetConsoleWindow`, `ATTACH_PARENT_PROCESS`) were checked character-for-
  character against the real vendored `winapi 0.3.9` source
  (`um/consoleapi.rs`, `um/wincon.rs`, both `extern "system"`) — no signature
  or calling-convention mismatch.
- `crates/tools` (including the new regression test below) cross-compiles
  cleanly for `x86_64-pc-windows-gnu` (`cargo check -p merc_tools --tests
  --target x86_64-pc-windows-gnu`), and passes `cargo clippy
  --all-targets --target x86_64-pc-windows-gnu` and `cargo +nightly fmt
  --check` with no warnings.
- The handle-clobbering mechanism itself was corroborated against
  independent, real-world precedent (above) rather than asserted from
  memory, specifically to avoid a hallucinated API-contract claim.

A regression test that reproduces the exact mechanism is added at
`crates/tools/tests/console_windows.rs`
(`init_console_preserves_redirected_stdout_handle`, `#[cfg(windows)]`): it
detaches the test process's console with `FreeConsole()` (reproducing the
"no console window" state), points `STD_OUTPUT_HANDLE` at a real temp file
via `SetStdHandle` (reproducing an OS-redirected launch), calls
`init_console()`, and asserts `GetStdHandle(STD_OUTPUT_HANDLE)` is
unchanged. It type-checks against the real `winapi` FFI on the Windows
target (`cargo check -p merc_tools --tests --target x86_64-pc-windows-gnu`)
but could not be executed in this sandbox; it is left in place as the
regression test for the fix, to be run on Windows CI.

**Direction of a fix** (not applied): check whether `STD_OUTPUT_HANDLE`
already refers to something usable (e.g. `GetFileType` returns other than
`FILE_TYPE_UNKNOWN`, or is simply not the sentinel invalid handle) before
calling `AttachConsole`/`AllocConsole`, matching the fix in the referenced
PR.

## Checked and found correct — `crates/tools/src/console.rs`

- `Console`'s `attached` field semantics are internally consistent: `true`
  means "a console already existed (either the process's own, or the
  parent's, attached to successfully)" and `Drop` correctly skips
  `FreeConsole` in that case; `false` means "this call allocated a brand new
  console" and `Drop` correctly frees exactly that one. No double-free, no
  freeing of a console this call did not allocate.
- `FreeConsole`/`AllocConsole`/`AttachConsole`/`GetConsoleWindow` signatures,
  calling convention (`extern "system"`), and `ATTACH_PARENT_PROCESS`'s value
  match the real `winapi 0.3.9` declarations exactly (checked against the
  vendored crate source, not from memory).
- `Console` stores no `HANDLE` of any kind (just a `bool`), so there is no
  handle-lifetime/leak concern beyond the console-attachment bookkeeping
  above.
- Single-threaded, called once at process start in the one real caller
  (`tools/gui/ltsgraph/src/main.rs:145`); no concurrency/reentrancy concern.

## Finding 2 (none) — `merc_derive_terms`'s generated `Transmutable` impl — SOUND

`crates/macros/src/merc_derive_terms.rs:262-277` (the `unsafe impl
Transmutable for #name_ref #generics_static` block inside `generated`)

No defect found. This mirrors, mechanically, the hand-written
`unsafe impl Transmutable for ATermRef<'static>` /
`SymbolRef<'static>` in `crates/aterm/src/transmutable.rs:37-59` that every
real caller (`Return::inner` in `crates/aterm/src/aterm.rs:553-559`,
`ProtectedReadGuard`/`ProtectedWriteGuard`'s `Deref`/`DerefMut` in
`crates/aterm/src/protected.rs:280-313`, `SharedTerm::arguments` in
`crates/aterm/src/storage/shared_term.rs:98-102`) already relies on, and
every one of those call sites ties the trait method's otherwise-unconstrained
`'a` to the actual borrow of `self` via ordinary lifetime elision/generic
parameters (e.g. `Return::inner(&self) -> &T::Target<'_>`), not to an
arbitrary caller-chosen lifetime — so the "caller must ensure `'a` does not
outlive the borrow" contract documented on the trait
(`crates/aterm/src/transmutable.rs:20-34`) is upheld everywhere it is used.

Two points worth recording precisely, since they are easy to get wrong when
reasoning about this pattern in the abstract:

1. **`std::mem::transmute::<&Self, &'a Self::Target<'a>>` does not itself
   check that `Self` and `Target<'a>` have matching layout.** `transmute`'s
   built-in size check applies to the types actually passed to it — here
   `&Self` and `&'a Target<'a>`, both ordinary references to `Sized` types,
   which are always pointer-width regardless of what they point to. So this
   pattern's soundness is *not* a compiler-enforced consequence of the
   `transmute` call; it depends entirely on `Self` and `Target<'a>` genuinely
   sharing layout. For every macro-generated `#name_ref`, that holds for a
   stronger reason than "the fields happen to match": `#name_ref
   #generics_static` and `#name_ref #generics_ref` are the *same generic
   struct*, differing only in a lifetime argument, and Rust guarantees
   lifetimes are erased before layout is computed — so they are not merely
   layout-compatible, they are the same compiled type. This holds regardless
   of whether the annotated struct has its own generics (none in the
   codebase do today; see point 2).
2. Every `#[merc_term]` struct in the codebase today (`DataExpression`,
   `SortExpression`, `BasicSort`, `FunctionSort`, `ContainerSort`,
   `SortAlias`, `DataFunctionSymbol`, `DataVariable`, `DataApplication`,
   `MachineNumber`, `DataEquation`, `DataAbstraction`, `WhrDecl`,
   `WhereClause`, `AtermInt`, `AtermString`, `TimedMultiAction`, `Action`,
   `ActionLabel`, ...) has no generics of its own and a single `term: ATerm`
   field (verified by grep across every `#[merc_term(...)]` use site), so the
   struct-with-generics code path in `create_generics_with_lifetimes`/
   `generics_phantom` (which would emit `PhantomData<T>` for the struct's own
   type parameters, `PhantomData<()>` otherwise) is exercised by none of them
   today. Nothing about it looks unsound if it were exercised (`PhantomData`
   is zero-sized regardless of its type argument, so it cannot introduce a
   layout mismatch either), but it is untested; a future `#[merc_term]`
   struct with real, non-phantom-only generics is worth a second look when it
   appears.

### Kani proof (new coverage)

`merc_macros` is a `proc-macro = true` crate: `cargo kani` refuses to target
it at all —

```
$ cd crates/macros && cargo kani
Kani Rust Verifier 0.68.0 (cargo plugin)
CBMC 6.11.0
error: No supported targets were found.
```

— and in any case the unsafe code under review is not present in
`merc_macros`'s own compiled output; it exists only in the tokens the macro
emits, compiled as part of whichever downstream crate applies
`#[merc_derive_terms]`. Proving it against a *real* derived type (e.g.
`DataExpressionRef` in `crates/data`) is impractical: every real `ATermRef`
is only constructible through `THREAD_TERM_POOL`, a global thread-local
`GcMutex`-protected term pool backed by a `DashMap` and a custom freelist
allocator — far too much global, allocation-heavy state for bounded model
checking to explore in reasonable time.

Following the task's fallback instruction, the harness instead runs the
*real* `#[merc_derive_terms]`/`#[merc_term]` macros (not a hand-copied
reproduction of their output) from a new integration test,
`crates/macros/tests/kani_transmute.rs`, against a small mock module
supplying minimal stand-ins for the exact set of names the generated code
references (`ATerm`, `ATermRef`, `Term`, `Markable`, `Marker`, `SymbolRef`,
`ATermArgs`, `TermIterator`, `ATermIndex`, `Transmutable`). An integration
test is an ordinary external consumer of a proc-macro crate — exactly how
`crates/data` uses it — so this is a genuine macro expansion, not a
paraphrase. Three proofs:

- `transmute_lifetime_preserves_value` — the value read back through the
  transmuted reference equals what was written, and the transmuted reference
  points at the same object (not a copy).
- `transmute_lifetime_mut_aliases_original_storage` — a write through the
  transmuted `&mut` is visible through the original binding afterwards
  (rules out the macro ever transmuting by value instead of by reference).
- `generated_types_round_trip_through_conversions` — sanity check that the
  macro really ran (the `From`/`Into` conversions between the mock `ATerm`
  and the generated `Test`/`TestRef` types work).

```
$ cd crates/macros && cargo kani --tests
...
Checking harness transmute_lifetime_mut_aliases_original_storage...
VERIFICATION:- SUCCESSFUL
Checking harness transmute_lifetime_preserves_value...
VERIFICATION:- SUCCESSFUL
Checking harness generated_types_round_trip_through_conversions...
VERIFICATION:- SUCCESSFUL

Manual Harness Summary:
Complete - 3 successfully verified harnesses, 0 failures, 3 total.
```

The harness is gated `#![cfg(kani)]`, so it compiles to zero tests under
plain `cargo test -p merc_macros` (verified) and does not affect normal CI;
running it requires `cargo kani --tests` from `crates/macros` (Kani 0.68.0,
matching `.github/workflows/test_kani.yml`'s pinned version). Wiring it into
that CI workflow is left to the implementor pass, since it is an
infrastructure/CI decision rather than a review finding.

## Tests added

- `crates/tools/tests/console_windows.rs` —
  `init_console_preserves_redirected_stdout_handle` (`#[cfg(windows)]`).
  Regression test for Finding 1. Type-checks on Windows
  (`cargo check -p merc_tools --tests --target x86_64-pc-windows-gnu`); not
  executed (no Windows runtime/Wine in this sandbox). Run on real Windows
  with `cargo test -p merc_tools --test console_windows`.
- `crates/macros/tests/kani_transmute.rs` — three `#[kani::proof]` harnesses
  proving the `Transmutable` soundness property described above, run against
  the real macro output via a mock term type. Run with `cd crates/macros &&
  cargo kani --tests` (all three pass on the current tree; this is new
  coverage, not a regression test for a defect).
- `crates/macros/Cargo.toml` / `crates/tools/Cargo.toml` — minimal
  dev-dependency/config additions needed only to compile the two test files
  above (`[lints] workspace = true` on `merc_macros` to allow `cfg(kani)`
  matching `crates/unsafety`/`crates/number`'s existing precedent; a
  `delegate` dev-dependency for the mock macro consumer; extra `winapi`
  features — `processenv`, `winbase` — as a Windows-only dev-dependency for
  `GetStdHandle`/`SetStdHandle`/`STD_OUTPUT_HANDLE`). No production code was
  changed.
