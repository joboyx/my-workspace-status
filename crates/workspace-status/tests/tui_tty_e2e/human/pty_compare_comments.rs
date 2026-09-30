use std::fs;
use std::path::{Path, PathBuf};

use crate::harness::PtySession;
use crate::seed::{compare_regions_workspace, git_stdout};
use crate::support::{
    compare_regions_diff_focused, open_compare_regions_diff, open_compare_regions_in,
    panes_tree_focused_diff_unfocused, panes_tree_unfocused_diff_focused, GIT_WAIT, SETTLE_MS,
    VISUAL_KEY_GAP_MS, WAIT,
};

const LINE_BODY: &str = "compare-line-note";
const RANGE_BODY: &str = "compare-range-note";
const WORKSPACE_BODY: &str = "workspace-line-note";
/// Worktree edit of `keep-b` (line 2) for the Workspace comment scenario.
const DIRTY_B: &str = "wt-dirty-b";

fn comment_store(workspace: &Path) -> PathBuf {
    workspace.join(".e2e-state").join("comments.json")
}

fn store_text(workspace: &Path) -> String {
    fs::read_to_string(comment_store(workspace)).unwrap_or_default()
}

fn head_sha(workspace: &Path) -> String {
    git_stdout(&workspace.join("app"), &["rev-parse", "HEAD"])
}

fn comment_overlay(screen: &str) -> bool {
    screen.contains("Comment")
        && screen.contains("body:")
        && screen.contains("Enter save")
        && screen.contains("empty deletes")
        && !screen.contains("# Comments")
}

fn overlay_closed(screen: &str) -> bool {
    !screen.contains("Enter save") && !screen.contains("empty deletes")
}

fn export_overlay(screen: &str) -> bool {
    screen.contains("# Comments") && screen.contains("copied · Esc close")
}

/// The diff row that shows `needle` (and nothing longer, e.g. keep-a vs
/// keep-a-dirty) carries `mark`.
fn diff_row_has(screen: &str, needle: &str, mark: char) -> bool {
    diff_row(screen, needle).is_some_and(|line| line.contains(mark))
}

fn diff_row<'a>(screen: &'a str, needle: &str) -> Option<&'a str> {
    screen.lines().find(|line| {
        line.match_indices(needle).any(|(at, _)| {
            let rest = &line[at + needle.len()..];
            !rest.starts_with(|c: char| c.is_alphanumeric() || c == '-')
        })
    })
}

/// The diff cursor bar (`▌`) sits on the row that shows `needle`.
fn diff_cursor_on(screen: &str, needle: &str) -> bool {
    diff_row(screen, needle).is_some_and(|line| line.contains('▌'))
}

/// Plain `j` presses with the human gap the held-nav backlog needs.
fn press_j(tui: &mut PtySession, times: usize) {
    for _ in 0..times {
        tui.letter_press('j');
        tui.wait_ms(VISUAL_KEY_GAP_MS);
    }
}

/// `j` until the diff cursor sits on `needle`.
fn move_diff_cursor_to(tui: &mut PtySession, needle: &str) {
    for _ in 0..12 {
        if diff_cursor_on(&tui.screen(), needle) {
            return;
        }
        press_j(tui, 1);
        tui.wait_ms(SETTLE_MS);
    }
    tui.wait_pred(
        |screen| diff_cursor_on(screen, needle),
        "j moves the diff cursor to the target row",
        WAIT,
    );
}

/// Type a comment body; `j`/`k`/`h`/`l` need the held-nav gap.
fn type_body(tui: &mut PtySession, body: &str) {
    for c in body.chars() {
        tui.key(c);
        if matches!(c, 'h' | 'j' | 'k' | 'l') {
            tui.wait_ms(VISUAL_KEY_GAP_MS);
        }
    }
    tui.wait_pred(
        |screen| comment_overlay(screen) && screen.contains(body),
        "the typed body shows in the Comment overlay",
        WAIT,
    );
}

/// `;` on one compare line saves a commit-line comment keyed to the head.
///
/// Two `j` presses put the cursor on keep-a (new line 1). `;` opens Comment
/// on `regions.txt:1` of the head commit; Enter saves it and paints `"` on
/// keep-a. `;` again reopens it: Ctrl-R marks it resolved and the mark
/// turns to `'`. `y` copies the compare comment with the head SHA, path,
/// and line.
#[test]
fn pty_compare_semicolon_comments_line_resolves_and_exports() {
    let (_root, workspace) = compare_regions_workspace();
    let head = head_sha(&workspace);
    let mut tui = open_compare_regions_diff(&workspace);
    press_j(&mut tui, 2);
    tui.wait_pred(
        |screen| diff_cursor_on(screen, "keep-a"),
        "two j presses move the compare diff cursor to keep-a",
        WAIT,
    );

    tui.key(';');
    tui.wait_pred(
        |screen| {
            comment_overlay(screen)
                && screen.contains("regions.txt:1")
                && screen.contains(&format!("commit {}", &head[..7]))
                && !screen.contains("Switch to Workspace tab")
        },
        "; opens Comment on regions.txt:1 of the compare head",
        WAIT,
    );
    type_body(&mut tui, LINE_BODY);
    tui.enter();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && compare_regions_diff_focused(screen)
                && screen.contains("comment saved")
                && diff_row_has(screen, "keep-a", '"')
                && !diff_row_has(screen, "keep-b", '"')
        },
        "Enter saves: \" on keep-a only",
        GIT_WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.contains(LINE_BODY)
            && stored.contains("commitLine")
            && stored.contains(&format!("\"sha\": \"{head}\""))
            && stored.contains("\"path\": \"regions.txt\"")
            && stored.contains("\"line\": 1")
            && !stored.contains("endLine"),
        "store must key the comment to the compare head, regions.txt:1:\n{stored}"
    );

    tui.key(';');
    tui.wait_pred(
        |screen| {
            comment_overlay(screen)
                && screen.contains(LINE_BODY)
                && screen.contains("Ctrl-R resolve")
                && !screen.contains("Comment · resolved")
        },
        "; on keep-a reopens the saved comment",
        WAIT,
    );
    tui.ctrl_letter('r');
    tui.wait_pred(
        |screen| comment_overlay(screen) && screen.contains("Comment · resolved"),
        "Ctrl-R marks the overlay resolved",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && compare_regions_diff_focused(screen)
                && diff_row_has(screen, "keep-a", '\'')
                && !diff_row_has(screen, "keep-a", '"')
        },
        "Enter saves resolved: keep-a shows the ' mark",
        GIT_WAIT,
    );
    assert!(
        store_text(&workspace).contains("\"resolved\": true"),
        "store must keep the resolved flag:\n{}",
        store_text(&workspace)
    );

    tui.key('y');
    tui.wait_pred(export_overlay, "y opens the copied export overlay", WAIT);
    tui.wait_clipboard_pred(
        |payloads| {
            payloads.iter().any(|text| {
                text.contains(&format!("commit `{head}`"))
                    && text.contains("`regions.txt`:1")
                    && text.contains(LINE_BODY)
                    && text.contains("[resolved]")
            })
        },
        "OSC 52 payload names the compare head, regions.txt:1, and the body",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| compare_regions_diff_focused(screen) && !export_overlay(screen),
        "Esc closes the export overlay on the compare diff",
        WAIT,
    );
}

/// `V`, `j`, `j`, `;` on a compare diff comments lines 1-3 of the head.
///
/// `y` then copies that range.
#[test]
fn pty_compare_highlight_semicolon_comments_range() {
    let (_root, workspace) = compare_regions_workspace();
    let head = head_sha(&workspace);
    let mut tui = open_compare_regions_diff(&workspace);
    press_j(&mut tui, 2);
    tui.wait_pred(
        |screen| diff_cursor_on(screen, "keep-a"),
        "two j presses move the compare diff cursor to keep-a",
        WAIT,
    );
    tui.shift_letter('V');
    tui.wait_pred(
        |screen| screen.contains("VISUAL"),
        "Shift+V starts the highlight",
        WAIT,
    );
    tui.wait_ms(VISUAL_KEY_GAP_MS);
    press_j(&mut tui, 2);
    tui.wait_pred(
        |screen| screen.contains("VISUAL") && diff_cursor_on(screen, "keep-c"),
        "j extends the highlight to keep-c",
        WAIT,
    );

    tui.key(';');
    tui.wait_pred(
        |screen| comment_overlay(screen) && screen.contains("regions.txt:1-3"),
        "; opens Comment on regions.txt:1-3",
        WAIT,
    );
    type_body(&mut tui, RANGE_BODY);
    tui.enter();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && compare_regions_diff_focused(screen)
                && !screen.contains("VISUAL")
                && ["keep-a", "keep-b", "keep-c"]
                    .iter()
                    .all(|needle| diff_row_has(screen, needle, '"'))
                && !diff_row_has(screen, "keep-d", '"')
        },
        "Enter saves: \" on keep-a through keep-c only, highlight gone",
        GIT_WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.contains(RANGE_BODY)
            && stored.contains("commitLine")
            && stored.contains(&format!("\"sha\": \"{head}\""))
            && stored.contains("\"line\": 1")
            && stored.contains("\"endLine\": 3"),
        "store must keep a compare range 1-3 on the head:\n{stored}"
    );

    tui.key('y');
    tui.wait_clipboard_pred(
        |payloads| {
            payloads.iter().any(|text| {
                text.contains(&format!("commit `{head}`"))
                    && text.contains("`regions.txt`:1-3")
                    && text.contains(RANGE_BODY)
            })
        },
        "OSC 52 payload names regions.txt:1-3 on the compare head",
        WAIT,
    );
}

/// A Workspace comment on a dirty line does not paint on the compare diff
/// and is not in its export.
///
/// The worktree changes keep-b (line 2). A Workspace `;` on that line
/// paints `"` there. With the tree still on regions.txt, "Diff vs default"
/// opens the compare tab: it shows the committed keep-b on line 2 of the
/// same checkout and path, which must stay unmarked, and `y` on the
/// compare diff must not copy the Workspace comment.
#[test]
fn pty_compare_hides_workspace_line_comment() {
    let (_root, workspace) = compare_regions_workspace();
    let regions = workspace.join("app").join("regions.txt");
    let committed = fs::read_to_string(&regions).unwrap();
    fs::write(
        &regions,
        committed.replace("keep-b\n", &format!("{DIRTY_B}\n")),
    )
    .unwrap();

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.search("regions.txt");
    tui.wait_pred(
        |screen| screen.contains("UNSTAGED") && screen.contains(DIRTY_B),
        "search focuses the dirty regions.txt worktree diff",
        GIT_WAIT,
    );
    tui.tab();
    tui.wait_pred(
        |screen| panes_tree_unfocused_diff_focused(screen) && screen.contains(DIRTY_B),
        "Tab focuses the Workspace diff",
        WAIT,
    );
    move_diff_cursor_to(&mut tui, DIRTY_B);
    tui.key(';');
    tui.wait_pred(
        |screen| comment_overlay(screen) && screen.contains("regions.txt:2"),
        "; opens Comment on the Workspace line 2",
        WAIT,
    );
    type_body(&mut tui, WORKSPACE_BODY);
    tui.enter();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && screen.contains("comment saved")
                && diff_row_has(screen, DIRTY_B, '"')
        },
        "Enter saves: \" on the Workspace line",
        GIT_WAIT,
    );
    assert!(
        store_text(&workspace).contains("worktreeLine"),
        "{}",
        store_text(&workspace)
    );

    // The tree cursor stays on regions.txt, so the parked Workspace diff is
    // the same checkout and path as the compare diff.
    tui.tab();
    tui.wait_pred(
        |screen| panes_tree_focused_diff_unfocused(screen) && diff_row_has(screen, DIRTY_B, '"'),
        "Tab returns focus to the Workspace tree on regions.txt",
        WAIT,
    );
    open_compare_regions_in(&mut tui);
    tui.wait_ms(SETTLE_MS);
    let screen = tui.screen();
    assert!(
        diff_row(&screen, "keep-b").is_some(),
        "compare diff shows keep-b:\n{screen}"
    );
    assert!(
        !diff_row_has(&screen, "keep-b", '"') && !screen.contains(DIRTY_B),
        "the Workspace comment must not paint on the compare diff:\n{screen}"
    );

    tui.key('y');
    tui.wait_pred(export_overlay, "y opens the export overlay", WAIT);
    tui.wait_clipboard_pred(
        |payloads| !payloads.is_empty(),
        "y copies the compare scope",
        WAIT,
    );
    let payloads = tui.clipboard_payloads();
    assert!(
        payloads.iter().all(|text| !text.contains(WORKSPACE_BODY)),
        "compare export must not copy the Workspace comment: {payloads:?}"
    );
}
