//! Give the terminal back when an exit signal (`SIGINT`, `SIGTERM`,
//! `SIGHUP`) ends `ws` while the TUI owns it.
//!
//! The git exit-signal handler (`git::stop_git_on_exit_signals`) calls
//! [`restore_from_signal`] through the hook that `run_tui` registers
//! (`git::set_exit_signal_hook`), so `git.rs` does not depend on the TUI.
//! A signal handler may call only async-signal-safe functions: no
//! crossterm, no allocation, no lock. So this restore writes fixed bytes
//! ([`RESTORE_SEQ`], the same bytes `restore_terminal` writes) with
//! `write(2)`, and puts back the terminal modes saved before raw mode
//! ([`save_terminal_modes`]) with `tcsetattr(3)`.

use std::sync::atomic::Ordering;
use std::sync::OnceLock;

use super::app::TERMINAL_SET_UP;
use super::pointer_shape::{POINTER_RESET, POINTER_SET};
use super::term_progress::{PROGRESS_CLEAR, PROGRESS_SHOWN};

/// What `restore_terminal` writes with no progress bar and no resize
/// pointer shown: mouse capture off (SGR, urxvt, any-event, button-event,
/// normal tracking), keyboard enhancement popped, synchronized update
/// ended, alternate screen left, cursor shown. A unit test compares it
/// with `restore_terminal_to`, so the two exit paths cannot drift.
pub(super) const RESTORE_SEQ: &[u8] =
    b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[<1u\x1b[?2026l\x1b[?1049l\x1b[?25h";

/// The terminal modes from before raw mode, and the descriptor they were
/// read from.
struct SavedModes {
    /// Stdin when it is a terminal, else an open `/dev/tty`: the same
    /// choice crossterm makes for raw mode.
    fd: libc::c_int,
    /// The modes `tcgetattr` read from `fd`.
    termios: libc::termios,
}

/// Set once by [`save_terminal_modes`]. The signal handler reads it with
/// `OnceLock::get`, which is one atomic load and a reference: no lock and
/// no allocation.
static SAVED_MODES: OnceLock<SavedModes> = OnceLock::new();

/// Save the terminal modes for [`restore_from_signal`]. `run_tui` calls it
/// just before raw mode.
///
/// It reads them from the terminal crossterm puts in raw mode: stdin when
/// stdin is a terminal, else `/dev/tty` (stdin from a pipe, say). The
/// `/dev/tty` descriptor stays open for the life of the process, so the
/// signal handler does not have to open it. With no terminal it saves
/// nothing, and the signal restore then only writes [`RESTORE_SEQ`].
pub(super) fn save_terminal_modes() {
    if SAVED_MODES.get().is_some() {
        return;
    }
    // SAFETY: `isatty`, `open` and `close` are plain syscalls; the path is
    // a NUL-terminated literal.
    let fd = unsafe {
        if libc::isatty(libc::STDIN_FILENO) == 1 {
            libc::STDIN_FILENO
        } else {
            libc::open(c"/dev/tty".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC)
        }
    };
    if fd < 0 {
        return;
    }
    let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: `tcgetattr` writes a whole `termios` into the pointer on
    // success, and the value is read only then.
    unsafe {
        if libc::tcgetattr(fd, termios.as_mut_ptr()) == 0 {
            let termios = termios.assume_init();
            let _ = SAVED_MODES.set(SavedModes { fd, termios });
        } else if fd != libc::STDIN_FILENO {
            libc::close(fd);
        }
    }
}

/// Restore the terminal from an exit-signal handler. Async-signal-safe:
/// atomics, `write(2)` and `tcsetattr(3)` only.
///
/// Acts only while the TUI has the terminal set up, and only once (it
/// clears the set-up flag, like `restore_terminal`). It clears a shown
/// OSC 9;4 bar and resets a shown OSC 22 pointer first, as
/// `restore_terminal` does, then writes [`RESTORE_SEQ`] and leaves raw
/// mode.
pub(super) fn restore_from_signal() {
    if !TERMINAL_SET_UP.swap(false, Ordering::SeqCst) {
        return;
    }
    if PROGRESS_SHOWN.swap(false, Ordering::SeqCst) {
        write_stdout(PROGRESS_CLEAR.as_bytes());
    }
    if POINTER_SET.swap(false, Ordering::SeqCst) {
        write_stdout(POINTER_RESET.as_bytes());
    }
    write_stdout(RESTORE_SEQ);
    if let Some(saved) = SAVED_MODES.get() {
        // SAFETY: `saved.termios` is a `termios` that `tcgetattr` filled
        // from `saved.fd`, which stays open; the call only reads it.
        unsafe {
            libc::tcsetattr(saved.fd, libc::TCSANOW, &saved.termios);
        }
    }
}

/// Write all of `bytes` to stdout with `write(2)`. A short write goes on
/// with the rest, an interrupted one tries again, and any other error
/// stops: the process is about to end from the signal.
fn write_stdout(mut bytes: &[u8]) {
    while !bytes.is_empty() {
        // SAFETY: the pointer and length name the live `bytes` slice.
        let wrote = unsafe { libc::write(libc::STDOUT_FILENO, bytes.as_ptr().cast(), bytes.len()) };
        match usize::try_from(wrote) {
            Ok(0) => return,
            Ok(n) => bytes = &bytes[n.min(bytes.len())..],
            Err(_) => {
                if std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RESTORE_SEQ;
    use crate::tui::app::restore_terminal_to;

    #[test]
    fn signal_restore_writes_the_bytes_of_the_exit_restore() {
        let mut out = Vec::new();
        restore_terminal_to(&mut out, false, false);
        assert_eq!(
            String::from_utf8_lossy(RESTORE_SEQ),
            String::from_utf8_lossy(&out),
            "the signal restore and restore_terminal write the same bytes"
        );
    }
}
