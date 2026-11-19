use std::fs;

use workspace_status::tui::workspace_store_id;

use crate::harness::PtySession;
use crate::seed::tree_focus_workspace;
use crate::support::{after_ack, tree_focused_on_abc, write_ui_store, GIT_WAIT, WAIT};

/// A saved tree focus is restored at launch with a one-time notice.
///
/// The UI-state store is written before launch with `treeFocus` mapping the
/// fixture's workspace store id (the id the app keys it by) to `ABC-123`.
/// Launch shows `focus restored: ABC-123`, the chip
/// `(o) ABC-123 · 2/4 repos` and the folded `2 outside focus · 1 dirty` row,
/// with no key pressed. Quit leaves the store as it was.
#[test]
fn pty_focus_restored_at_launch() {
    let (_root, workspace) = tree_focus_workspace();
    let ui_store = workspace.join(".e2e-state").join("ui-state-focus.json");
    let body = format!(
        "{{\n  \"version\": 1,\n  \"activitySparkline\": false,\n  \"treeFocus\": {{\n    \"{}\": \"ABC-123\"\n  }}\n}}\n",
        workspace_store_id(&workspace)
    );
    write_ui_store(&ui_store, &body);
    let env = [("WS_STATUS_UI_STORE", ui_store.to_str().unwrap())];

    let mut tui = PtySession::open_with_env(&workspace, &env);
    tui.wait_pred(
        after_ack("focus restored: ABC-123", tree_focused_on_abc),
        "launch restores the saved ABC-123 focus: notice, chip, folded outside row",
        GIT_WAIT,
    );
    tui.key('q');
    tui.wait_exit(WAIT);
    assert_eq!(
        fs::read_to_string(&ui_store).unwrap(),
        body,
        "a launch that only restores the focus must leave the UI-state store unchanged"
    );
}
