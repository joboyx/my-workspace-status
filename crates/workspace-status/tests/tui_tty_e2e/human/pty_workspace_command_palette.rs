use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, SETTLE_MS, WAIT};

/// Same gap as `pty_shift_left_right_tree_pan`. A same-letter `h`/`j`/`k`/`l`
/// burst is dropped by `discard_held_nav_backlog` after the first press.
const PALETTE_NAV_LETTER_GAP_MS: u64 = 50;

fn palette_open(screen: &str) -> bool {
    screen.contains("Enter run")
}

fn palette_closed(screen: &str) -> bool {
    !screen.contains("Enter run")
}

fn no_pull_toast(screen: &str) -> bool {
    !screen.contains("nothing behind to pull") && !screen.contains("Pulling")
}

/// Package version sits on the help overlay lower-right, same idea as
/// `pty_help_overlay` / headless `assert_help_version`.
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

fn wait_first_paint(tui: &PtySession) {
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        |screen| {
            tree_has(screen, "README.md")
                && tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "workspace")
                && !tree_cursor_on(screen, "app")
                && palette_closed(screen)
        },
        "first paint: tree cursor on README.md; command palette is closed",
        WAIT,
    );
}

fn wait_readme_cursor(tui: &mut PtySession) {
    tui.search("README");
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "app")
                && !tree_cursor_on(screen, "workspace")
        },
        "/README keeps the tree cursor on README.md (a miss jumps to app or workspace)",
        WAIT,
    );
}

/// Type a palette filter at human key gaps for nav letters.
///
/// `keys("full-file")` lands as `ful-file`: the input thread treats the
/// second `l` as held-nav backlog. `pull` can still match `Pull behind`
/// after a dropped `l`; `full-file` cannot.
fn type_palette_filter(tui: &mut PtySession, query: &str) {
    for c in query.chars() {
        tui.key(c);
        if matches!(c, 'h' | 'H' | 'j' | 'J' | 'k' | 'K' | 'l' | 'L') {
            tui.wait_ms(PALETTE_NAV_LETTER_GAP_MS);
        }
    }
}

fn open_ctrl_k_filter(tui: &mut PtySession, query: &str, title: &str) {
    tui.ctrl_letter('k');
    tui.wait_pred(
        |screen| palette_open(screen) && screen.contains("Ctrl-k"),
        "Ctrl-k opens the command palette (a no-op leaves idle chrome without Enter run)",
        WAIT,
    );
    type_palette_filter(tui, query);
    tui.wait_pred(
        |screen| palette_open(screen) && screen.contains(title) && screen.contains(query),
        &format!(
            "palette filter `{query}` shows `{title}` (Enter before the filter lands would run the first catalog row; a dropped nav letter would keep a truncated query)"
        ),
        WAIT,
    );
}

fn enter_keeps_palette_open(tui: &mut PtySession, what: &str) {
    tui.enter();
    tui.wait_pred(palette_open, what, WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        palette_open,
        &format!("{what} (holds; a delayed close would drop Enter run)"),
        WAIT,
    );
}

fn esc_closes_palette(tui: &mut PtySession) {
    tui.esc();
    tui.wait_pred(
        palette_closed,
        "Esc closes the command palette (a stuck overlay keeps Enter run)",
        WAIT,
    );
}

/// Ctrl-k, filter Pull behind, Enter runs pull and closes the palette.
///
/// Docs: Enter closes then dispatches the highlighted enabled command.
/// Daily seed is current, so the toast is `nothing behind to pull` (or a
/// brief `Pulling`). `gg` first so the workspace row is the pull scope. A
/// no-op, a stay-open overlay, or a silent close without that toast is red.
#[test]
fn pty_workspace_palette_filter_pull_enter_closes_palette() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);

    tui.gg();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "workspace")
                && !tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "app")
        },
        "gg moves the tree cursor to the workspace row before opening the palette",
        WAIT,
    );

    open_ctrl_k_filter(&mut tui, "pull", "Pull behind");
    tui.enter();
    tui.wait_pred(
        |screen| {
            palette_closed(screen)
                && (screen.contains("nothing behind to pull") || screen.contains("Pulling"))
        },
        "Enter runs Pull behind and closes the palette (a no-op keeps Enter run or never toasts)",
        GIT_WAIT,
    );
}

/// Ctrl-k, filter Keymap help, Enter closes the palette and paints `?` help.
///
/// MOVE plus the package version in the lower-right is the bar (same idea
/// as `pty_help_overlay`). A no-op, a stay-open palette, or help without
/// the version is red.
#[test]
fn pty_workspace_palette_filter_help_enter_opens_keymap_help() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);

    open_ctrl_k_filter(&mut tui, "help", "Keymap help");
    tui.enter();
    tui.wait_pred(
        |screen| {
            palette_closed(screen) && screen.contains("MOVE") && help_version_lower_right(screen)
        },
        "Enter runs Keymap help: palette chrome gone, MOVE overlay, version lower-right",
        WAIT,
    );
}

/// `:` opens the palette; filter then Esc dismisses and keeps the README cursor.
///
/// Search to README first. Fail if the cursor jumps to `app` or workspace,
/// or if Enter run stays. This is the colon open path (Ctrl-k is the other
/// tests) and the open + filter + Esc dismiss path.
#[test]
fn pty_workspace_palette_colon_esc_keeps_readme_cursor() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);
    wait_readme_cursor(&mut tui);

    tui.key(':');
    tui.wait_pred(
        palette_open,
        ": opens the command palette (a no-op leaves idle chrome without Enter run)",
        WAIT,
    );
    type_palette_filter(&mut tui, "pull");
    tui.wait_pred(
        |screen| palette_open(screen) && screen.contains("Pull behind") && screen.contains("pull"),
        "colon palette filter `pull` shows Pull behind before Esc",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            palette_closed(screen)
                && tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "app")
                && !tree_cursor_on(screen, "workspace")
        },
        "Esc closes the palette and leaves the tree cursor on README.md (not app / workspace)",
        WAIT,
    );
}

/// Pull behind on a file row stays dimmed: Enter keeps the palette open.
///
/// No pull toast. Cursor stays on README. A dispatch that closes the
/// overlay or toasts `nothing behind to pull` / `Pulling` is red.
#[test]
fn pty_workspace_palette_disabled_pull_on_file_keeps_open() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);
    wait_readme_cursor(&mut tui);

    open_ctrl_k_filter(&mut tui, "pull", "Pull behind");
    tui.enter();
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && no_pull_toast(screen)
                && tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "app")
                && !tree_cursor_on(screen, "workspace")
        },
        "Enter on dimmed Pull behind keeps the palette; no pull toast; cursor stays on README",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| {
            palette_open(screen) && no_pull_toast(screen) && tree_cursor_on(screen, "README.md")
        },
        "disabled Pull behind still holds (not a flicker-close or delayed toast)",
        WAIT,
    );
}

/// Push on a file row stays dimmed with `repo / checkout only`.
///
/// Enter keeps the palette and that reason. Absent `nothing to push`.
#[test]
fn pty_workspace_palette_disabled_push_on_file_keeps_open() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);
    wait_readme_cursor(&mut tui);

    open_ctrl_k_filter(&mut tui, "push", "Push");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("Push")
                && screen.contains("repo / checkout only")
        },
        "filtered Push on a file row shows repo / checkout only",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("repo / checkout only")
                && !screen.contains("nothing to push")
                && tree_cursor_on(screen, "README.md")
        },
        "Enter on dimmed Push keeps the palette and reason; no nothing to push toast",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("repo / checkout only")
                && !screen.contains("nothing to push")
        },
        "disabled Push still holds (not a flicker-close or delayed toast)",
        WAIT,
    );
}

/// View gates stay dimmed: focus branches / highlight on a file; full-file
/// / reviewed on the workspace row. Enter keeps `Enter run` each time.
///
/// Esc between filters. Right pane on the workspace row is not a file
/// diff, so Full-file context must not run.
#[test]
fn pty_workspace_palette_disabled_view_gates_keep_open() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);
    wait_readme_cursor(&mut tui);

    open_ctrl_k_filter(&mut tui, "focus branches", "Graph focus branches");
    enter_keeps_palette_open(
        &mut tui,
        "Enter on Graph focus branches on a file row keeps the palette",
    );
    esc_closes_palette(&mut tui);

    open_ctrl_k_filter(&mut tui, "highlight", "Highlight diff lines");
    enter_keeps_palette_open(
        &mut tui,
        "Enter on Highlight diff lines on a file row keeps the palette",
    );
    esc_closes_palette(&mut tui);

    tui.gg();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "workspace")
                && !tree_cursor_on(screen, "README.md")
                && palette_closed(screen)
        },
        "gg moves to the workspace row with the palette closed",
        WAIT,
    );

    open_ctrl_k_filter(&mut tui, "full-file", "Full-file context");
    enter_keeps_palette_open(
        &mut tui,
        "Enter on Full-file context on the workspace row keeps the palette",
    );
    esc_closes_palette(&mut tui);

    open_ctrl_k_filter(&mut tui, "reviewed", "Mark reviewed");
    enter_keeps_palette_open(
        &mut tui,
        "Enter on Mark reviewed on the workspace row keeps the palette",
    );
}

/// Revert from the palette on README closes the overlay and arms boxed confirm.
///
/// Strong oracle: `Revert README.md?`. Do not confirm `y`. A stay-open
/// palette (`Enter run`) or a silent revert is red.
#[test]
fn pty_workspace_palette_revert_opens_boxed_confirm() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);
    wait_readme_cursor(&mut tui);

    open_ctrl_k_filter(&mut tui, "revert", "Revert");
    tui.enter();
    tui.wait_pred(
        |screen| {
            palette_closed(screen)
                && screen.contains("Revert README.md?")
                && screen.contains("cancel")
                && tree_cursor_on(screen, "README.md")
                && !screen.contains("reverted")
        },
        "Enter on Revert closes the palette and paints Revert README.md? (do not confirm y)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| {
            palette_closed(screen)
                && screen.contains("Revert README.md?")
                && !screen.contains("reverted")
        },
        "revert confirm holds (not a flicker, y path, or palette return)",
        WAIT,
    );
}

/// Daily keys stay themselves: `?` help, `/` pane search, `p` / Shift+P do
/// not open the palette.
///
/// Fail if `Enter run` appears. CSI-u Shift+P is the live Shift+letter
/// path, not a raw `P` byte.
#[test]
fn pty_workspace_palette_does_not_steal_daily_keys() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);

    tui.key('?');
    tui.wait_pred(
        |screen| screen.contains("MOVE") && palette_closed(screen),
        "? opens keymap help (MOVE), not the command palette (Enter run)",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("MOVE") && palette_closed(screen),
        "Esc closes help so / is pane search, not help search or palette",
        WAIT,
    );

    tui.key('/');
    tui.wait_pred(
        |screen| screen.contains("SEARCH") && palette_closed(screen),
        "/ opens pane search (SEARCH), not the command palette",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("SEARCH") && palette_closed(screen),
        "Esc closes pane search before bare p",
        WAIT,
    );

    tui.key('p');
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        palette_closed,
        "bare p does not open the command palette",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        palette_closed,
        "bare p still does not open the command palette",
        WAIT,
    );

    tui.shift_letter('p');
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        palette_closed,
        "CSI-u Shift+P does not open the command palette",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        palette_closed,
        "CSI-u Shift+P still does not open the command palette",
        WAIT,
    );
}
