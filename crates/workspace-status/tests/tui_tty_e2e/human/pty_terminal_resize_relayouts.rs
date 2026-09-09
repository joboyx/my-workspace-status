use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{pane_top, title_has_diff, tree_has, TREE_LABEL_COL, WAIT};

/// 0-based column of the tree/right box join (`┐┌` / `││` / `┘└`).
fn pane_join_col(screen: &str) -> Option<u16> {
    let line = screen.lines().next()?;
    for sep in ["┐┌", "││", "┘└"] {
        if let Some(byte_at) = line.find(sep) {
            return Some(line[..byte_at].chars().count() as u16);
        }
    }
    screen.lines().skip(1).find_map(|line| {
        line.find("││")
            .map(|byte_at| line[..byte_at].chars().count() as u16)
    })
}

fn join_line_count(screen: &str) -> usize {
    screen.lines().filter(|line| line.contains("││")).count()
}

fn has_pane_join_pair(screen: &str) -> bool {
    screen.contains("││") || screen.contains("┐┌")
}

fn has_bottom_corners(screen: &str) -> bool {
    screen.contains("┘└")
}

fn join_is_a_split(screen: &str, join: u16) -> bool {
    join > TREE_LABEL_COL
        && has_pane_join_pair(screen)
        && pane_top(screen).contains("tree")
        && title_has_diff(screen)
}

/// Two-pane idle chrome on a live PTY. Not the default-size launch oracle.
fn two_pane_idle(screen: &str) -> bool {
    let Some(join) = pane_join_col(screen) else {
        return false;
    };
    tree_has(screen, "README.md")
        && screen.contains("? help")
        && !screen.contains("MOVE")
        && !screen.contains("/ search help")
        && has_pane_join_pair(screen)
        && join_is_a_split(screen, join)
}

fn panes_narrower(screen: &str, join_wide: u16) -> bool {
    let Some(join) = pane_join_col(screen) else {
        return false;
    };
    two_pane_idle(screen) && join < join_wide && join_is_a_split(screen, join)
}

fn list_shorter(screen: &str, join_lines_tall: usize) -> bool {
    two_pane_idle(screen) && join_line_count(screen) < join_lines_tall && has_bottom_corners(screen)
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

fn help_overlay_idle(screen: &str) -> bool {
    screen.contains("MOVE")
        && screen.contains("/ search help")
        && !screen.contains("HELP  /")
        && help_version_lower_right(screen)
}

fn help_keeps_overlay(screen: &str, help_wide_joins: usize) -> bool {
    help_overlay_idle(screen) && (help_wide_joins < 2 || join_line_count(screen) < help_wide_joins)
}

/// Terminal resize relayouts pane split, list height, and the help overlay.
///
/// `PtySession::resize` changes the PTY winsize. The live loop must pick
/// that up on poll timeout (`TIOCGWINSZ`) with no dummy key. Shift-pan
/// and tree hscroll stay on `pty_shift_left_right_tree_pan` and
/// `pty_tree_sgr_hscroll_pans_clipped_path`. Help keymap rows stay on
/// `pty_help_overlay`. Headless `pane_tree_width` stays on
/// `terminal_resize_relayouts_panes_gutter_help_and_lists`.
///
/// Fail if resize is a no-op: the wide join stays or vanishes under a
/// parser clip, `? help` / `┘└` drop off a short grid, or a narrow help
/// frame loses MOVE / the package version.
#[test]
fn pty_terminal_resize_relayouts() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open_size(&workspace, 200, 40);
    tui.wait_pred(
        two_pane_idle,
        "wide first paint: README, ? help, and a tree/right join",
        WAIT,
    );
    let join_wide = pane_join_col(&tui.screen())
        .unwrap_or_else(|| panic!("tree/right join on the wide first paint:\n{}", tui.screen()));
    assert!(
        join_line_count(&tui.screen()) > 8 && has_bottom_corners(&tui.screen()),
        "wide frame has a tall inner pane and ┘└:\n{}",
        tui.screen()
    );
    assert!(
        !panes_narrower(&tui.screen(), join_wide),
        "narrow-width claim must be false before resize (join still {join_wide}):\n{}",
        tui.screen()
    );

    tui.resize(80, 40);
    tui.wait_pred(
        |screen| panes_narrower(screen, join_wide),
        "80-col resize moves the join left (a no-op keeps the wide join or clips ┐┌ away)",
        WAIT,
    );

    let join_lines_tall = join_line_count(&tui.screen());
    assert!(
        join_lines_tall > 8 && has_bottom_corners(&tui.screen()),
        "80x40 still has a tall inner pane before the height shrink:\n{}",
        tui.screen()
    );
    assert!(
        !list_shorter(&tui.screen(), join_lines_tall),
        "short-height claim must be false before the 16-row resize:\n{}",
        tui.screen()
    );

    tui.resize(80, 16);
    tui.wait_pred(
        |screen| list_shorter(screen, join_lines_tall),
        "16-row resize keeps ? help and ┘└ with fewer ││ lines (parser clip drops the tall footer)",
        WAIT,
    );

    tui.resize(200, 48);
    tui.wait_pred(
        two_pane_idle,
        "200x48 restore: idle two-pane chrome before help (no dummy key after resize)",
        WAIT,
    );
    tui.key('?');
    tui.wait_pred(
        help_overlay_idle,
        "help overlay is open with MOVE, idle / search help, and the package version",
        WAIT,
    );
    let help_wide_joins = join_line_count(&tui.screen());
    if help_wide_joins >= 2 {
        assert!(
            !help_keeps_overlay(&tui.screen(), help_wide_joins),
            "narrow-help wrap claim must be false before resize:\n{}",
            tui.screen()
        );
    }

    tui.resize(80, 48);
    tui.wait_pred(
        |screen| help_keeps_overlay(screen, help_wide_joins),
        "narrow help keeps MOVE and the lower-right version (parser clip drops the overlay version)",
        WAIT,
    );
}
