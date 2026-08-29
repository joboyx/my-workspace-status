use crate::harness::{PtySession, COLS};
use crate::seed::daily_workspace;
use crate::support::WAIT;

/// Compact painted help text so wrapped description fragments rejoin.
fn help_compact(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Documented `?` overlay rows (`tui/help.rs` MOVE / GIT / VIEW).
///
/// Independent of `HELP_GROUPS` so a wrong overlay still fails.
const HELP_MOVE_ROWS: &[(&str, &str)] = &[
    ("j k", "down / up"),
    ("h l", "fold · pan lists/diff · Shift+←→ tree"),
    ("z", "toggle fold (instant; no-op on graph/diff)"),
    ("zz", "toggle subtree (no-op on graph/diff)"),
    ("gg G", "top / bottom of focused pane"),
    ("Home End", "top / bottom"),
    ("/", "search focused pane (Enter arms)"),
    ("n N", "next / prev match (after Enter)"),
];

const HELP_GIT_ROWS: &[(&str, &str)] = &[
    ("s", "stage scope"),
    ("S", "stash menu"),
    ("u", "unstage scope"),
    ("x", "revert (y/Y)"),
    ("e", "open in editor"),
    ("space", "mark dirty file reviewed (eye)"),
    ("f", "fetch remotes"),
    ("p", "pull behind"),
    ("P", "push ahead/diverged/new"),
    ("d", "default branch"),
    ("b", "depth 0 picker · graph local/origin/*"),
    ("m", "graph merge into HEAD"),
    ("C", "create (in picker)"),
    ("W", "remove linked worktree"),
    ("r", "refresh now"),
    ("a p D", "focused stash apply/pop/drop"),
];

const HELP_VIEW_ROWS: &[(&str, &str)] = &[
    ("i", "inline / split"),
    ("t", "flat / tree"),
    (".", "show / hide ignored repos"),
    ("T", "cycle theme"),
    ("Ctrl-o", "full-file · keep hunk in view"),
    ("o O", "graph focus branches / clear"),
    ("PgUp PgDn", "page focused pane"),
    ("Ctrl-u Ctrl-d", "page focused ±5"),
    ("m", "mouse · drag pane, split, or graph scrollbars"),
    (";", "comment focused row / line"),
    ("y", "copy comments as markdown"),
    ("Esc", "back / unfocus · never quit"),
    ("Enter dblclick", "focus right / drill"),
    ("?", "this help"),
    ("Tab", "other pane"),
    ("q", "quit"),
    ("Ctrl-C Ctrl-C", "quit (press twice)"),
];

/// Split the painted overlay into MOVE / GIT / VIEW columns.
///
/// Inner width and column width follow `help_inner_width` /
/// `help_column_width` at the PTY default. Footer is excluded so
/// `/ search help` does not leak into the keymap columns.
fn help_group_columns(screen: &str) -> Option<[String; 3]> {
    let lines: Vec<&str> = screen.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.contains("MOVE") && line.contains("GIT") && line.contains("VIEW"))?;
    let inner_w = (COLS as usize).saturating_sub(4);
    let col_w = inner_w / 3;
    let mut cols = [String::new(), String::new(), String::new()];
    for line in &lines[start..] {
        if line.contains("/ search help") || line.contains("Esc closes") {
            break;
        }
        let chars: Vec<char> = line.chars().collect();
        if chars.len() < 2 + col_w {
            continue;
        }
        let inner: Vec<char> = chars.into_iter().skip(2).take(inner_w).collect();
        for (idx, col) in cols.iter_mut().enumerate() {
            let from = idx * col_w;
            let to = (from + col_w).min(inner.len());
            if from < inner.len() {
                col.extend(inner[from..to].iter());
                col.push('\n');
            }
        }
    }
    Some(cols)
}

fn help_column_has_row(column: &str, keys: &str, desc: &str) -> bool {
    let compact = help_compact(column);
    compact.contains(&help_compact(keys)) && compact.contains(&help_compact(desc))
}

fn help_version_lower_right(screen: &str) -> bool {
    let version = workspace_status::APP_VERSION;
    let Some(line) = screen.lines().rev().find(|line| line.contains(version)) else {
        return false;
    };
    let Some(idx) = line.rfind(version) else {
        return false;
    };
    line[idx + version.len()..]
        .chars()
        .all(|c| c.is_whitespace() || matches!(c, '│' | '╯' | '╮' | '┘' | '┐' | '║' | '┤'))
}

/// Full documented overlay: groups, key rows, footer, version.
///
/// A no-op (`? help` chrome only), MOVE/GIT/VIEW titles without rows, or
/// a clipped last GIT wrap must fail.
fn documented_help_overlay(screen: &str) -> bool {
    let Some(header) = screen
        .lines()
        .find(|line| line.contains("MOVE") && line.contains("GIT") && line.contains("VIEW"))
    else {
        return false;
    };
    let move_at = match header.find("MOVE") {
        Some(idx) => idx,
        None => return false,
    };
    let git_at = match header.find("GIT") {
        Some(idx) => idx,
        None => return false,
    };
    let view_at = match header.find("VIEW") {
        Some(idx) => idx,
        None => return false,
    };
    if !(move_at < git_at && git_at < view_at) {
        return false;
    }
    let Some([move_col, git_col, view_col]) = help_group_columns(screen) else {
        return false;
    };
    let move_c = help_compact(&move_col);
    let git_c = help_compact(&git_col);
    let view_c = help_compact(&view_col);
    if !move_c.contains("MOVE") || move_c.contains("GIT") || move_c.contains("VIEW") {
        return false;
    }
    if !git_c.contains("GIT") || git_c.contains("MOVE") || git_c.contains("VIEW") {
        return false;
    }
    if !view_c.contains("VIEW") || view_c.contains("MOVE") || view_c.contains("GIT") {
        return false;
    }
    if HELP_MOVE_ROWS
        .iter()
        .any(|(keys, desc)| !help_column_has_row(&move_col, keys, desc))
        || HELP_GIT_ROWS
            .iter()
            .any(|(keys, desc)| !help_column_has_row(&git_col, keys, desc))
        || HELP_VIEW_ROWS
            .iter()
            .any(|(keys, desc)| !help_column_has_row(&view_col, keys, desc))
    {
        return false;
    }
    // Group membership: page keys stay VIEW; git writes stay GIT.
    if move_c.contains("page focused pane")
        || move_c.contains("stage scope")
        || move_c.contains("cycle theme")
        || git_c.contains("down / up")
        || git_c.contains("cycle theme")
        || view_c.contains("stage scope")
        || view_c.contains("down / up")
        || view_c.contains("open in editor")
    {
        return false;
    }
    screen.contains("/ search help")
        && screen.contains("Esc closes")
        && !screen.contains("? help")
        && help_version_lower_right(screen)
}

/// `?` paints the documented MOVE / GIT / VIEW overlay on a live TTY.
///
/// Fail if `?` is a no-op, if the groups are wrong, or if a documented
/// key row is missing (including a clipped last GIT wrap). Help `/`
/// search and Enter-arm stay on `pty_help_enter_does_not_arm_pane_search`.
#[test]
fn pty_help_overlay() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            screen.contains("? help")
                && screen.contains("README.md")
                && !screen.contains("MOVE")
                && !screen.contains("open in editor")
                && !screen.contains("/ search help")
        },
        "idle chrome shows ? help and the overlay is closed",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        documented_help_overlay,
        "documented MOVE / GIT / VIEW overlay (groups, key rows, footer, version)",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("MOVE")
                && !screen.contains("open in editor")
                && !screen.contains("/ search help")
                && screen.contains("? help")
                && screen.contains("README.md")
        },
        "Esc closes help and restores idle ? help chrome",
        WAIT,
    );
}
