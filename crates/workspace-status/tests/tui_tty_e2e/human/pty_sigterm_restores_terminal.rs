use std::thread;
use std::time::{Duration, Instant};

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{tree_has, WAIT};

/// Leave the alternate screen (`CSI ? 1049 l`).
const LEAVE_ALT_SCREEN: &[u8] = b"\x1b[?1049l";

/// `SIGTERM` to a running TUI gives the terminal back before `ws` dies:
/// the child leaves the alternate screen, turns mouse capture off, shows
/// the cursor, and leaves raw mode.
///
/// Without the signal restore, the default `SIGTERM` action ends the
/// process with raw mode, the alternate screen, and mouse capture still on,
/// and none of these bytes follow the signal.
#[test]
fn pty_sigterm_restores_terminal() {
    let (_root, workspace) = daily_workspace();
    let tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| tree_has(screen, "README.md") && screen.contains("? help"),
        "first paint: tree chrome",
        WAIT,
    );
    let raw_mode = libc::ICANON | libc::ECHO;
    assert_eq!(
        tui.local_modes() & raw_mode,
        0,
        "the running TUI has the pty in raw mode"
    );

    let before = tui.raw_output().len();
    let pid = libc::pid_t::try_from(tui.child_pid()).expect("pid fits pid_t");
    // SAFETY: a plain syscall with no pointers, to the child this test
    // spawned and has not reaped.
    assert_eq!(unsafe { libc::kill(pid, libc::SIGTERM) }, 0, "send SIGTERM");

    let deadline = Instant::now() + WAIT;
    loop {
        let after = tui.raw_output()[before..].to_vec();
        let restored = after
            .windows(LEAVE_ALT_SCREEN.len())
            .any(|w| w == LEAVE_ALT_SCREEN)
            && after.windows(8).any(|w| w == b"\x1b[?1000l")
            && after.windows(6).any(|w| w == b"\x1b[?25h");
        if restored && tui.local_modes() & raw_mode == raw_mode {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "after SIGTERM: leave the alternate screen, mouse off, cursor shown, raw mode off; \
             bytes after the signal: {:?}; local modes: {:#x}",
            String::from_utf8_lossy(&after),
            tui.local_modes()
        );
        thread::sleep(Duration::from_millis(20));
    }
}
