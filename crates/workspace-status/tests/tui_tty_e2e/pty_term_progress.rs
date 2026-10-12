//! OSC 9;4 terminal progress while a workspace fetch-all runs.

use std::thread;
use std::time::{Duration, Instant};

use crate::harness::PtySession;
use crate::seed::two_unfetched_behind_workspace;
use crate::slow_git::{wait_slow_git_started, SlowFetchPullGit};
use crate::support::{ack_rows, tree_cursor_on, GIT_WAIT, WAIT};

/// Start of every OSC 9;4 sequence.
const OSC_9_4: &[u8] = b"\x1b]9;4;";

/// `<st>;<pr>` of each OSC 9;4 sequence in `raw`, in order.
fn progress_states(raw: &[u8]) -> Vec<String> {
    let mut states = Vec::new();
    let mut at = 0;
    while let Some(start) = raw[at..]
        .windows(OSC_9_4.len())
        .position(|w| w == OSC_9_4)
        .map(|i| at + i + OSC_9_4.len())
    {
        let Some(len) = raw[start..].iter().position(|b| *b == 0x07) else {
            break;
        };
        states.push(String::from_utf8_lossy(&raw[start..start + len]).into_owned());
        at = start + len;
    }
    states
}

/// Spawn on the two-repo fixture with the slow git wrapper and `env`.
fn open_two_repos(env: &[(&str, &str)]) -> (std::path::PathBuf, SlowFetchPullGit, PtySession) {
    let (root, workspace) = two_unfetched_behind_workspace();
    let slow = SlowFetchPullGit::install(&workspace);
    let mut all_env = vec![(
        "WORKSPACE_STATUS_GIT",
        slow.shim.to_str().expect("utf-8 shim path"),
    )];
    all_env.extend_from_slice(env);
    let tui = PtySession::open_with_env(&workspace, &all_env);
    (root, slow, tui)
}

/// `gg` onto the workspace row, then `f`: fetch every primary.
fn fetch_all(tui: &mut PtySession, slow: &SlowFetchPullGit) {
    tui.wait_contains("No updates", WAIT);
    tui.gg();
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "workspace"),
        "gg focuses the workspace row",
        WAIT,
    );
    tui.key('f');
    wait_slow_git_started(slow, || tui.screen(), WAIT);
}

/// Wait until the reader thread has every byte the exited child wrote.
fn settle_raw(tui: &PtySession) -> Vec<u8> {
    let start = Instant::now();
    let mut raw = tui.raw_output();
    loop {
        thread::sleep(Duration::from_millis(100));
        let next = tui.raw_output();
        if next.len() == raw.len() || start.elapsed() >= WAIT {
            return next;
        }
        raw = next;
    }
}

/// Workspace `f` over two repos writes a normal OSC 9;4 progress bar while
/// the slow fetches run; `q` before they finish ends with the clear state.
/// Fail if no progress is written, or the last OSC 9;4 sequence before exit
/// is not `0;0` (the terminal keeps a stale bar).
#[test]
fn pty_fetch_all_emits_progress_and_quit_clears_it() {
    let (_root, slow, mut tui) = open_two_repos(&[]);
    fetch_all(&mut tui, &slow);
    let start = Instant::now();
    while !progress_states(&tui.raw_output())
        .iter()
        .any(|state| state.starts_with("1;"))
    {
        assert!(
            start.elapsed() < WAIT,
            "no OSC 9;4 normal progress while the fetch-all runs; states: {:?}; screen:\n{}",
            progress_states(&tui.raw_output()),
            tui.screen()
        );
        thread::sleep(Duration::from_millis(25));
    }
    assert!(
        !ack_rows(&tui.screen()).contains("Fetched"),
        "q must land while the batch is in flight; screen:\n{}",
        tui.screen()
    );

    tui.key('q');
    tui.wait_exit(GIT_WAIT);
    let states = progress_states(&settle_raw(&tui));
    assert_eq!(
        states.last().map(String::as_str),
        Some("0;0"),
        "the last OSC 9;4 before exit clears the bar; states: {states:?}"
    );
}

/// Progress on (the default), no batch: launch and `q` write no OSC 9;4 at
/// all. Fail if the exit writes a clear for a bar that never showed (a
/// terminal that shows OSC 9 as a notification would pop one per quit).
#[test]
fn pty_quit_without_batch_writes_no_osc_9_4() {
    let (_root, _slow, mut tui) = open_two_repos(&[]);
    tui.wait_contains("No updates", WAIT);
    tui.key('q');
    tui.wait_exit(WAIT);
    let raw = settle_raw(&tui);
    assert!(
        !raw.windows(OSC_9_4.len()).any(|w| w == OSC_9_4),
        "no batch: no OSC 9;4; states: {:?}",
        progress_states(&raw)
    );
}

/// `WS_STATUS_TERM_PROGRESS=0`: a full fetch-all and quit write no OSC 9;4.
#[test]
fn pty_term_progress_off_writes_no_osc_9_4() {
    let (_root, slow, mut tui) = open_two_repos(&[("WS_STATUS_TERM_PROGRESS", "0")]);
    fetch_all(&mut tui, &slow);
    tui.wait_pred(
        |screen| ack_rows(screen).contains("Fetched"),
        "the fetch-all lands",
        GIT_WAIT,
    );
    tui.key('q');
    tui.wait_exit(GIT_WAIT);
    let states = progress_states(&settle_raw(&tui));
    assert!(
        states.is_empty(),
        "termProgress off writes no OSC 9;4; states: {states:?}"
    );
}
