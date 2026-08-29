use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{daily_workspace, git};
use crate::support::{right_pane, tree_cursor_on, tree_has, GIT_WAIT, SETTLE_MS, WAIT};

const FILE: &str = "wide-hunk.rs";
const HEAD: &str = "WIDE-FILE-HEAD-00";
const NEAR_ABOVE: &str = "WIDE-CTX-ABOVE";
const HUNK: &str = "WIDE-HUNK-NEEDLE";
const HUNK_OLD: &str = "WIDE-HUNK-BASE";
const NEAR_BELOW: &str = "WIDE-CTX-BELOW";
const TAIL: &str = "WIDE-FILE-TAIL-39";
const HUNK_ONLY_HEADER: &str = "@@ -46,7 +46,7 @@";
const FULL_FILE_HEADER: &str = "@@ -1,97 +1,97 @@";

fn seed_wide_hunk_tracked(workspace: &Path) {
    let app = workspace.join("app");
    let mut committed = String::new();
    for i in 0..40 {
        committed.push_str(&format!("WIDE-FILE-HEAD-{i:02}\n"));
    }
    committed.push_str(&format!("{NEAR_ABOVE}\n"));
    for i in 0..7 {
        committed.push_str(&format!("WIDE-PAD-A{i}\n"));
    }
    committed.push_str(&format!("{HUNK_OLD}\n"));
    for i in 0..7 {
        committed.push_str(&format!("WIDE-PAD-B{i}\n"));
    }
    committed.push_str(&format!("{NEAR_BELOW}\n"));
    for i in 0..40 {
        committed.push_str(&format!("WIDE-FILE-TAIL-{i:02}\n"));
    }
    fs::write(app.join(FILE), &committed).unwrap();
    git(&app, &["add", FILE]);
    git(&app, &["commit", "-q", "-m", "wide-hunk tracked"]);
    fs::write(app.join(FILE), committed.replace(HUNK_OLD, HUNK)).unwrap();
}

fn help_lists_ctrl_o_keep_hunk(screen: &str) -> bool {
    let compact = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.contains("VIEW")
        && compact.contains("Ctrl-o")
        && compact.contains("full-file")
        && compact.contains("keep hunk in view")
        && compact.contains("open in editor")
}

fn diff_header_full(screen: &str) -> bool {
    right_pane(screen)
        .lines()
        .next()
        .is_some_and(|line| line.contains(" · full"))
}

fn hunk_only_file_diff(screen: &str) -> bool {
    let right = right_pane(screen);
    tree_cursor_on(screen, FILE)
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "app")
        && screen.contains("UNSTAGED")
        && right.contains(HUNK_ONLY_HEADER)
        && right.contains(HUNK)
        && right.contains(HUNK_OLD)
        && !right.contains(NEAR_ABOVE)
        && !right.contains(NEAR_BELOW)
        && !right.contains(HEAD)
        && !right.contains(TAIL)
        && !right.contains(FULL_FILE_HEADER)
        && !diff_header_full(screen)
        && screen.contains("ctrl+o")
        && screen.contains("full file")
        && !screen.contains("WIP on graph")
        && !screen.contains("MOVE")
}

fn full_file_keeps_hunk(screen: &str) -> bool {
    let right = right_pane(screen);
    tree_cursor_on(screen, FILE)
        && screen.contains("UNSTAGED")
        && right.contains(FULL_FILE_HEADER)
        && right.contains(HUNK)
        && right.contains(HUNK_OLD)
        && right.contains(NEAR_ABOVE)
        && right.contains(NEAR_BELOW)
        && !right.contains(HEAD)
        && !right.contains(TAIL)
        && !right.contains(HUNK_ONLY_HEADER)
        && diff_header_full(screen)
        && !screen.contains("WIP on graph")
        && !screen.contains("MOVE")
}

fn merger_graph_no_full_file(screen: &str) -> bool {
    let right = right_pane(screen);
    tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, FILE)
        && screen.contains("WIP on graph")
        && right.contains("working tree clean")
        && !right.contains(HUNK)
        && !right.contains(FULL_FILE_HEADER)
        && !right.contains("UNSTAGED")
        && !diff_header_full(screen)
        && !screen.contains("MOVE")
}

/// Help VIEW: Ctrl-o is full-file and keeps the hunk in view.
///
/// Documented result: on a focused file-diff, Ctrl-o reloads with unlimited
/// unified context so body that default `-U3` hid is visible, and the current
/// hunk stays on screen. A second Ctrl-o restores hunk-only. Off a file-diff
/// (graph / repo row) the key refuses. Encoding: CSI-u Control+o
/// (`CSI 111 ; 5 : 1 u` press, `: 3` release). C0 `\x0f` (`PtySession::ctrl`)
/// is a different path. A live PTY hunt after first paint used CSI-u.
///
/// The daily README is too small: the hunk is the whole file. This claim
/// seeds a tracked dirty file whose default hunk is mid-file. Fail if
/// nothing happens, if only the ` · full` header suffix flips, if the pane
/// jumps to the file start (hunk gone), or if a graph row also expands.
#[test]
fn pty_ctrl_o_full_file_context() {
    let (_root, workspace) = daily_workspace();
    seed_wide_hunk_tracked(&workspace);

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && tree_has(screen, FILE)
                && !tree_cursor_on(screen, FILE)
                && !screen.contains(HUNK)
                && screen.contains("? help")
        },
        "launch cursor is README; wide-hunk is on the tree",
        GIT_WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        |screen| help_lists_ctrl_o_keep_hunk(screen) && screen.contains("MOVE"),
        "help VIEW lists Ctrl-o as full-file · keep hunk in view",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("keep hunk in view")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("? help")
        },
        "Esc closes help so Ctrl-o is the full-file key",
        WAIT,
    );

    tui.search("wide-hunk");
    tui.wait_pred(
        hunk_only_file_diff,
        "focused tracked file-diff is hunk-only: change visible, far context hidden",
        GIT_WAIT,
    );

    tui.ctrl_letter('o');
    tui.wait_pred(
        full_file_keeps_hunk,
        "CSI-u Ctrl-o expands past ±3 context and keeps the hunk in view (a no-op stays hunk-only; jump-to-top shows HEAD-00 and drops the hunk)",
        GIT_WAIT,
    );

    tui.ctrl_letter('o');
    tui.wait_pred(
        hunk_only_file_diff,
        "second CSI-u Ctrl-o restores hunk-only (documented toggle; one-way would keep · full and far context)",
        GIT_WAIT,
    );

    tui.search("merger");
    tui.wait_pred(
        merger_graph_no_full_file,
        "merger graph row is not a file-diff",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    let before = tui.screen();
    tui.ctrl_letter('o');
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| merger_graph_no_full_file(screen) && screen == before,
        "CSI-u Ctrl-o on a graph row refuses: no full-file marker, no layout change",
        WAIT,
    );
}
