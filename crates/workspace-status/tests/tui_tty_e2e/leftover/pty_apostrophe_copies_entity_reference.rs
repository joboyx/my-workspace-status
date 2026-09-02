use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{documented_launch_first_paint, pane_unstaged_readme, tree_cursor_on, WAIT};

fn overlay_closed(screen: &str) -> bool {
    !screen.contains("Enter save")
        && !screen.contains("empty deletes")
        && !screen.contains("copied to clipboard")
        && !screen.contains("MOVE")
}

/// Tree file focused, status flash `copied`, no comment / export overlay.
fn tree_file_copied(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && overlay_closed(screen)
        && screen.contains("copied")
        && !screen.contains("no copy target")
        && !screen.contains("no comment target")
}

/// Numbered dirty-file diff focused (Tab from the tree file).
fn right_diff_focused(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && pane_unstaged_readme(screen)
        && screen.contains("[workspace]")
        && overlay_closed(screen)
}

fn payload_is_file(text: &str) -> bool {
    text.contains("kind: file") && text.contains("path: README.md")
}

fn payload_is_diff(text: &str) -> bool {
    text.contains("kind: diff") && text.contains("path: README.md") && text.contains("lines:")
}

/// `'` copies a pasteable entity reference via OSC 52.
///
/// Docs + VIEW: `'` copies entity reference. A focused tree file is a
/// silent no-op for `;` (`no comment target`); copy must still emit OSC
/// 52 with `kind: file`. Tab then `'` on the numbered file diff emits
/// `kind: diff`. Status flash `copied`. No comment overlay. Paint-only
/// (flash without a decoded payload) is red.
#[test]
fn pty_apostrophe_copies_entity_reference() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch focuses the dirty README tree file",
        WAIT,
    );
    assert!(
        tui.clipboard_payloads().is_empty(),
        "launch must not copy:\n{}",
        tui.screen()
    );

    tui.key('\'');
    tui.wait_pred(
        tree_file_copied,
        "' on a tree file flashes copied (not a comment overlay)",
        WAIT,
    );
    tui.wait_clipboard_pred(
        |payloads| payloads.iter().any(|p| payload_is_file(p)),
        "OSC 52 payload is kind: file plus path README.md",
        WAIT,
    );
    let payloads = tui.clipboard_payloads();
    assert!(
        !payloads.is_empty(),
        "tree-file ' must emit OSC 52, not a silent no-op:\n{}",
        tui.screen()
    );
    let file = tui.last_clipboard().expect("kind: file OSC 52 payload");
    assert!(
        payload_is_file(&file),
        "tree-file copy must include kind: file and path: README.md:\n{file}"
    );

    tui.tab();
    tui.wait_pred(
        right_diff_focused,
        "Tab focuses the dirty README diff",
        WAIT,
    );
    let before = tui.clipboard_payloads().len();

    tui.key('\'');
    tui.wait_pred(
        |screen| right_diff_focused(screen) && screen.contains("copied"),
        "' on a numbered diff line flashes copied",
        WAIT,
    );
    tui.wait_clipboard_pred(
        |payloads| payloads.len() > before && payloads.iter().any(|p| payload_is_diff(p)),
        "OSC 52 payload is kind: diff plus path README.md and lines",
        WAIT,
    );
    let payloads = tui.clipboard_payloads();
    assert!(
        payloads.len() > before,
        "diff-line ' must emit a second OSC 52, not a silent no-op:\n{}",
        tui.screen()
    );
    let diff = tui.last_clipboard().expect("kind: diff OSC 52 payload");
    assert!(
        payload_is_diff(&diff),
        "diff copy must include kind: diff, path, and lines:\n{diff}"
    );
}
