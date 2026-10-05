use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, documented_launch_first_paint, no_updates_group_folded, status_row, tree_cursor_on,
    tree_has, tree_line_containing, SETTLE_MS, WAIT,
};

/// Painted SEARCH status line with this query, the typing cursor, and the
/// live preview's `no match` in place of the Enter-arms hint.
///
/// An armed `/{query}` chip or help `HELP  /{query}` cannot pass.
fn search_prompt_has_no_match_query(screen: &str, query: &str) -> bool {
    let status = status_row(screen);
    status.contains("SEARCH")
        && status.contains(&format!("{query}▏"))
        && status.contains("no match")
        && !status.contains("Enter arms query")
        && !status.contains(&format!("/{query}"))
}

/// Ignored `notes` is absent from the left tree and is not the cursor.
///
/// Status / crumb `notes` must not count. A shown-ignored `notes` row with
/// trailing `@` and the ignored glyph is the `.` path.
fn hidden_notes_stay_out(screen: &str) -> bool {
    tree_line_containing(screen, "notes").is_none()
        && !tree_has(screen, "notes")
        && !tree_cursor_on(screen, "notes")
        && !screen.contains("@ notes")
        && !screen.contains("workspace › notes")
        && !screen.contains("[notes]")
}

/// Seed repos stay painted, back where `/` found them.
///
/// `/n` / `/no` hit the visible No-updates group, but `notes` has no hit,
/// so the tree returns to the `/` origin: the launch README cursor with
/// No updates folded again. A filter that hides app/merger/README, a jump
/// onto ignored `notes`, or a cursor left on No updates cannot pass.
fn seed_tree_after_notes_query(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && no_updates_group_folded(screen)
        && tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "workspace")
}

/// Typing `/notes` on the tree. Help `/` paints `HELP  /notes`.
fn typing_notes_search_hidden(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    search_prompt_has_no_match_query(screen, "notes")
        && hidden_notes_stay_out(screen)
        && seed_tree_after_notes_query(screen)
        && !screen.contains("MOVE")
        && !screen.contains("HELP  /")
        && !crumb.contains("no match")
}

/// Enter arms `/notes`. SEARCH is gone. `no match` sits on the breadcrumb.
fn armed_notes_search_no_match(screen: &str) -> bool {
    let status = status_row(screen);
    let crumb = crumb_row(screen);
    status.contains("/notes")
        && status.contains("? help")
        && status.contains("focus right")
        && !status.contains("SEARCH")
        && !screen.contains("SEARCH")
        && !screen.contains("Enter arms query")
        && !screen.contains("MOVE")
        && !screen.contains("HELP  /")
        && crumb.contains("no match")
        && hidden_notes_stay_out(screen)
        && seed_tree_after_notes_query(screen)
}

/// `/notes` on a cold-start tree does not reveal hidden ignored `notes`.
///
/// Docs + keymap: `/` search jumps among visible rows (including folded
/// rows already in the tree). Ignored repos stay out until `.`. Daily
/// seed first paint hides `notes`. Typing `/notes` keeps SEARCH on the
/// status row (`notes▏` + the preview's `no match`). Each character
/// applies from the `/` origin: `n` and `no` hit the visible `No updates`
/// group; `notes` itself has no visible hit, so the tree goes back to the
/// launch README cursor with that group folded (vim incsearch). Ignored
/// `@ notes` is never inserted. Enter arms `/notes` with a `no match`
/// breadcrumb toast. A no-op `/`, help search (`HELP  /notes`), `.`
/// show-ignored, a jump onto `notes`, or a filter that hides
/// app/merger/README cannot pass.
#[test]
fn pty_search_does_not_reveal_hidden_ignored() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: ignored notes hidden (`.` has not run)",
        WAIT,
    );

    tui.key('/');
    tui.keys("notes");
    tui.wait_pred(
        typing_notes_search_hidden,
        "/notes types SEARCH notes▏ no match; ignored notes stay off the tree and cursor; no hit puts the cursor back on README (a no-op `/`, help `HELP  /notes`, or `.` `@ notes` cannot pass)",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        armed_notes_search_no_match,
        "Enter arms /notes; SEARCH closes; breadcrumb toasts no match; ignored notes still hidden",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        armed_notes_search_no_match,
        "armed /notes no-match holds (not a flicker or delayed notes reveal)",
        WAIT,
    );
}
