//! Regression test for `merc_tools::init_console` on Windows.
//!
//! `init_console` calls `AttachConsole(ATTACH_PARENT_PROCESS)` whenever
//! `GetConsoleWindow()` is null, i.e. whenever the current process has no
//! console *window* -- which is also true of a process whose stdout/stderr
//! were redirected to a file or a pipe by its caller (`myapp.exe > out.log`,
//! `myapp.exe | findstr foo`). Per Microsoft's documented behaviour (and
//! per real-world bugs filed against other Rust projects that hit exactly
//! this, e.g. https://github.com/firezone/firezone/pull/15071),
//! `AttachConsole`/`AllocConsole` overwrite the process's `STD_OUTPUT_HANDLE`
//! / `STD_ERROR_HANDLE` with a handle to the console screen buffer,
//! discarding whatever the caller had redirected them to.
//!
//! `crates/tools/src/console.rs` never checks whether stdout/stderr already
//! point at a real, redirected destination before attaching, so a caller
//! that redirects this binary's output loses that redirection: subsequent
//! `println!`/log output goes to the (re-)attached console instead of the
//! file or pipe the caller asked for.
//!
//! This test cannot be executed in this review sandbox (no Windows runtime
//! or Wine is available here -- see the review report), but it does
//! type-check against the real `winapi` FFI declarations under
//! `cargo check --target x86_64-pc-windows-gnu --tests -p merc_tools`.
//! It is left in place as the regression test for the fix.

#![cfg(windows)]

use std::fs::File;
use std::io::Read;
use std::os::windows::io::AsRawHandle;

use winapi::um::processenv::GetStdHandle;
use winapi::um::processenv::SetStdHandle;
use winapi::um::winbase::STD_OUTPUT_HANDLE;
use winapi::um::wincon::FreeConsole;

/// `init_console` must not replace an already-redirected `STD_OUTPUT_HANDLE`
/// with a handle to an attached/allocated console.
///
/// Simulates a GUI-subsystem process launched with its stdout redirected to
/// a file (`myapp.exe > out.log`): the handle is set explicitly via
/// `SetStdHandle` before `init_console` runs, exactly as the OS loader does
/// for a real redirected launch. `FreeConsole` beforehand reproduces the
/// "no console window" state (`GetConsoleWindow().is_null()`) that a
/// console-less GUI-subsystem process starts in and that redirection alone
/// does not change.
#[test]
fn init_console_preserves_redirected_stdout_handle() {
    // SAFETY: `FreeConsole` takes no arguments and detaching this test
    // process's console (if it has one) does not invalidate any live
    // references; it only changes what `GetConsoleWindow` reports next.
    unsafe {
        FreeConsole();
    }

    let dir = std::env::temp_dir();
    let path = dir.join(format!("merc_console_test_{}.log", std::process::id()));
    let file = File::create(&path).expect("failed to create redirect target");
    let redirected_handle = file.as_raw_handle();

    // SAFETY: `redirected_handle` comes from a `File` kept alive for the
    // rest of this test, and `STD_OUTPUT_HANDLE` is a valid handle type.
    let ok = unsafe { SetStdHandle(STD_OUTPUT_HANDLE, redirected_handle as *mut _) };
    assert_ne!(ok, 0, "SetStdHandle failed to set up the redirection fixture");

    let before = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    assert_eq!(before as usize, redirected_handle as usize);

    let _console = merc_tools::init_console().expect("init_console failed");

    let after = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    assert_eq!(
        after as usize, redirected_handle as usize,
        "init_console() replaced the caller's redirected STD_OUTPUT_HANDLE with a console \
         handle; output that should have gone to the redirect target will go to the console \
         screen buffer instead"
    );

    drop(file);
    let mut check = String::new();
    if let Ok(mut reopened) = File::open(&path) {
        let _ = reopened.read_to_string(&mut check);
    }
    let _ = std::fs::remove_file(&path);
}
