use crate::harness::PtySession;
use crate::seed::{topic_and_unborn_workspace, worktree_workspace};
use crate::support::{tree_cursor_on, tree_has, SETTLE_MS, WAIT};

const PRIMARY_BRANCH: &str = "feature/primary-open";
const LINKED_BRANCH: &str = "feature/linked-open";

/// Same gap as `pty_workspace_command_palette`. A same-letter `h`/`j`/`k`/`l`
/// burst is dropped by `discard_held_nav_backlog` after the first press.
const PALETTE_NAV_LETTER_GAP_MS: u64 = 50;

fn palette_open(screen: &str) -> bool {
    screen.contains("Enter run")
}

fn palette_closed(screen: &str) -> bool {
    !screen.contains("Enter run")
}

fn workspace_only(screen: &str) -> bool {
    screen.contains("# workspace") && !screen.contains("· vs")
}

fn family_visible(screen: &str) -> bool {
    tree_has(screen, "app") && tree_has(screen, PRIMARY_BRANCH) && tree_has(screen, LINKED_BRANCH)
}

fn family_row_focused(screen: &str) -> bool {
    tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, PRIMARY_BRANCH)
        && !tree_cursor_on(screen, LINKED_BRANCH)
        && !tree_cursor_on(screen, "workspace")
}

/// Type a palette filter at human key gaps for nav letters.
///
/// `keys("vs default")` can drop the `l` in `default`. `close` has the same
/// held-nav risk.
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

/// Missing default and unborn HEAD keep Diff vs default / vs branch dimmed.
///
/// On `topic`, the palette shows Default branch not found. Enter keeps the
/// overlay and does not add a compare tab. On `empty`, both Diff vs default
/// and Diff vs branch show HEAD has no commit. Enter still does not open a tab.
#[test]
fn pty_compare_missing_default_and_unborn_disable_open() {
    let (_root, workspace) = topic_and_unborn_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| tree_has(screen, "topic") && tree_has(screen, "empty") && palette_closed(screen),
        "first paint: topic and empty are on the tree; command palette is closed",
        WAIT,
    );

    tui.search("topic");
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "topic")
                && !tree_cursor_on(screen, "empty")
                && !tree_cursor_on(screen, "workspace")
                && palette_closed(screen)
        },
        "/topic keeps the tree cursor on topic (a miss jumps to empty or workspace)",
        WAIT,
    );
    open_ctrl_k_filter(&mut tui, "vs default", "Diff vs default");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("Diff vs default")
                && screen.contains("Default branch not found")
        },
        "Diff vs default on topic shows Default branch not found",
        WAIT,
    );
    enter_keeps_palette_open(
        &mut tui,
        "Enter on dimmed Diff vs default keeps the palette",
    );
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("Default branch not found")
                && workspace_only(screen)
        },
        "Enter keeps Default branch not found; no compare tab (no · vs)",
        WAIT,
    );
    esc_closes_palette(&mut tui);

    tui.search("empty");
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "empty")
                && !tree_cursor_on(screen, "topic")
                && !tree_cursor_on(screen, "workspace")
                && palette_closed(screen)
        },
        "/empty keeps the tree cursor on empty (a miss jumps to topic or workspace)",
        WAIT,
    );
    open_ctrl_k_filter(&mut tui, "vs default", "Diff vs default");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("Diff vs default")
                && screen.contains("HEAD has no commit")
        },
        "Diff vs default on empty shows HEAD has no commit",
        WAIT,
    );
    esc_closes_palette(&mut tui);

    tui.wait_pred(
        |screen| tree_cursor_on(screen, "empty") && palette_closed(screen),
        "Esc leaves the cursor on empty with the palette closed",
        WAIT,
    );
    open_ctrl_k_filter(&mut tui, "vs branch", "Diff vs branch…");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("Diff vs branch…")
                && screen.contains("HEAD has no commit")
        },
        "Diff vs branch on empty shows HEAD has no commit",
        WAIT,
    );
    enter_keeps_palette_open(&mut tui, "Enter on dimmed Diff vs branch keeps the palette");
    tui.wait_pred(
        |screen| {
            palette_open(screen) && screen.contains("HEAD has no commit") && workspace_only(screen)
        },
        "Enter keeps HEAD has no commit; no compare tab (no · vs)",
        WAIT,
    );
}

/// Workspace and family rows are not compare targets.
///
/// Diff vs default stays dimmed with Focus a checkout to compare. Close
/// compare tab cannot close the Workspace tab.
#[test]
fn pty_compare_workspace_and_family_are_not_targets() {
    let (_root, workspace) = worktree_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| family_visible(screen) && palette_closed(screen),
        "first paint: family app, primary, and linked branches; command palette is closed",
        WAIT,
    );

    tui.gg();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "workspace")
                && !tree_cursor_on(screen, "app")
                && !tree_cursor_on(screen, PRIMARY_BRANCH)
                && !tree_cursor_on(screen, LINKED_BRANCH)
                && palette_closed(screen)
        },
        "gg moves the tree cursor to the workspace row (not app or a checkout leaf)",
        WAIT,
    );
    open_ctrl_k_filter(&mut tui, "vs default", "Diff vs default");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("Diff vs default")
                && screen.contains("Focus a checkout to compare")
        },
        "Diff vs default on the workspace row shows Focus a checkout to compare",
        WAIT,
    );
    esc_closes_palette(&mut tui);

    tui.search("app");
    tui.wait_pred(
        |screen| family_row_focused(screen) && palette_closed(screen),
        "/app lands on the family row, not a checkout leaf or workspace",
        WAIT,
    );
    open_ctrl_k_filter(&mut tui, "vs default", "Diff vs default");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("Diff vs default")
                && screen.contains("Focus a checkout to compare")
        },
        "Diff vs default on the family row shows Focus a checkout to compare",
        WAIT,
    );
    esc_closes_palette(&mut tui);

    open_ctrl_k_filter(&mut tui, "close compare", "Close compare tab");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("Close compare tab")
                && screen.contains("Workspace tab cannot be closed")
        },
        "Close compare tab on Workspace shows Workspace tab cannot be closed",
        WAIT,
    );
    enter_keeps_palette_open(
        &mut tui,
        "Enter on dimmed Close compare tab keeps the palette",
    );
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("Workspace tab cannot be closed")
                && workspace_only(screen)
        },
        "Enter keeps a single Workspace tab (no close, no · vs)",
        WAIT,
    );
}
