use crate::harness::PtySession;
use crate::seed::compare_ahead_workspace;
use crate::support::{GIT_WAIT, WAIT};

/// `'` on a compare DiffPane copies an entity reference. It does not switch tabs.
#[test]
fn pty_compare_apostrophe_copies_diff() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    tui.ctrl_letter('k');
    tui.keys("vs default");
    tui.enter();
    tui.wait_contains("app · vs origin/main", GIT_WAIT);
    tui.enter();
    tui.wait_pred(
        |screen| screen.contains("COMMITTED") && screen.contains("alpha.txt"),
        "Enter focuses the compare DiffPane",
        WAIT,
    );
    assert!(
        tui.clipboard_payloads().is_empty(),
        "open must not copy:\n{}",
        tui.screen()
    );

    tui.key('\'');
    tui.wait_pred(
        |screen| {
            screen.contains("copied")
                && !screen.contains("Switch to Workspace tab")
                && !screen.contains("no copy target")
        },
        "' on compare DiffPane flashes copied",
        WAIT,
    );
    tui.wait_clipboard_pred(
        |payloads| {
            payloads.iter().any(|text| {
                text.contains("kind: diff") && text.contains("path: alpha.txt")
            })
        },
        "OSC 52 payload is kind: diff plus path alpha.txt",
        WAIT,
    );
}
