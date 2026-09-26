use std::result::Result;

use merc_utilities::MercError;
#[cfg(windows)]
use winapi::um::consoleapi::AllocConsole;
#[cfg(windows)]
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
#[cfg(windows)]
use winapi::um::processenv::GetStdHandle;
#[cfg(windows)]
use winapi::um::winbase::STD_ERROR_HANDLE;
#[cfg(windows)]
use winapi::um::winbase::STD_OUTPUT_HANDLE;
#[cfg(windows)]
use winapi::um::wincon::ATTACH_PARENT_PROCESS;
#[cfg(windows)]
use winapi::um::wincon::AttachConsole;
#[cfg(windows)]
use winapi::um::wincon::FreeConsole;
#[cfg(windows)]
use winapi::um::wincon::GetConsoleWindow;
#[cfg(windows)]
use winapi::um::winnt::HANDLE;

/// Guard returned by [`init_console`]; dropping it frees a console this call allocated.
pub struct Console {
    #[cfg(windows)]
    attached: bool,
}

/// Decides whether `init_console` should attach to or allocate a console, given
/// the current process state. Kept as a plain, platform-independent function of
/// booleans (rather than inlined among the `unsafe`/FFI calls that compute them)
/// so the decision itself is unit-testable on every platform, including this
/// crate's native Linux/macOS test runs, without needing a Windows runtime.
///
/// We must not attach or allocate when the caller already validly redirected
/// either standard stream (`stdout_valid`/`stderr_valid`), since
/// `AttachConsole`/`AllocConsole` re-point *both* `STD_OUTPUT_HANDLE` and
/// `STD_ERROR_HANDLE` at the new console's screen buffer, silently discarding
/// whichever one the caller did set up, even if the other was left at its
/// default (unset) value.
// Only reachable from production code on Windows (see `init_console` below); on
// every other platform its only caller is this module's own `#[cfg(test)]`
// tests, which exercise the decision logic natively without a Windows runtime.
#[cfg_attr(not(windows), allow(dead_code))]
fn should_attach_or_allocate_console(has_console_window: bool, stdout_valid: bool, stderr_valid: bool) -> bool {
    !has_console_window && !stdout_valid && !stderr_valid
}

/// Returns whether `which` (`STD_OUTPUT_HANDLE` or `STD_ERROR_HANDLE`) currently
/// refers to a real, usable destination (a console, a file, or a pipe) rather
/// than being unset.
///
/// A GUI-subsystem process has no console *window*
/// (`GetConsoleWindow().is_null()`) whether or not its caller redirected its
/// standard handles to a file or a pipe, so `GetConsoleWindow` alone cannot
/// distinguish "never given any stdio" from "already validly redirected".
/// `GetStdHandle` can: it is null only when nothing was ever set, and
/// `INVALID_HANDLE_VALUE` only on an explicit failure sentinel: both mean
/// "genuinely unattached", anything else is a real handle the caller already
/// set up and that attaching or allocating a console would silently replace.
#[cfg(windows)]
unsafe fn has_valid_std_handle(which: winapi::shared::minwindef::DWORD) -> bool {
    // SAFETY: `GetStdHandle` takes no pointers and is safe to call in any
    // process state; it only reads process-global state.
    let handle: HANDLE = unsafe { GetStdHandle(which) };
    !handle.is_null() && handle != INVALID_HANDLE_VALUE
}

/// Attaches to a console so `println!` and panic output are visible from a
/// Windows-subsystem GUI binary, which otherwise starts with no attached stdio.
///
/// On Windows this attaches to the parent process' console, an existing console,
/// or allocates a fresh one when none is available. It never does so, however,
/// when the caller already redirected `stdout`/`stderr` to a real destination
/// (a file or a pipe): `AttachConsole`/`AllocConsole` re-point
/// `STD_OUTPUT_HANDLE`/`STD_ERROR_HANDLE` at the new console's screen buffer,
/// which would silently discard that redirection. Dropping the returned guard
/// frees a console that this call allocated. On other platforms it is a no-op and
/// the guard does nothing.
pub fn init_console() -> Result<Console, MercError> {
    #[cfg(windows)]
    unsafe {
        // SAFETY: These console functions take no pointers and are safe to call
        // in any process state; the only soundness obligation is to balance an
        // allocated console with a single `FreeConsole`, which the `Drop` impl
        // does (guarded by `attached` so we never free a pre-existing console).
        // Check if we're attached to an existing Windows console, or if the
        // caller already gave us real, redirected stdio (a file or a pipe) that
        // attaching/allocating a console would clobber.
        let has_console_window = !GetConsoleWindow().is_null();
        let stdout_valid = has_valid_std_handle(STD_OUTPUT_HANDLE);
        let stderr_valid = has_valid_std_handle(STD_ERROR_HANDLE);
        if should_attach_or_allocate_console(has_console_window, stdout_valid, stderr_valid) {
            // Try to attach to an existing Windows console.
            //
            // It's normally a no-brainer to call this - it just makes println! and friends
            // work as expected, without cluttering the screen with a console in the general
            // case.
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                // Try to attach to a console, and if not, allocate ourselves a new one.
                if AllocConsole() != 0 {
                    Ok(Console { attached: false })
                } else {
                    Err("Failed to attach to a console, and to create one".into())
                }
            } else {
                // We attached to an existing console.
                Ok(Console { attached: true })
            }
        } else {
            // The program was started with a console attached, or its stdio was
            // already validly redirected by the caller; leave it alone.
            Ok(Console { attached: true })
        }
    }

    #[cfg(not(windows))]
    {
        Ok(Console {})
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        // Free the allocated console, when it was not attached.
        #[cfg(windows)]
        if !self.attached {
            // SAFETY: `FreeConsole` takes no arguments; `attached == false`
            // means `init_console` allocated this console, so freeing it here
            // balances that allocation exactly once.
            unsafe { FreeConsole() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::should_attach_or_allocate_console;

    /// The only case where we may attach/allocate: no console window, and
    /// neither standard stream already points somewhere real. This is the
    /// case a plain GUI-subsystem process is launched in with no redirection.
    #[test]
    fn attaches_only_when_fully_unattached() {
        assert!(should_attach_or_allocate_console(false, false, false));
    }

    /// A process with a console window already attached must never trigger a
    /// fresh attach/allocate, regardless of the (irrelevant, in that case)
    /// standard-handle state.
    #[test]
    fn never_attaches_with_an_existing_console_window() {
        for stdout_valid in [false, true] {
            for stderr_valid in [false, true] {
                assert!(!should_attach_or_allocate_console(true, stdout_valid, stderr_valid));
            }
        }
    }

    /// The regression case: no console window (true of every GUI-subsystem
    /// process), but the caller already redirected `stdout` (or `stderr`) to a
    /// real file/pipe handle. Attaching or allocating a console here would
    /// silently discard that redirection, so it must not happen.
    #[test]
    fn never_attaches_when_either_stream_is_already_redirected() {
        assert!(!should_attach_or_allocate_console(false, true, false));
        assert!(!should_attach_or_allocate_console(false, false, true));
        assert!(!should_attach_or_allocate_console(false, true, true));
    }
}
